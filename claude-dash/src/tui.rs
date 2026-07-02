//! The `claude-dash` TUI — a read-only dashboard.
//!
//! Renders the **Budget left rail**, the **Active Session** panels, and the
//! **Session History** view:
//! - The left rail shows the 5-hour and 7-day **Rolling Window**s with
//!   **Utilization** as a percentage and a live countdown to each reset.
//!   **Budget** is the newest `req` across all session files (account-wide, so
//!   the freshest reading wins).
//! - Each **Active Session** panel — one per running **Session** — is labelled
//!   `project · model · id` and shows that session's per-**Session**
//!   **Throughput** as a rolling 60s tokens/min rate plus a braille sparkline,
//!   windowed (not instantaneous) so bursty per-request data reads smoothly.
//! - **Session History** lists every ended **Session** as
//!   `project · model · total tokens · duration · ended (relative)`, scrollable.
//!
//! The right pane shows one of three [`View`]s at a time — **Live** (the
//! `Human`-Origin Active Session panels, the default), **History** (the
//! scrollable ended-Session list), or **Agents** (the `Agent`-Origin Active
//! Session panels) — cycled with `Tab`/`BackTab`. The **Budget left rail** stays
//! visible in all three.
//!
//! Active vs ended is the [`lifecycle`] classifier's call, not the TUI's: the
//! render code only renders the classification (active panels / History rows),
//! it never computes liveness itself. The only liveness I/O is the classifier's
//! injected [`lifecycle::pid_alive`] adapter.
//!
//! Liveness comes from two sources: a `notify` file-watch on the store
//! directory, and a ~1s tick so countdowns advance and new records appear within
//! ~1s.

use std::path::Path;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::ExecutableCommand;
use notify::{RecursiveMode, Watcher};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, Paragraph};
use ratatui::{Frame, Terminal};

use crate::budget;
use crate::lifecycle::{self, EndedSession};
use crate::record::{Origin, ReqRecord};
use crate::store::{self, SessionView};
use crate::throughput::{self, RollingRate};

/// How often the dashboard ticks so countdowns advance and freshly-appended
/// records are reflected within ~1s.
const TICK: Duration = Duration::from_millis(1000);

/// Which right-pane [`View`] is showing. Three exist, cycled with `Tab`
/// (forward) / `BackTab` (reverse) in the captain's tab order **Live Human
/// Session | History | Live Agents**:
/// - **Live** (the default) — the **Active Session** panels for `Human`-**Origin**
///   sessions (`cca`).
/// - **History** — the scrollable ended-**Session** list (any Origin).
/// - **Agents** — the **Active Session** panels for `Agent`-**Origin** sessions
///   (`ccagent`), same panel layout as Live.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum View {
    Live,
    History,
    Agents,
}

impl View {
    /// Advance **Live → History → Agents → Live** (the `Tab` action).
    fn next(self) -> Self {
        match self {
            View::Live => View::History,
            View::History => View::Agents,
            View::Agents => View::Live,
        }
    }

    /// Reverse **Live → Agents → History → Live** (the `BackTab` action).
    fn prev(self) -> Self {
        match self {
            View::Live => View::Agents,
            View::Agents => View::History,
            View::History => View::Live,
        }
    }
}

/// Select the **Active Session**s of a given **Origin** — the filter behind the
/// **Live** (Human) and **Agents** (Agent) views over the shared active set the
/// [`lifecycle`] classifier produced. Active-vs-ended stays the classifier's
/// call; this only partitions the active set by Origin. A session with no `start`
/// can't be active, but defaults to `Human` for safety.
fn active_of_origin<'a>(active: &[&'a SessionView], origin: Origin) -> Vec<&'a SessionView> {
    active
        .iter()
        .copied()
        .filter(|v| v.start.as_ref().map(|s| s.origin).unwrap_or_default() == origin)
        .collect()
}

