//! The `claude-dash` TUI — a read-only dashboard.
//!
//! This slice renders the **Budget left rail** and N concurrent **Active
//! Session** panels:
//! - The left rail shows the 5-hour and 7-day **Rolling Window**s with
//!   **Utilization** as a percentage and a live countdown to each reset.
//!   **Budget** is the newest `req` across all session files (account-wide, so
//!   the freshest reading wins).
//! - Each **Active Session** panel — one per **Session** present in the store —
//!   is labelled `project · model · id` and shows that session's per-**Session**
//!   **Throughput** as a rolling 60s tokens/min rate plus a braille sparkline,
//!   windowed (not instantaneous) so bursty per-request data reads smoothly.
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

use crate::record::ReqRecord;
use crate::store::{self, SessionView};
use crate::throughput::{self, RollingRate};

/// How often the dashboard ticks so countdowns advance and freshly-appended
/// records are reflected within ~1s.
const TICK: Duration = Duration::from_millis(1000);

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
    loop {
        let now = chrono::Utc::now().timestamp();
        let now_ms = chrono::Utc::now().timestamp_millis();
        terminal.draw(|f| draw(f, budget.as_ref(), &sessions, now, now_ms))?;

        // Wait up to one tick for a keypress; the tick itself advances the
        // countdown.
        if event::poll(TICK)? {
            if let Event::Key(key) = event::read()? {
                let ctrl_c =
                    key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL);
                if key.code == KeyCode::Char('q') || ctrl_c {
                    return Ok(());
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

/// Draw the **Budget left rail** and the N concurrent **Active Session** panels.
fn draw(
    frame: &mut Frame,
    budget: Option<&ReqRecord>,
    sessions: &[SessionView],
    now_epoch: i64,
    now_ms: i64,
) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(40), Constraint::Min(0)])
        .split(frame.area());

    draw_budget_rail(frame, chunks[0], budget, now_epoch);
    draw_sessions(frame, chunks[1], sessions, now_ms);
}

/// Lay the **Session**s out as a vertical stack of equal panels — one **Active
/// Session** panel per **Session** present in the store.
fn draw_sessions(frame: &mut Frame, area: Rect, sessions: &[SessionView], now_ms: i64) {
    if sessions.is_empty() {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Active Sessions ");
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let msg = Paragraph::new("No Sessions yet.\nLaunch `cca` to start one…")
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

/// One **Session** panel: titled `project · model · id`, showing that session's
/// per-**Session** **Throughput** as a rolling 60s tokens/min rate plus a braille
/// sparkline. (Every **Session** in the store renders as an **Active Session**
/// until slice 04 adds lifecycle to split active from **Session History**.)
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
            format!("{} ", rate.tokens_per_min),
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

/// The left rail: 5h and 7d **Rolling Window** gauges with % **Utilization** and
/// a countdown to each reset.
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

    // Layout: representative line, 5h label, 5h gauge, 7d label, 7d gauge.
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // representative / status
            Constraint::Length(1), // 5h label + countdown
            Constraint::Length(1), // 5h gauge
            Constraint::Length(1), // spacer
            Constraint::Length(1), // 7d label + countdown
            Constraint::Length(1), // 7d gauge
            Constraint::Min(0),
        ])
        .split(inner);

    let b = &req.budget;
    let rep = if b.rep.is_empty() { "—" } else { b.rep.as_str() };
    let header = Paragraph::new(Line::from(vec![
        Span::styled("representative: ", Style::default().fg(Color::DarkGray)),
        Span::styled(rep, Style::default().add_modifier(Modifier::BOLD)),
    ]));
    frame.render_widget(header, rows[0]);

    render_window(frame, rows[1], rows[2], "5h", b.b5_util, b.b5_reset, now_epoch);
    render_window(frame, rows[4], rows[5], "7d", b.b7_util, b.b7_reset, now_epoch);
}

/// Render one **Rolling Window**: a label line (`<name>  <pct>%  resets in …`)
/// and a gauge filled to its **Utilization**.
fn render_window(
    frame: &mut Frame,
    label_area: Rect,
    gauge_area: Rect,
    name: &str,
    util: f64,
    reset_epoch: i64,
    now_epoch: i64,
) {
    let pct = (util.clamp(0.0, 1.0) * 100.0).round() as u16;
    let countdown = format_countdown(reset_epoch - now_epoch);

    let label = Paragraph::new(Line::from(vec![
        Span::styled(format!("{name} "), Style::default().add_modifier(Modifier::BOLD)),
        Span::styled(format!("{pct}% "), Style::default().fg(util_color(util))),
        Span::styled(
            format!("resets in {countdown}"),
            Style::default().fg(Color::DarkGray),
        ),
    ]));
    frame.render_widget(label, label_area);

    let gauge = Gauge::default()
        .gauge_style(Style::default().fg(util_color(util)))
        .ratio(util.clamp(0.0, 1.0))
        .label(format!("{pct}%"));
    frame.render_widget(gauge, gauge_area);
}

/// Colour a gauge by how much of its window is consumed.
fn util_color(util: f64) -> Color {
    if util >= 0.9 {
        Color::Red
    } else if util >= 0.6 {
        Color::Yellow
    } else {
        Color::Green
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
    fn util_colors_by_severity() {
        assert_eq!(util_color(0.1), Color::Green);
        assert_eq!(util_color(0.7), Color::Yellow);
        assert_eq!(util_color(0.95), Color::Red);
    }
}
