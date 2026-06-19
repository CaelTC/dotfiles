//! Turn-done alerts: ping the machine when an **Active Session** finishes its
//! turn — i.e. **Claude** stops producing requests and goes quiet, waiting for
//! the user's next message. This is the "I walked away, tell me when it's my
//! turn again" signal, distinct from a **Session** *ending* (the `claude`
//! process exiting), which the [`crate::lifecycle`] classifier handles.
//!
//! In the same deep-seam spirit as `Budget::from_headers` and `classify`: the
//! decision lives in pure functions over a [`SessionView`] plus the current
//! clock — [`is_live`] (is this session live or idle right now?) and
//! [`TurnWatcher::settle`] (which sessions *just* crossed live→idle?) — while the
//! only I/O, posting the OS banner, is the one-line fire-and-forget adapter
//! [`notify_macos`]. So the edge detection is unit-testable with no terminal, no
//! filesystem, and no real notifications.
//!
//! "Live" vs "idle" is purely a gap on the **Throughput** stream: a **Session**
//! is *live* while its most recent `req` is within `idle_after_ms`, and *idle*
//! once it has been quiet at least that long. Because requests within one turn
//! can have gaps (a long tool call), the threshold is a coarse heuristic, not a
//! true turn boundary — a tool call longer than the threshold can produce an
//! early "done" that a later request supersedes. `CLAUDE_DASH_IDLE_SECS` tunes
//! it.

use std::collections::HashSet;
use std::process::Command;

use crate::store::SessionView;

/// One **Active Session** that just finished its turn (crossed live→idle), ready
/// to be turned into a notification. Carries the same `project · model · id`
/// label the active panel shows, so the banner names the session at a glance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnDone {
    /// The **Session** id (store key) — also the dedup key in [`TurnWatcher`].
    pub id: String,
    /// The session's `project · model · id` label for the banner body.
    pub label: String,
}

/// Whether an **Active Session** is *live* (a `req` arrived within
/// `idle_after_ms` of `now_ms`) or *idle* (quiet at least that long, or no `req`
/// captured yet).
///
/// Pure over the [`SessionView`] and the clock. A session with no `req`s is not
/// live — it has never produced **Throughput**, so there is no turn to finish.
pub fn is_live(view: &SessionView, now_ms: i64, idle_after_ms: i64) -> bool {
    match view.reqs.iter().map(|r| r.ts).max() {
        Some(last_ts) => now_ms - last_ts < idle_after_ms,
        None => false,
    }
}

/// Edge-detector over the live/idle state of the **Active Session**s: remembers
/// which sessions are currently live so each settle returns only those that
/// *just* crossed live→idle (Claude finished its turn).
///
/// Stateful by necessity — a transition is a change between two ticks — but the
/// transition rule itself ([`settle`](TurnWatcher::settle)) is pure over the
/// prior state plus the current actives and clock, so it is tested with plain
/// `SessionView`s and no real notifications.
#[derive(Debug, Default)]
pub struct TurnWatcher {
    /// The ids of the **Active Session**s that were live as of the last settle.
    live: HashSet<String>,
}

impl TurnWatcher {
    /// A watcher that has seen nothing yet. On the first [`settle`](Self::settle)
    /// no session can register a transition (an already-idle session was never
    /// recorded as live), so starting the dashboard onto pre-existing idle
    /// sessions raises no spurious ping.
    pub fn new() -> Self {
        Self::default()
    }

    /// Update the watcher with the current **Active Session**s and return those
    /// that just crossed live→idle since the previous settle.
    ///
    /// The rule, per active session:
    /// - live now, not previously live ⇒ record as live (no event).
    /// - idle now, previously live ⇒ a turn finished — emit a [`TurnDone`] and
    ///   forget it (so re-activating then idling again pings again next turn).
    ///
    /// A session that disappeared from `active` (its **Session** *ended*) is
    /// pruned silently — process exit is not a turn-done. So a ping fires only
    /// for a session that is still running but went quiet.
    pub fn settle(
        &mut self,
        active: &[&SessionView],
        now_ms: i64,
        idle_after_ms: i64,
    ) -> Vec<TurnDone> {
        let active_ids: HashSet<&str> = active.iter().map(|v| v.id.as_str()).collect();
        // Drop any remembered session that is no longer active (it ended) — its
        // exit is the lifecycle's concern, not a turn-done.
        self.live.retain(|id| active_ids.contains(id.as_str()));

        let mut done = Vec::new();
        for view in active {
            let live_now = is_live(view, now_ms, idle_after_ms);
            let was_live = self.live.contains(&view.id);
            if live_now && !was_live {
                self.live.insert(view.id.clone());
            } else if !live_now && was_live {
                self.live.remove(&view.id);
                done.push(TurnDone {
                    id: view.id.clone(),
                    label: label(view),
                });
            }
        }
        done
    }
}

/// The `project · model · id` label for a **Session**, matching the active
/// panel's title — project from the `start` record, **Model** from the freshest
/// captured **Throughput**, falling back to `—` when absent.
fn label(view: &SessionView) -> String {
    let project = view
        .start
        .as_ref()
        .map(|s| s.project.as_str())
        .filter(|p| !p.is_empty())
        .unwrap_or("—");
    let model = view
        .reqs
        .iter()
        .rev()
        .find_map(|r| r.throughput.as_ref())
        .map(|tp| tp.model.as_str())
        .filter(|m| !m.is_empty())
        .unwrap_or("—");
    format!("{project} · {model} · {}", view.id)
}