/// Run the dashboard until the user quits (`q` or Ctrl-C).
pub fn run() -> Result<()> {
    let dir = store::sessions_dir()?;
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating store dir {}", dir.display()))?;

    // File-watch the store directory for liveness. We don't act on the event
    // payload — any change just means "re-read on the next tick".
    let (watch_tx, watch_rx) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |res| {
        let _ = watch_tx.send(res);
    })
    .context("creating file watcher")?;
    watcher
        .watch(&dir, RecursiveMode::NonRecursive)
        .with_context(|| format!("watching store dir {}", dir.display()))?;

    let mut terminal = setup_terminal()?;
    let result = event_loop(&mut terminal, &dir, &watch_rx);
    restore_terminal(&mut terminal)?;
    result
}

/// The render/poll loop. Redraws every tick so countdowns advance, but only
/// re-reads the store when the watcher reports a change — the **Budget** reading
/// itself is unchanged between writes, and the per-tick redraw just needs the
/// current clock for the countdown.
fn event_loop<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    dir: &Path,
    watch_rx: &Receiver<notify::Result<notify::Event>>,
) -> Result<()> {
    // One read of the store yields the per-Session grouping primitive; both the
    // account-wide Budget and the N session panels are thin selections over it.
    let mut sessions = store::session_views_in_dir(dir);
    let mut budget = store::newest_req_in_views(&sessions).cloned();

    // Right-pane view state: Live is the default; History tracks a scroll offset
    // (rows hidden above the viewport), clamped each tick to the current list.
    let mut view = View::Live;
    let mut history_scroll: u16 = 0;
    loop {
        let now = chrono::Utc::now().timestamp();
        let now_ms = chrono::Utc::now().timestamp_millis();

        // Classify once per tick: the active/ended split feeds both the render and
        // the turn-done detection (pid-liveness is the classifier's only I/O).
        let (active, history) = lifecycle::split_sessions(&sessions, lifecycle::pid_alive);

        // Clamp the History scroll to what's actually scrollable now that the
        // list and terminal size are known: rows beyond the last viewport line.
        let max_scroll = max_history_scroll(history.len(), terminal.size()?.height);
        history_scroll = history_scroll.min(max_scroll);

        terminal.draw(|f| draw(f, budget.as_ref(), &active, &history, now, now_ms, view, history_scroll))?;

        // Wait up to one tick for a keypress; the tick itself advances the
        // countdown.
        if event::poll(TICK)? {
            if let Event::Key(key) = event::read()? {
                let ctrl_c =
                    key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL);
                if key.code == KeyCode::Char('q') || ctrl_c {
                    return Ok(());
                }
                match key.code {
                    // Tab cycles forward (Live → History → Agents), BackTab
                    // reverse; entering History starts at the top.
                    KeyCode::Tab => {
                        view = view.next();
                        history_scroll = 0;
                    }
                    KeyCode::BackTab => {
                        view = view.prev();
                        history_scroll = 0;
                    }
                    // History scrolling (no-op in Live view).
                    KeyCode::Down | KeyCode::Char('j') if view == View::History => {
                        history_scroll = history_scroll.saturating_add(1).min(max_scroll);
                    }
                    KeyCode::Up | KeyCode::Char('k') if view == View::History => {
                        history_scroll = history_scroll.saturating_sub(1);
                    }
                    KeyCode::PageDown if view == View::History => {
                        history_scroll = history_scroll.saturating_add(10).min(max_scroll);
                    }
                    KeyCode::PageUp if view == View::History => {
                        history_scroll = history_scroll.saturating_sub(10);
                    }
                    KeyCode::Home | KeyCode::Char('g') if view == View::History => {
                        history_scroll = 0;
                    }
                    KeyCode::End | KeyCode::Char('G') if view == View::History => {
                        history_scroll = max_scroll;
                    }
                    _ => {}
                }
            }
        }

        // Drain any pending watch events so a burst of writes coalesces into one
        // re-read; only then is the store re-globbed and re-parsed.
        let mut changed = false;
        while watch_rx.try_recv().is_ok() {
            changed = true;
        }
        if changed {
            sessions = store::session_views_in_dir(dir);
            budget = store::newest_req_in_views(&sessions).cloned();
        }
    }
}

