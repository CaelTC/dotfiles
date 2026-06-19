//! **Session** lifecycle: classify each [`SessionView`] as an **Active Session**
//! or part of **Session History**, and summarise an ended **Session** for the
//! History view.
//!
//! This is a **deep, pure seam** in the spirit of `Budget::from_headers(lookup)`:
//! the classification rule lives in one pure function ([`classify`]) over a
//! [`SessionView`] plus an *injected* pid-liveness check. The only I/O
//! — asking the OS whether a pid is alive — is a one-line adapter ([`pid_alive`])
//! that satisfies the injected seam, so the classifier itself is unit-testable
//! with a stub liveness `Fn` and no real processes.
//!
//! The rule:
//! - **Active** — has a `start`, no `end`, and its `pid` is alive.
//! - **History** — has an `end`, **or** its pid is dead (covers `cca` being
//!   killed without writing `end`).
//!
//! Because the classification is rebuilt from the on-disk [`SessionView`] on
//! every read, **Session History** is durable across a `claude-dash` restart for
//! free — there is no in-memory-only lifecycle state.

use crate::store::SessionView;
use crate::throughput;

/// The lifecycle classification of one **Session**: either an **Active Session**
/// (its **Proxy** is attached) or an ended **Session** summarised for **Session
/// History**.
///
/// The TUI renders this; it never recomputes it. Active sessions keep their
/// [`SessionView`] (they still need the live **Throughput** samples); ended ones
/// carry a pre-computed [`EndedSession`] summary so the History view is a thin
/// render over already-derived fields.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionState {
    /// An **Active Session** — has a `start`, no `end`, and its pid is alive.
    Active,
    /// An ended **Session** — bound for **Session History** — with its summary.
    Ended(EndedSession),
}

/// The **Session History** summary of one ended **Session**: the fields the
/// History view renders as `project · model · total tokens · duration · ended
/// (relative)`. Derived purely from the [`SessionView`] by [`classify`].
#[derive(Debug, Clone, PartialEq)]
pub struct EndedSession {
    /// The **Session**'s project (cwd basename), or `"—"` when it has no `start`.
    pub project: String,
    /// The **Model** that served the **Session** — the freshest captured — or
    /// `"—"` when no **Throughput** was ever captured.
    pub model: String,
    /// Total tokens over the **Session**: the sum of each `req`'s **Throughput**
    /// [`throughput::Throughput::total`], consistent with the live rate's notion
    /// of "total".
    pub total_tokens: u64,
    /// The **Session**'s duration in milliseconds (end − start), or `None` when
    /// it has no `start` to measure from.
    pub duration_ms: Option<i64>,
    /// When the **Session** ended, epoch milliseconds. Durable across restart —
    /// it is the `end` record's `ts` when present, else (for a pid-dead-without-
    /// `end` session) the last `req` ts, else the `start` ts. This is also the
    /// sort key for "the last 10 ended **Session**s".
    pub ended_ms: i64,
}

/// Classify one **Session** into [`SessionState`], given an injected pid-liveness
/// check.
///
/// Pure over its inputs — the only I/O (pid-liveness) is the injected `is_alive`
/// closure — so it is the lifecycle test surface, exercised with a stub `Fn`. The
/// rule:
/// - has `end` ⇒ **History** (regardless of liveness — an explicit exit wins).
/// - no `end`, has `start`, pid alive ⇒ **Active**.
/// - no `end`, pid dead (or no `start` to vouch for liveness) ⇒ **History**.
///
/// No-start handling: a **Session** file carrying only `req`s (no `start`, so no
/// pid to probe) is treated as **History** — we have no liveness handle to call
/// it active, and an end-less, start-less file is a remnant. Its `ended_ms` is
/// the last `req` ts (see [`EndedSession::ended_ms`]).
pub fn classify<F>(view: &SessionView, is_alive: F) -> SessionState
where
    F: Fn(i32) -> bool,
{
    let alive = match (&view.end, &view.start) {
        // An explicit `end` is the strongest signal — ended regardless of pid.
        (Some(_), _) => false,
        // No `end` but a `start`: liveness is the pid's liveness.
        (None, Some(start)) => is_alive(start.pid),
        // No `end` and no `start`: no pid to probe ⇒ not active.
        (None, None) => false,
    };

    if alive {
        SessionState::Active
    } else {
        SessionState::Ended(summarize(view))
    }
}

