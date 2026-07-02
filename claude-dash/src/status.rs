//! `claude-dash status` — a one-shot SwiftBar readout of the current **Budget**.
//!
//! SwiftBar re-runs this on a fixed cadence (the `.15s.` in the plugin filename)
//! and renders stdout as a menu-bar item: line 1 is the title, then a `---`
//! separator, then dropdown lines. We read the **most recent** `req` **Budget**
//! straight from the store — the same "newest `req` across all **Session**s"
//! selector the dashboard uses ([`store::newest_req_in_views`]) — so the menu bar
//! and the TUI always agree. No **Proxy**, no daemon: the store is the source.
//!
//! The headline number is the **Representative Window**'s **Utilization**
//! (via [`Budget::representative`]), coloured by [`Budget::severity`] — matching
//! the dashboard's binding-window emphasis and severity palette. It's prefixed by
//! the white Claude mark, emitted as an always-white SwiftBar `image=`
//! (see [`SPLASH_ICON`]).

use std::path::Path;

use anyhow::Result;
use chrono::{DateTime, Local, TimeZone};

use crate::budget::{self, Budget, Severity, Window};
use crate::store;

/// Base64 of `assets/splash.png` — the white Claude mark, emitted as SwiftBar's
/// `image=` so it renders always-white (not tinted to the menu-bar colour).
/// Regenerate from `assets/splash.svg` (the Claude mark recoloured white; see
/// `assets/`); single line, no wrapping.
const SPLASH_ICON: &str = "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";

/// Entry point for `claude-dash status`: resolve the store, format the SwiftBar
/// output, print it, and exit 0. Always `Ok(())` — SwiftBar needs exit 0 + stdout
/// even when there's no data yet.
pub fn run() -> Result<()> {
    let dir = store::sessions_dir()?;
    print!("{}", render_dir(&dir));
    Ok(())
}

/// Read the newest **Budget** from a store `dir` and format the SwiftBar output.
/// Split from [`render`] so a fixture store dir can drive it end-to-end (store
/// selection + formatting) in one test.
fn render_dir(dir: &Path) -> String {
    let views = store::session_views_in_dir(dir);
    let budget = store::newest_req_in_views(&views).map(|req| req.budget.clone());
    render(budget.as_ref())
}

/// Format the SwiftBar output for an optional **Budget**. Pure over its input so
/// it's unit-testable without touching the filesystem. Resets are formatted as
/// absolute local times from the stored epochs, so no wall-clock is needed.
fn render(budget: Option<&Budget>) -> String {
    let Some(b) = budget else {
        // No-data path: benign title + one dropdown line, still exit 0.
        return format!("— | image={}\n---\nno usage data yet\n", SPLASH_ICON);
    };

    // Headline = the Representative (binding) Window's Utilization, coloured by
    // the account-wide severity for that window — mirrors the dashboard.
    let (rep_util, _) = b.window(b.representative());
    let title = format!(
        "{}% | image={} color={}\n",
        budget::percent(rep_util),
        SPLASH_ICON,
        swiftbar_color(b.severity(rep_util)),
    );

    // Dropdown: both windows with their % and reset, each coloured by its own
    // severity (same per-window palette as the dashboard rail).
    let (b5_util, b5_reset) = b.window(Window::FiveHour);
    let (b7_util, b7_reset) = b.window(Window::SevenDay);
    let five = window_line("5-hour", b5_util, b5_reset, b.severity(b5_util), reset_time);
    let seven = window_line("7-day", b7_util, b7_reset, b.severity(b7_util), reset_weekday);

    format!("{title}---\n{five}\n{seven}\n")
}

/// One dropdown line: `<name>  <pct>%  (resets <when>) | color=<sev>`. `fmt_reset`
/// renders the reset epoch (time-of-day for 5-hour, weekday for 7-day).
fn window_line(
    name: &str,
    util: f64,
    reset_epoch: i64,
    severity: Severity,
    fmt_reset: fn(i64) -> String,
) -> String {
    format!(
        "{name}  {}%  (resets {}) | color={}",
        budget::percent(util),
        fmt_reset(reset_epoch),
        swiftbar_color(severity),
    )
}