/// Draw the **Budget left rail**, the active [`View`] (Live panels or scrollable
/// History) in the right pane, and the bottom help line.
///
/// The active/ended split is the [`lifecycle`] classifier's, computed once per
/// tick in the event loop and passed in; the TUI just renders the selected view.
/// History is rebuilt from the store every read, so it survives a `claude-dash`
/// restart for free.
#[allow(clippy::too_many_arguments)]
fn draw(
    frame: &mut Frame,
    budget: Option<&ReqRecord>,
    active: &[&SessionView],
    history: &[EndedSession],
    now_epoch: i64,
    now_ms: i64,
    view: View,
    history_scroll: u16,
) {
    // Reserve the bottom row for the help line; the rest holds rail + content.
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(frame.area());

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(40), Constraint::Min(0)])
        .split(outer[0]);

    draw_budget_rail(frame, chunks[0], budget, now_epoch);

    // The right pane shows one view at a time; Live is the default. Live and
    // Agents share the Active Session panel layout, differing only in which
    // Origin they select from the (classifier-produced) active set.
    match view {
        View::Live => {
            let humans = active_of_origin(active, Origin::Human);
            draw_sessions(
                frame,
                chunks[1],
                &humans,
                now_ms,
                " Active Sessions ",
                "No Sessions yet.\nLaunch `cca` to start one…",
            );
        }
        View::Agents => {
            let agents = active_of_origin(active, Origin::Agent);
            draw_sessions(
                frame,
                chunks[1],
                &agents,
                now_ms,
                " Live Agents ",
                "No agent Sessions yet.\nLaunch `ccagent` to start one…",
            );
        }
        View::History => draw_history(frame, chunks[1], history, now_ms, history_scroll),
    }

    draw_help(frame, outer[1], view);
}

/// The largest valid **History** scroll offset for a list of `rows` rendered into
/// a terminal `term_height` rows tall: rows that would sit above the viewport's
/// last line. Mirrors [`draw`]'s layout — one help row, then the History box's
/// two borders — so scrolling never runs past the final entry into empty space.
fn max_history_scroll(rows: usize, term_height: u16) -> u16 {
    let visible = term_height.saturating_sub(3); // help line + top/bottom border
    (rows as u16).saturating_sub(visible)
}

/// The bottom help line: names all three views (Live Human | History | Live
/// Agents) with the current one bracketed, the `Tab`/`BackTab` cycle, plus the
/// scroll hint while in **History**. Renders inverted-dim so it reads as chrome.
fn draw_help(frame: &mut Frame, area: Rect, view: View) {
    let hint = match view {
        View::Live => "[Live Human]  History  Live Agents   ·   [Tab/⇧Tab] cycle   [q] quit",
        View::History => {
            "Live Human  [History]  Live Agents   ·   [Tab/⇧Tab] cycle   [↑/↓ j/k] scroll   [g/G] top/bottom   [q] quit"
        }
        View::Agents => "Live Human  History  [Live Agents]   ·   [Tab/⇧Tab] cycle   [q] quit",
    };
    let para = Paragraph::new(Line::from(Span::styled(
        format!(" {hint} "),
        Style::default().fg(Color::DarkGray),
    )));
    frame.render_widget(para, area);
}

/// Lay the **Active Session**s out as a vertical stack of equal panels — one
/// panel per **Active Session** (a **Session** the [`lifecycle`] classifier
/// judged still running). The `empty_title`/`empty_hint` are shown when there are
/// none, so the **Live** and **Agents** views can name their own empty state
/// (`cca` vs `ccagent`) while sharing the panel layout.
fn draw_sessions(
    frame: &mut Frame,
    area: Rect,
    sessions: &[&SessionView],
    now_ms: i64,
    empty_title: &str,
    empty_hint: &str,
) {
    if sessions.is_empty() {
        let block = Block::default().borders(Borders::ALL).title(empty_title.to_string());
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let msg = Paragraph::new(empty_hint.to_string())
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(msg, inner);
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![
            Constraint::Ratio(1, sessions.len() as u32);
            sessions.len()
        ])
        .split(area);

    for (panel, view) in rows.iter().zip(sessions.iter()) {
        draw_session_panel(frame, *panel, view, now_ms);
    }
}