/// Summarise a **Session** for **Session History** — the pure derivation of the
/// History row's fields from a [`SessionView`]. The relative "ended Xm ago" label
/// is formatted at render time from [`EndedSession::ended_ms`] via
/// [`format_ended_ago`], so no `now` is needed here.
fn summarize(view: &SessionView) -> EndedSession {
    let project = view
        .start
        .as_ref()
        .map(|s| s.project.clone())
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| "—".to_string());

    // The Model is the freshest captured Throughput's model (Throughput breaks
    // down per Model; the last turn's model is the freshest one captured).
    let model = view
        .reqs
        .iter()
        .rev()
        .find_map(|r| r.throughput.as_ref())
        .map(|tp| tp.model.clone())
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| "—".to_string());

    // Total tokens = sum of each req's Throughput total, reusing the same
    // total() the live rate meters so "total" stays consistent.
    let total_tokens: u64 = view
        .reqs
        .iter()
        .filter_map(|r| r.throughput.as_ref())
        .map(throughput::Throughput::total)
        .sum();

    // Ended time, durable across restart: the `end` ts when present, else the
    // last req ts (pid-dead-without-end), else the start ts.
    let ended_ms = view
        .end
        .as_ref()
        .map(|e| e.ts)
        .or_else(|| view.reqs.iter().map(|r| r.ts).max())
        .or_else(|| view.start.as_ref().map(|s| s.ts))
        .unwrap_or(0);

    // Duration = end − start, when there's a start to measure from.
    let duration_ms = view.start.as_ref().map(|s| ended_ms - s.ts);

    EndedSession {
        project,
        model,
        total_tokens,
        duration_ms,
        ended_ms,
    }
}

/// The **Session History** selector: classify every [`SessionView`], keep the
/// **Active** ones (with their view, for live **Throughput**) and the **Ended**
/// ones (summarised), and return the last 10 ended by `ended_ms` (most recent
/// first). A thin pure selector over [`classify`] — the same shape as the
/// account-wide **Budget** and **Active Session** selectors.
pub fn split_sessions<F>(
    views: &[SessionView],
    is_alive: F,
) -> (Vec<&SessionView>, Vec<EndedSession>)
where
    F: Fn(i32) -> bool,
{
    let mut active = Vec::new();
    let mut history = Vec::new();
    for view in views {
        match classify(view, &is_alive) {
            SessionState::Active => active.push(view),
            SessionState::Ended(ended) => history.push(ended),
        }
    }
    // The last 10 ended: most recently ended first.
    history.sort_by(|a, b| b.ended_ms.cmp(&a.ended_ms));
    history.truncate(10);
    (active, history)
}

/// Humanise "ended Xm ago" from an end timestamp `ended_ms` against `now_ms`
/// (both epoch milliseconds). A small pure formatter, in the spirit of
/// `format_countdown`: `just now` under a minute, then `Xm`, `Xh`, `Xd` ago.
pub fn format_ended_ago(ended_ms: i64, now_ms: i64) -> String {
    let secs = (now_ms - ended_ms) / 1_000;
    if secs < 60 {
        return "just now".to_string();
    }
    let minutes = secs / 60;
    if minutes < 60 {
        return format!("{minutes}m ago");
    }
    let hours = minutes / 60;
    if hours < 24 {
        return format!("{hours}h ago");
    }
    let days = hours / 24;
    format!("{days}d ago")
}

