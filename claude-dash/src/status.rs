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
//! a monochrome radial-burst "splash" glyph, emitted as a SwiftBar
//! `templateImage=` (see [`SPLASH_ICON`]) so it tints to the menu-bar colour.

use std::path::Path;

use anyhow::Result;
use chrono::{DateTime, Local, TimeZone};

use crate::budget::{self, Budget, Severity, Window};
use crate::store;

/// Base64 of `assets/splash.png` — a monochrome 12-spoke radial burst, emitted as
/// SwiftBar's `templateImage=` so it renders as a Claude-style sunburst and is
/// tinted to the menu-bar colour (adapting to light/dark). Regenerate from
/// `assets/splash.svg` (see `assets/`); single line, no wrapping.
const SPLASH_ICON: &str = "iVBORw0KGgoAAAANSUhEUgAAACwAAAAsCAYAAAAehFoBAAAAIGNIUk0AAHomAACAhAAA+gAAAIDoAAB1MAAA6mAAADqYAAAXcJy6UTwAAAAGYktHRAD/AP8A/6C9p5MAAAAHdElNRQfqBwESCznSxA/sAAAAJXRFWHRkYXRlOmNyZWF0ZQAyMDI2LTA3LTAxVDE4OjExOjU3KzAwOjAw/jyQpwAAACV0RVh0ZGF0ZTptb2RpZnkAMjAyNi0wNy0wMVQxODoxMTo1NyswMDowMI9hKBsAAAAodEVYdGRhdGU6dGltZXN0YW1wADIwMjYtMDctMDFUMTg6MTE6NTcrMDA6MDDYdAnEAAAClElEQVRYw+2ZQWsTQRTHfzbtRVqbiyJ4SPwGQsgp5NQiSr5ALi25GIT2mkNBWirknA/hZ5Ae+g0EwU+gLVhaai4FeynWeMhbnLx9uzOju9gF/zBkZ+b//u9lZvbNzi78RzWxA0zl986jB8yc0itSfOkPbBpAO8f2mafu+m6LXmkBt4AT4ANwBqwanCtPHbE7E50T0S0l4JfO9WNgFGmfYCT2lm6hAd+q+j6RUyr8fY9uYVgGbli8qY4UZ1f176r+I9V/I7pBiB3hH8CGansBdALtO8J3sSG6peJYjdI1UJO+rBGuCc/tO451nDXCLeANsIc9Xa9U/T6w5fG1Jbw8HcTfnvgPyh4N0musa/DGijcD6hkjXDe4Y0OzS/oe8d7UbUM8mb6mw1szOJOMgCcGd83RapJeZklp+wJeAs4zjJORSZz1jf5DT30mdsmfHuf4OicwMaxmONJOl4EL1f7ZU78Qu75H/xB7J81Fg3Te1M63PY512Tb+pM7rsZtRCh3SKcktXwODzeNdE57Pg1ADBpGjGVoG/M7jhaOOfde7Zch8jQ49vInoBeOeqveYP79eBdg+ZL4ZPPVoIsG5+AK8A74F+FkHPgHvdccOfz+1wwynvpEOKanj1rQA0T42fCkspExhMSkfBEyPDw8i22NwABVfw6Go8w+zRAwqlYcrs9NV5lmiUk9rlXsertyJo6EM7vyZDvyn5qYRwED6so75A8OmaWhHn5pDcOfeS+ShC2yqtuf434/dCs/FJvZyKwyVe7c2AlZU2+tIDc1fIeK1bWzAer9/C5xGapyKXZ5uYWixmNStnci3JBA7d3MKzgbBa0fwkXkqeiTXPw3OuqcO8B14IoFeEj9LhaLUr0hloVLf6UrFL9meUIaxNzbJAAAAAElFTkSuQmCC";

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
        return format!("— | templateImage={}\n---\nno usage data yet\n", SPLASH_ICON);
    };

    // Headline = the Representative (binding) Window's Utilization, coloured by
    // the account-wide severity for that window — mirrors the dashboard.
    let (rep_util, _) = b.window(b.representative());
    let title = format!(
        "{}% | templateImage={} color={}\n",
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
        // by the burst template image.
        assert!(title.starts_with("33%"), "title was {title:?}");
        assert!(title.contains("templateImage="), "title was {title:?}");
        assert!(out.contains("---"));
        assert!(out.contains("5-hour  20%"));
        assert!(out.contains("7-day  33%"));
    }

    #[test]
    fn no_data_prints_benign_title_and_still_formats() {
        let dir = tempfile::tempdir().unwrap();
        let out = render_dir(dir.path());
        assert!(out.starts_with("—"), "out was {out:?}");
        assert!(out.contains("templateImage="), "out was {out:?}");
        assert!(out.contains("no usage data yet"));
    }
}