/// One **Active Session** panel: titled `project · model · id`, showing that
/// session's per-**Session** **Throughput** as a rolling 60s tokens/min rate plus
/// a braille sparkline.
fn draw_session_panel(frame: &mut Frame, area: Rect, view: &SessionView, now_ms: i64) {
    // The Model is the newest reading's model (Throughput breaks down per Model;
    // the active turn's model is the freshest one captured).
    let model = view
        .reqs
        .iter()
        .rev()
        .find_map(|r| r.throughput.as_ref())
        .map(|tp| tp.model.as_str())
        .filter(|m| !m.is_empty())
        .unwrap_or("—");

    // The panel label is `project · model · id` — project from the `start`
    // record, model from the freshest Throughput, id is the Session id.
    let project = view
        .start
        .as_ref()
        .map(|s| s.project.as_str())
        .filter(|p| !p.is_empty())
        .unwrap_or("—");
    let title = format!(" {project} · {model} · {} ", view.id);

    let block = Block::default().borders(Borders::ALL).title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // The Throughput samples: each `req` carrying a Throughput reading is one
    // (ts, total-tokens) point for the rolling window.
    let samples: Vec<(i64, u64)> = view
        .reqs
        .iter()
        .filter_map(|r| r.throughput.as_ref().map(|tp| (r.ts, tp.total())))
        .collect();

    if samples.is_empty() {
        let msg = Paragraph::new("No Throughput yet.\nWaiting for a request…")
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(msg, inner);
        return;
    }

    let rate = throughput::rolling_rate(samples, now_ms);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // rate
            Constraint::Length(1), // sparkline
            Constraint::Min(0),
        ])
        .split(inner);

    let rate_line = Paragraph::new(Line::from(vec![
        Span::styled(
            format!("{} ", humanize_tokens(rate.tokens_per_min)),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("tok/min (60s)", Style::default().fg(Color::DarkGray)),
    ]));
    frame.render_widget(rate_line, rows[0]);

    let spark = Paragraph::new(Line::from(Span::styled(
        braille_sparkline(&rate),
        Style::default().fg(Color::Cyan),
    )));
    frame.render_widget(spark, rows[1]);
}

/// The **Session History** view: every ended **Session**, each rendered as one
/// row `project · model · total tokens · duration · ended (relative)`, scrolled
/// by `scroll` rows. The rows are already classified, summarised, and ordered
/// (most recent first) by the [`lifecycle`] classifier — this just formats them
/// and clips to the viewport via [`Paragraph::scroll`].
fn draw_history(frame: &mut Frame, area: Rect, history: &[EndedSession], now_ms: i64, scroll: u16) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" Session History ({}) ", history.len()));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if history.is_empty() {
        let msg = Paragraph::new("No ended Sessions yet.")
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(msg, inner);
        return;
    }

    let lines: Vec<Line> = history.iter().map(|e| history_row(e, now_ms)).collect();
    let para = Paragraph::new(lines).scroll((scroll, 0));
    frame.render_widget(para, inner);
}

/// Format one **Session History** row from an [`EndedSession`]:
/// `project · model · <total> tok · <duration> · ended <relative>`. The relative
/// "ended Xm ago" is measured against `now_ms` (epoch milliseconds).
fn history_row(ended: &EndedSession, now_ms: i64) -> Line<'static> {
    // `summarize` already substitutes "—" for an absent project/model, so the
    // fields can be rendered directly — the placeholder lives in one place.
    let sep = Span::styled(" · ", Style::default().fg(Color::DarkGray));
    Line::from(vec![
        Span::styled(ended.project.clone(), Style::default().add_modifier(Modifier::BOLD)),
        sep.clone(),
        Span::styled(ended.model.clone(), Style::default().fg(Color::Cyan)),
        sep.clone(),
        Span::styled(
            format!("{} tok", humanize_tokens(ended.total_tokens)),
            Style::default().fg(Color::Gray),
        ),
        sep.clone(),
        Span::styled(
            lifecycle::format_duration(ended.duration_ms),
            Style::default().fg(Color::Gray),
        ),
        sep,
        Span::styled(
            format!("ended {}", lifecycle::format_ended_ago(ended.ended_ms, now_ms)),
            Style::default().fg(Color::DarkGray),
        ),
    ])
}