/// Humanise a **Session**'s duration (milliseconds) as `Xs` / `Xm Ys` / `Xh Ym`,
/// or `—` when there was no `start` to measure from. A small pure formatter for
/// the History row.
pub fn format_duration(duration_ms: Option<i64>) -> String {
    let Some(ms) = duration_ms else {
        return "—".to_string();
    };
    let secs = (ms.max(0)) / 1_000;
    let hours = secs / 3_600;
    let minutes = (secs % 3_600) / 60;
    let seconds = secs % 60;
    if hours > 0 {
        format!("{hours}h {minutes:02}m")
    } else if minutes > 0 {
        format!("{minutes}m {seconds:02}s")
    } else {
        format!("{seconds}s")
    }
}

/// Ask the OS whether a process is alive: `kill(pid, 0)` returns success while
/// the process exists (or as a permission error `EPERM` — still alive), and
/// fails with `ESRCH` once it is gone.
///
/// This is the one-line I/O adapter that satisfies [`classify`]'s injected
/// liveness seam. We use `libc::kill` directly — a single cheap syscall, no
/// allocation, no process table walk — rather than pulling in `sysinfo` (heavy,
/// scans every process) for what is one `kill(pid, 0)`.
pub fn pid_alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    // SAFETY: `kill` with signal 0 performs only the existence/permission check
    // and delivers no signal. It has no memory effects.
    let rc = unsafe { libc::kill(pid, 0) };
    if rc == 0 {
        return true;
    }
    // EPERM means the process exists but we may not signal it — still alive.
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::Budget;
    use crate::record::{EndRecord, ReqRecord, StartRecord};
    use crate::throughput::Throughput;

    fn start(pid: i32, project: &str, ts: i64) -> StartRecord {
        StartRecord {
            id: "sess".to_string(),
            ts,
            project: project.to_string(),
            cwd: format!("/work/{project}"),
            pid,
        }
    }

    fn req(ts: i64, tp: Option<Throughput>) -> ReqRecord {
        ReqRecord::from_budget(
            &Budget {
                b5_util: 0.1,
                b5_reset: 1,
                b7_util: 0.1,
                b7_reset: 1,
                rep: "five_hour".to_string(),
                status: "allowed".to_string(),
                ..Default::default()
            },
            ts,
            tp,
        )
    }

    fn throughput(total_each: u64, model: &str) -> Throughput {
        // input carries the whole sum so total() == total_each.
        Throughput {
            input: total_each,
            output: 0,
            cache_read: 0,
            cache_write: 0,
            model: model.to_string(),
        }
    }

    fn view(start: Option<StartRecord>, reqs: Vec<ReqRecord>, end: Option<EndRecord>) -> SessionView {
        SessionView {
            id: "sess".to_string(),
            start,
            reqs,
            end,
        }
    }

    const ALIVE: fn(i32) -> bool = |_| true;
    const DEAD: fn(i32) -> bool = |_| false;

    #[test]
    fn start_no_end_pid_alive_is_active() {
        let v = view(Some(start(42, "proj", 1_000)), vec![req(1_500, None)], None);
        assert_eq!(classify(&v, ALIVE), SessionState::Active);
    }

    #[test]
    fn start_with_end_is_history_even_when_pid_alive() {
        let v = view(
            Some(start(42, "proj", 1_000)),
            vec![req(1_500, None)],
            Some(EndRecord {
                id: "sess".to_string(),
                ts: 1_900,
            }),
        );
        // Liveness ALIVE, but the explicit end wins → History.
        match classify(&v, ALIVE) {
            SessionState::Ended(e) => assert_eq!(e.ended_ms, 1_900),
            other => panic!("expected Ended, got {other:?}"),
        }
    }

    #[test]
    fn start_no_end_pid_dead_is_history() {
        let v = view(Some(start(42, "proj", 1_000)), vec![req(1_500, None)], None);
        match classify(&v, DEAD) {
            // ended_ms falls back to the last req ts when there's no end record.
            SessionState::Ended(e) => assert_eq!(e.ended_ms, 1_500),
            other => panic!("expected Ended, got {other:?}"),
        }
    }

    #[test]
    fn no_start_only_reqs_is_history() {
        let v = view(None, vec![req(1_500, None)], None);
        match classify(&v, ALIVE) {
            SessionState::Ended(e) => {
                assert_eq!(e.project, "—");
                assert_eq!(e.ended_ms, 1_500);
                assert_eq!(e.duration_ms, None);
            }
            other => panic!("expected Ended, got {other:?}"),
        }
    }

    #[test]
    fn ended_summary_fields_are_computed_from_the_view() {
        let v = view(
            Some(start(42, "nootka-kiosk", 1_000)),
            vec![
                req(1_200, Some(throughput(100, "claude-opus-4-8"))),
                req(1_800, Some(throughput(250, "claude-opus-4-8"))),
            ],
            Some(EndRecord {
                id: "sess".to_string(),
                ts: 4_000,
            }),
        );
        let SessionState::Ended(e) = classify(&v, DEAD) else {
            panic!("expected Ended");
        };
        assert_eq!(e.project, "nootka-kiosk");
        assert_eq!(e.model, "claude-opus-4-8");
        assert_eq!(e.total_tokens, 350); // 100 + 250
        assert_eq!(e.ended_ms, 4_000); // the end ts
        assert_eq!(e.duration_ms, Some(3_000)); // 4_000 − 1_000
    }

    #[test]
    fn restart_durability_classification_depends_only_on_view_and_liveness() {
        // Same on-disk view + same injected liveness ⇒ identical classification,
        // with no hidden in-memory state. This is the "survives a restart"
        // guarantee: History is rebuilt from the store every read.
        let v = view(
            Some(start(42, "proj", 1_000)),
            vec![req(1_500, Some(throughput(10, "m")))],
            Some(EndRecord {
                id: "sess".to_string(),
                ts: 1_900,
            }),
        );
        let first = classify(&v, DEAD);
        let second = classify(&v, DEAD);
        assert_eq!(first, second);
        assert!(matches!(first, SessionState::Ended(_)));
    }

    #[test]
    fn split_keeps_active_and_last_ten_ended_most_recent_first() {
        // One active (alive, no end) + twelve ended; expect the active kept and
        // only the 10 most-recently-ended in descending ended_ms order.
        let mut views = vec![view(Some(start(1, "live", 0)), vec![req(50, None)], None)];
        for i in 0..12i64 {
            views.push(view(
                Some(start(1000 + i as i32, "p", 0)),
                vec![],
                Some(EndRecord {
                    id: format!("e{i}"),
                    ts: i * 100,
                }),
            ));
        }
        // Liveness: only the first view's pid (1) is alive.
        let (active, history) = split_sessions(&views, |pid| pid == 1);
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].start.as_ref().unwrap().project, "live");
        assert_eq!(history.len(), 10);
        // Most recent first: ended_ms 1100, 1000, …, 200.
        assert_eq!(history[0].ended_ms, 1_100);
        assert_eq!(history[9].ended_ms, 200);
    }

    #[test]
    fn format_ended_ago_cases() {
        let now = 1_000_000_000_000i64;
        assert_eq!(format_ended_ago(now - 30_000, now), "just now"); // 30s
        assert_eq!(format_ended_ago(now - 5 * 60_000, now), "5m ago");
        assert_eq!(format_ended_ago(now - 3 * 3_600_000, now), "3h ago");
        assert_eq!(format_ended_ago(now - 2 * 86_400_000, now), "2d ago");
    }

    #[test]
    fn format_duration_cases() {
        assert_eq!(format_duration(None), "—");
        assert_eq!(format_duration(Some(45_000)), "45s");
        assert_eq!(format_duration(Some(2 * 60_000 + 5_000)), "2m 05s");
        assert_eq!(format_duration(Some(3_600_000 + 4 * 60_000)), "1h 04m");
    }

    #[test]
    fn pid_alive_reports_self_alive_and_unused_pid_dead() {
        let me = std::process::id() as i32;
        assert!(pid_alive(me));
        assert!(!pid_alive(0));
        assert!(!pid_alive(-1));
    }
}