/// Map a [`Severity`] to a SwiftBar `color=` value — the render-edge translation
/// matching the dashboard palette (Ok→green, Warning→yellow, Critical→red).
fn swiftbar_color(severity: Severity) -> &'static str {
    match severity {
        Severity::Ok => "green",
        Severity::Warning => "yellow",
        Severity::Critical => "red",
    }
}

/// Format a reset epoch (seconds) as a local time-of-day, e.g. `3:41pm`. Used for
/// the fast-moving 5-hour window. Returns `—` for an absent (0/negative) epoch.
fn reset_time(epoch: i64) -> String {
    match local(epoch) {
        Some(dt) => dt.format("%-I:%M%P").to_string(),
        None => "—".to_string(),
    }
}

/// Format a reset epoch (seconds) as a local weekday, e.g. `Fri`. Used for the
/// slow 7-day window. Returns `—` for an absent (0/negative) epoch.
fn reset_weekday(epoch: i64) -> String {
    match local(epoch) {
        Some(dt) => dt.format("%a").to_string(),
        None => "—".to_string(),
    }
}

/// Resolve an epoch-seconds reset into local time, or `None` when it's absent
/// (0/negative — a partial reading that carried no reset).
fn local(epoch: i64) -> Option<DateTime<Local>> {
    if epoch <= 0 {
        return None;
    }
    Local.timestamp_opt(epoch, 0).single()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::{Record, ReqRecord};
    use crate::store::{append_record, session_path};

    fn budget(rep: &str, b5: f64, b7: f64) -> Budget {
        Budget {
            b5_util: b5,
            b5_reset: 1_750_000_000,
            b7_util: b7,
            b7_reset: 1_750_500_000,
            rep: rep.to_string(),
            status: "allowed".to_string(),
            ..Default::default()
        }
    }

    fn req(ts: i64, b: &Budget) -> Record {
        Record::Req(ReqRecord::from_budget(b, ts, None))
    }

    /// The one focused test: a fixture store with two `req`s across two sessions;
    /// `status` must select the NEWEST record's Budget and headline the
    /// Representative Window's % (7-day here, since rep="seven_day").
    #[test]
    fn selects_newest_budget_and_headlines_representative_percent() {
        let dir = tempfile::tempdir().unwrap();

        // Older reading (five_hour rep, 42%).
        let a = session_path(dir.path(), "aaaa");
        append_record(&a, &req(100, &budget("five_hour", 0.42, 0.10))).unwrap();

        // Newest reading (seven_day rep, 7d util 0.33) in a second session.
        let b = session_path(dir.path(), "bbbb");
        append_record(&b, &req(400, &budget("seven_day", 0.20, 0.33))).unwrap();

        let out = render_dir(dir.path());
        let title = out.lines().next().unwrap();

        // Newest record wins (ts 400), and its Representative Window is 7-day →
        // headline is 33%, not the older record's or the 5-hour window's, carried
        // by the always-white Claude mark image.
        assert!(title.starts_with("33%"), "title was {title:?}");
        assert!(title.contains("image="), "title was {title:?}");
        assert!(!title.contains("templateImage="), "title was {title:?}");
        assert!(out.contains("---"));
        assert!(out.contains("5-hour  20%"));
        assert!(out.contains("7-day  33%"));
    }

    #[test]
    fn no_data_prints_benign_title_and_still_formats() {
        let dir = tempfile::tempdir().unwrap();
        let out = render_dir(dir.path());
        assert!(out.starts_with("—"), "out was {out:?}");
        assert!(out.contains("image="), "out was {out:?}");
        assert!(!out.contains("templateImage="), "out was {out:?}");
        assert!(out.contains("no usage data yet"));
    }
}