/// Render a [`RollingRate`]'s per-bucket token sums as a braille sparkline — one
/// braille block per bucket, scaled to the busiest bucket so the bars show the
/// *shape* of recent **Throughput** rather than absolute height.
fn braille_sparkline(rate: &RollingRate) -> String {
    // Braille bar glyphs from empty to full (8 levels).
    const BARS: [char; 8] = ['⡀', '⡄', '⡆', '⡇', '⣇', '⣧', '⣷', '⣿'];
    let max = rate.buckets.iter().copied().max().unwrap_or(0);
    if max == 0 {
        return BARS[0].to_string().repeat(rate.buckets.len());
    }
    rate.buckets
        .iter()
        .map(|&b| {
            let level = ((b as f64 / max as f64) * (BARS.len() - 1) as f64).round() as usize;
            BARS[level.min(BARS.len() - 1)]
        })
        .collect()
}

/// The left rail: the two **Rolling Window**s (5h, 7d) with % **Utilization**, a
/// gauge, status, and a countdown to each reset; the **Representative Window**
/// emphasised; the **overage** state surfaced when the headers report it.
///
/// Every presentation decision (which window is representative, each window's
/// [`Severity`], the overage banner) is computed by the pure, ratatui-free seam
/// on [`Budget`] — this function only *renders* those decisions, mapping
/// [`Severity`] to a `Color` at the render edge.
fn draw_budget_rail(frame: &mut Frame, area: Rect, budget: Option<&ReqRecord>, now_epoch: i64) {
    let block = Block::default().borders(Borders::ALL).title(" Budget ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(req) = budget else {
        let msg = Paragraph::new("No Budget reading yet.\nWaiting for a request…")
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(msg, inner);
        return;
    };

    let b = &req.budget;
    // The pure seam's decisions — the TUI computes nothing here.
    let rep_window = b.representative();
    let overage = b.overage();

    // Layout: status line, 5h label, 5h gauge, spacer, 7d label, 7d gauge, then
    // an overage banner row that stays empty when there's no overage to show.
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // status
            Constraint::Length(1), // 5h label + countdown
            Constraint::Length(1), // 5h gauge
            Constraint::Length(1), // spacer
            Constraint::Length(1), // 7d label + countdown
            Constraint::Length(1), // 7d gauge
            Constraint::Length(1), // spacer
            Constraint::Length(1), // overage banner
            Constraint::Min(0),
        ])
        .split(inner);

    let status = if b.status.is_empty() { "—" } else { b.status.as_str() };
    let status_line = Paragraph::new(Line::from(vec![
        Span::styled("status: ", Style::default().fg(Color::DarkGray)),
        Span::styled(status, Style::default().add_modifier(Modifier::BOLD)),
    ]));
    frame.render_widget(status_line, rows[0]);

    render_window(
        frame,
        rows[1],
        rows[2],
        "5h",
        b.b5_util,
        b.severity(b.b5_util),
        b.b5_reset,
        now_epoch,
        rep_window == budget::Window::FiveHour,
    );
    render_window(
        frame,
        rows[4],
        rows[5],
        "7d",
        b.b7_util,
        b.severity(b.b7_util),
        b.b7_reset,
        now_epoch,
        rep_window == budget::Window::SevenDay,
    );

    // The overage banner: shown only when the seam reports one, coloured by its
    // Severity. The row simply stays blank otherwise.
    if let Some(overage) = overage {
        let banner = Paragraph::new(Line::from(Span::styled(
            overage.label,
            Style::default()
                .fg(severity_color(overage.severity))
                .add_modifier(Modifier::BOLD),
        )));
        frame.render_widget(banner, rows[7]);
    }
}