/// Post a macOS banner notification, fire-and-forget. The one I/O adapter behind
/// the turn-done seam: it spawns `osascript` and never blocks on, waits for, or
/// panics over the result — a missing/failing notifier must never disturb the
/// dashboard (the same fail-open spirit as the capture path).
///
/// Both `title` and `body` are interpolated into an AppleScript string literal,
/// so each is escaped via [`escape_applescript`] to keep an embedded quote or
/// backslash from breaking the script.
pub fn notify_macos(title: &str, body: &str) {
    let script = format!(
        "display notification \"{}\" with title \"{}\"",
        escape_applescript(body),
        escape_applescript(title),
    );
    // spawn (not output/status): we neither wait nor care if it fails.
    let _ = Command::new("osascript").arg("-e").arg(script).spawn();
}

/// Escape a string for embedding inside an AppleScript double-quoted literal:
/// backslash and double-quote are the only characters that can terminate or
/// corrupt the literal.
fn escape_applescript(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::Budget;
    use crate::record::{ReqRecord, StartRecord};
    use crate::throughput::Throughput;

    fn req(ts: i64, model: Option<&str>) -> ReqRecord {
        let tp = model.map(|m| Throughput {
            input: 10,
            output: 0,
            cache_read: 0,
            cache_write: 0,
            model: m.to_string(),
        });
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

    fn view(id: &str, project: &str, reqs: Vec<ReqRecord>) -> SessionView {
        SessionView {
            id: id.to_string(),
            start: Some(StartRecord {
                id: id.to_string(),
                ts: 0,
                project: project.to_string(),
                cwd: format!("/work/{project}"),
                pid: 1,
            }),
            reqs,
            end: None,
        }
    }

    const IDLE_AFTER: i64 = 30_000; // 30s

    #[test]
    fn is_live_true_within_window_false_when_quiet_or_empty() {
        let now = 1_000_000;
        let recent = view("s", "p", vec![req(now - 5_000, None)]);
        let stale = view("s", "p", vec![req(now - 60_000, None)]);
        let empty = view("s", "p", vec![]);
        assert!(is_live(&recent, now, IDLE_AFTER));
        assert!(!is_live(&stale, now, IDLE_AFTER));
        assert!(!is_live(&empty, now, IDLE_AFTER));
    }

    #[test]
    fn settle_emits_once_on_live_to_idle_transition() {
        let mut w = TurnWatcher::new();
        let now = 1_000_000;

        // Tick 1: a live session — recorded, no event yet.
        let live = view("a", "proj", vec![req(now - 1_000, Some("claude-opus-4-8"))]);
        assert!(w.settle(&[&live], now, IDLE_AFTER).is_empty());

        // Tick 2: same session, now quiet past the threshold — one turn-done.
        let later = now + IDLE_AFTER + 1_000;
        let done = w.settle(&[&live], later, IDLE_AFTER);
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].id, "a");
        assert_eq!(done[0].label, "proj · claude-opus-4-8 · a");

        // Tick 3: still idle — no repeat ping for the same quiet period.
        assert!(w.settle(&[&live], later + 1_000, IDLE_AFTER).is_empty());
    }

    #[test]
    fn settle_pings_again_after_reactivation() {
        let mut w = TurnWatcher::new();
        let t0 = 1_000_000;
        let v1 = view("a", "p", vec![req(t0, None)]);
        w.settle(&[&v1], t0, IDLE_AFTER); // live
        let idle_at = t0 + IDLE_AFTER + 1;
        assert_eq!(w.settle(&[&v1], idle_at, IDLE_AFTER).len(), 1); // turn 1 done

        // A new request arrives — live again — then quiets a second time.
        let v2 = view("a", "p", vec![req(idle_at + 1_000, None)]);
        assert!(w.settle(&[&v2], idle_at + 1_500, IDLE_AFTER).is_empty()); // live
        let idle2 = idle_at + 1_000 + IDLE_AFTER + 1;
        assert_eq!(w.settle(&[&v2], idle2, IDLE_AFTER).len(), 1); // turn 2 done
    }

    #[test]
    fn already_idle_session_at_startup_does_not_ping() {
        // First-ever settle onto a session that is already quiet: no transition.
        let mut w = TurnWatcher::new();
        let now = 1_000_000;
        let stale = view("a", "p", vec![req(now - 60_000, None)]);
        assert!(w.settle(&[&stale], now, IDLE_AFTER).is_empty());
    }

    #[test]
    fn ended_session_is_pruned_without_pinging() {
        // A live session that vanishes from `active` (it ended) must not ping —
        // process exit is the lifecycle's concern, not a turn-done.
        let mut w = TurnWatcher::new();
        let now = 1_000_000;
        let live = view("a", "p", vec![req(now, None)]);
        w.settle(&[&live], now, IDLE_AFTER); // recorded live
        // Next tick it's gone from the active set.
        let empty: [&SessionView; 0] = [];
        assert!(w.settle(&empty, now + IDLE_AFTER + 1, IDLE_AFTER).is_empty());
    }

    #[test]
    fn label_falls_back_to_dashes() {
        let v = SessionView {
            id: "z9".to_string(),
            start: None,
            reqs: vec![req(1, None)], // a req but no throughput → model "—"
            end: None,
        };
        assert_eq!(label(&v), "— · — · z9");
    }

    #[test]
    fn escape_applescript_escapes_quotes_and_backslashes() {
        assert_eq!(escape_applescript(r#"a"b\c"#), r#"a\"b\\c"#);
        assert_eq!(escape_applescript("plain"), "plain");
    }
}