/// Render one **Rolling Window**: a label line (`<marker> <name>  <pct>%  resets
/// in …`) and a gauge filled to its **Utilization**, coloured by [`Severity`].
///
/// The **Representative Window** is emphasised with a leading `▶` marker and a
/// bold, brighter name; the other window gets a blank lead and a dim name — so
/// the binding window reads at a glance.
#[allow(clippy::too_many_arguments)]
fn render_window(
    frame: &mut Frame,
    label_area: Rect,
    gauge_area: Rect,
    name: &str,
    util: f64,
    severity: budget::Severity,
    reset_epoch: i64,
    now_epoch: i64,
    representative: bool,
) {
    let pct = budget::percent(util);
    let countdown = format_countdown(reset_epoch - now_epoch);
    let color = severity_color(severity);

    // Emphasis: the Representative (binding) window gets a ▶ marker + bold/bright
    // name; the other window a blank lead + dim name.
    let (marker, name_style) = if representative {
        ("▶ ", Style::default().fg(Color::White).add_modifier(Modifier::BOLD))
    } else {
        ("  ", Style::default().fg(Color::DarkGray))
    };

    let label = Paragraph::new(Line::from(vec![
        Span::styled(marker, Style::default().fg(color)),
        Span::styled(format!("{name} "), name_style),
        Span::styled(format!("{pct}% "), Style::default().fg(color)),
        Span::styled(
            format!("resets in {countdown}"),
            Style::default().fg(Color::DarkGray),
        ),
    ]));
    frame.render_widget(label, label_area);

    let gauge = Gauge::default()
        .gauge_style(Style::default().fg(color))
        .ratio(util.clamp(0.0, 1.0))
        .label(format!("{pct}%"));
    frame.render_widget(gauge, gauge_area);
}

/// Map a pure [`Severity`] to a gauge/label `Color` — the single render-edge
/// translation, keeping ratatui out of the [`Budget`] presentation seam.
fn severity_color(severity: budget::Severity) -> Color {
    match severity {
        budget::Severity::Ok => Color::Green,
        budget::Severity::Warning => Color::Yellow,
        budget::Severity::Critical => Color::Red,
    }
}

/// Humanize a token count into a compact form: `< 1000` stays exact, then `1.2k`,
/// `100k`, `1.2M`. The fractional digit is dropped once the leading number reaches
/// three figures (`100k`, not `100.0k`) so the width stays stable.
fn humanize_tokens(n: u64) -> String {
    fn scale(n: u64, divisor: f64, suffix: char) -> String {
        let v = n as f64 / divisor;
        if v >= 100.0 {
            format!("{}{suffix}", v.round() as u64)
        } else {
            format!("{v:.1}{suffix}")
        }
    }
    if n < 1_000 {
        n.to_string()
    } else if n < 1_000_000 {
        scale(n, 1_000.0, 'k')
    } else {
        scale(n, 1_000_000.0, 'M')
    }
}

/// Format a countdown of `secs` seconds as `HHh MMm SSs` (or `Dd HHh` for long
/// 7-day windows). Returns `now` once the reset has passed.
fn format_countdown(secs: i64) -> String {
    if secs <= 0 {
        return "now".to_string();
    }
    let days = secs / 86_400;
    let hours = (secs % 86_400) / 3_600;
    let minutes = (secs % 3_600) / 60;
    let seconds = secs % 60;

    if days > 0 {
        format!("{days}d {hours:02}h {minutes:02}m")
    } else {
        format!("{hours:02}h {minutes:02}m {seconds:02}s")
    }
}

fn setup_terminal() -> Result<Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>> {
    enable_raw_mode().context("enabling raw mode")?;
    let mut stdout = std::io::stdout();
    stdout
        .execute(EnterAlternateScreen)
        .context("entering alternate screen")?;
    let backend = ratatui::backend::CrosstermBackend::new(stdout);
    Terminal::new(backend).context("creating terminal")
}

fn restore_terminal<B: ratatui::backend::Backend + std::io::Write>(
    terminal: &mut Terminal<B>,
) -> Result<()> {
    disable_raw_mode().ok();
    terminal.backend_mut().execute(LeaveAlternateScreen).ok();
    terminal.show_cursor().ok();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn countdown_formats_short_window() {
        // 1h 02m 03s
        assert_eq!(format_countdown(3_600 + 120 + 3), "01h 02m 03s");
    }

    #[test]
    fn countdown_formats_multi_day_window() {
        // 2 days, 3 hours, 4 minutes
        assert_eq!(format_countdown(2 * 86_400 + 3 * 3_600 + 4 * 60), "2d 03h 04m");
    }

    #[test]
    fn countdown_is_now_when_elapsed() {
        assert_eq!(format_countdown(0), "now");
        assert_eq!(format_countdown(-5), "now");
    }

    #[test]
    fn braille_sparkline_has_one_glyph_per_bucket() {
        let rate = RollingRate {
            tokens_per_min: 100,
            buckets: vec![0, 5, 10, 0],
        };
        let spark = braille_sparkline(&rate);
        assert_eq!(spark.chars().count(), 4);
    }

    #[test]
    fn braille_sparkline_scales_busiest_bucket_to_full() {
        let rate = RollingRate {
            tokens_per_min: 100,
            buckets: vec![0, 100],
        };
        let spark: Vec<char> = braille_sparkline(&rate).chars().collect();
        // The busiest bucket renders as the fullest glyph.
        assert_eq!(spark[1], '⣿');
    }

    #[test]
    fn braille_sparkline_all_zero_is_flat() {
        let rate = RollingRate {
            tokens_per_min: 0,
            buckets: vec![0, 0, 0],
        };
        let spark = braille_sparkline(&rate);
        assert_eq!(spark.chars().count(), 3);
    }

    #[test]
    fn humanize_tokens_scales_by_magnitude() {
        assert_eq!(humanize_tokens(0), "0");
        assert_eq!(humanize_tokens(999), "999");
        assert_eq!(humanize_tokens(1_200), "1.2k");
        assert_eq!(humanize_tokens(100_000), "100k");
        assert_eq!(humanize_tokens(1_200_000), "1.2M");
        assert_eq!(humanize_tokens(1_000_000), "1.0M");
    }

    #[test]
    fn view_cycles_forward_and_reverse() {
        // Forward (Tab): Live → History → Agents → Live.
        assert_eq!(View::Live.next(), View::History);
        assert_eq!(View::History.next(), View::Agents);
        assert_eq!(View::Agents.next(), View::Live);
        // Reverse (BackTab): Live → Agents → History → Live.
        assert_eq!(View::Live.prev(), View::Agents);
        assert_eq!(View::Agents.prev(), View::History);
        assert_eq!(View::History.prev(), View::Live);
    }

    /// A `SessionView` with the given id and **Origin**, active-shaped (has a
    /// `start`, no `end`) so it stands in for a classified Active Session.
    fn active_view(id: &str, origin: Origin) -> SessionView {
        SessionView {
            id: id.to_string(),
            start: Some(crate::record::StartRecord {
                id: id.to_string(),
                ts: 0,
                project: "p".to_string(),
                cwd: "/p".to_string(),
                pid: 1,
                origin,
            }),
            reqs: vec![],
            end: None,
        }
    }

    #[test]
    fn live_selects_humans_and_agents_selects_agents() {
        let h1 = active_view("h1", Origin::Human);
        let a1 = active_view("a1", Origin::Agent);
        let h2 = active_view("h2", Origin::Human);
        let active: Vec<&SessionView> = vec![&h1, &a1, &h2];

        // Live view (Human) selects only the human sessions.
        let humans = active_of_origin(&active, Origin::Human);
        let human_ids: Vec<&str> = humans.iter().map(|v| v.id.as_str()).collect();
        assert_eq!(human_ids, vec!["h1", "h2"]);

        // Agents view (Agent) selects only the agent sessions.
        let agents = active_of_origin(&active, Origin::Agent);
        let agent_ids: Vec<&str> = agents.iter().map(|v| v.id.as_str()).collect();
        assert_eq!(agent_ids, vec!["a1"]);
    }

    #[test]
    fn max_history_scroll_caps_at_last_viewport_line() {
        // 24-row terminal ⇒ 21 visible history rows (24 − 1 help − 2 borders).
        // Fewer rows than fit: nothing to scroll.
        assert_eq!(max_history_scroll(5, 24), 0);
        assert_eq!(max_history_scroll(21, 24), 0);
        // More rows than fit: the overflow is the max offset.
        assert_eq!(max_history_scroll(30, 24), 9);
    }

    #[test]
    fn max_history_scroll_handles_tiny_terminals() {
        // A terminal too short for any content row can't scroll past 0 underflow.
        assert_eq!(max_history_scroll(10, 2), 10);
        assert_eq!(max_history_scroll(0, 24), 0);
    }

    #[test]
    fn severity_maps_to_color_at_the_render_edge() {
        assert_eq!(severity_color(budget::Severity::Ok), Color::Green);
        assert_eq!(severity_color(budget::Severity::Warning), Color::Yellow);
        assert_eq!(severity_color(budget::Severity::Critical), Color::Red);
    }
}
