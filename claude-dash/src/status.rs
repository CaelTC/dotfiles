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
//! (see [`splash_icon`]).

use std::path::Path;
use std::sync::OnceLock;

use anyhow::Result;
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use chrono::{DateTime, Local, TimeZone};

use crate::budget::{self, Budget, Severity, Window};
use crate::store;

/// Raw bytes of `assets/splash.png` — the white Claude mark (44×44 RGBA, @2x),
/// rasterized from `assets/splash.svg`. Compiled in with `include_bytes!` so the
/// embedded icon is always byte-identical to the committed asset (a hand-pasted
/// base64 const once shipped two mangled bytes, corrupting the IDAT chunk and
/// leaving SwiftBar to silently render no icon at all).
const SPLASH_PNG: &[u8] = include_bytes!("../assets/splash.png");

/// Display size (points) for the menu-bar mark. SwiftBar hands the decoded PNG
/// straight to `NSImage(data:)` and sets it on the status-item button with no
/// size cap, and a 44px PNG without DPI metadata is 44 *points* — which dwarfed
/// the menu bar. SwiftBar's `MenuLineParameters.resizedImageIfRequested` scales
/// the image only when BOTH `width=` and `height=` params are present, so both
/// are emitted below. 18pt is the standard menu-bar icon size; the 44px bitmap
/// stays as the backing so the mark is crisp on retina (@2x = 36px < 44px).
const SPLASH_ICON_POINTS: u32 = 18;

/// Base64 of [`SPLASH_PNG`] for SwiftBar's `image=` (NOT `templateImage=`, so it
/// renders always-white instead of tinting to the menu-bar colour). Encoded once
/// on first use; single line by construction, as SwiftBar requires.
fn splash_icon() -> &'static str {
    static ICON: OnceLock<String> = OnceLock::new();
    ICON.get_or_init(|| BASE64.encode(SPLASH_PNG))
}

/// Entry point for `claude-dash status`: resolve the store, format the SwiftBar
/// output, print it, and exit 0. Always `Ok(())` — SwiftBar needs exit 0 + stdout
/// even when there's no data yet.
pub fn run() -> Result<()> {
    let dir = store::sessions_dir()?;
    print!("{}", render_dir(&dir, Local::now().timestamp()));
    Ok(())
}

/// Read the newest **Budget** from a store `dir` and format the SwiftBar output.
/// Split from [`render`] so a fixture store dir can drive it end-to-end (store
/// selection + formatting) in one test.
fn render_dir(dir: &Path, now_epoch: i64) -> String {
    let views = store::session_views_in_dir(dir);
    let budget = store::newest_req_in_views(&views).map(|req| req.budget.clone());
    render(budget.as_ref(), now_epoch)
}

/// Format the SwiftBar output for an optional **Budget** as of `now_epoch`
/// (seconds). Pure over its inputs so it's unit-testable without touching the
/// filesystem. Reset *times* are formatted from the stored epochs; `now_epoch` is
/// only needed to zero a window whose reset has already passed (see
/// [`Budget::util_at`]).
fn render(budget: Option<&Budget>, now_epoch: i64) -> String {
    let Some(b) = budget else {
        // No-data path: benign title + one dropdown line, still exit 0.
        return format!(
            "— | image={} width={s} height={s}\n---\nno usage data yet\n",
            splash_icon(),
            s = SPLASH_ICON_POINTS,
        );
    };

    // Headline = the Representative (binding) Window's Utilization. The title %
    // stays white regardless of severity (dropdown lines still carry severity
    // colour); severity is conveyed by the dashboard, not the menu-bar number.
    let rep_util = b.util_at(b.representative(), now_epoch);
    let title = format!(
        "{}% | image={} width={s} height={s} color=white\n",
        budget::percent(rep_util),
        splash_icon(),
        s = SPLASH_ICON_POINTS,
    );

    // Dropdown: both windows with their % and reset, each coloured by its own
    // severity (same per-window palette as the dashboard rail).
    let (_, b5_reset) = b.window(Window::FiveHour);
    let (_, b7_reset) = b.window(Window::SevenDay);
    let b5_util = b.util_at(Window::FiveHour, now_epoch);
    let b7_util = b.util_at(Window::SevenDay, now_epoch);
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

        // `now` before both fixture resets (1.75e9), so no window is zeroed here.
        let out = render_dir(dir.path(), 1_700_000_000);
        let title = out.lines().next().unwrap();

        // Newest record wins (ts 400), and its Representative Window is 7-day →
        // headline is 33%, not the older record's or the 5-hour window's, carried
        // by the always-white Claude mark image at menu-bar point size.
        assert!(title.starts_with("33%"), "title was {title:?}");
        assert!(title.contains("image="), "title was {title:?}");
        assert!(!title.contains("templateImage="), "title was {title:?}");
        // SwiftBar scales a title image only when BOTH width= and height= are
        // present; without them the 44px PNG renders at 44pt and floods the bar.
        assert!(
            title.contains("width=18 height=18"),
            "title must pin the menu-bar icon size, was {title:?}"
        );
        assert!(out.contains("---"));
        assert!(out.contains("5-hour  20%"));
        assert!(out.contains("7-day  33%"));
    }

    #[test]
    fn no_data_prints_benign_title_and_still_formats() {
        let dir = tempfile::tempdir().unwrap();
        let out = render_dir(dir.path(), 1_700_000_000);
        assert!(out.starts_with("—"), "out was {out:?}");
        assert!(out.contains("image="), "out was {out:?}");
        assert!(!out.contains("templateImage="), "out was {out:?}");
        assert!(
            out.contains("width=18 height=18"),
            "no-data title must pin the menu-bar icon size too, was {out:?}"
        );
        assert!(out.contains("no usage data yet"));
    }

    /// Once a window's reset has passed, its Utilization reads 0 without any fresh
    /// reading — the idle-account case the whole feature exists for. Same fixture
    /// budget (5h=42%, 7d=10%, rep=five_hour), rendered with `now` *after* the
    /// 5-hour reset but *before* the 7-day reset: the headline and 5-hour line drop
    /// to 0%, the 7-day line still shows its stored 10%.
    #[test]
    fn windows_read_zero_once_their_reset_has_passed() {
        let b = budget("five_hour", 0.42, 0.10); // b5_reset=1.75e9, b7_reset=1.7505e9
        let out = render(Some(&b), 1_750_000_001);
        let title = out.lines().next().unwrap();
        assert!(title.starts_with("0%"), "5h reset passed → headline 0%, was {title:?}");
        assert!(out.contains("5-hour  0%"), "5h line zeroed, was {out:?}");
        assert!(out.contains("7-day  10%"), "7d not yet reset, was {out:?}");
    }

    /// The embedded icon must round-trip: SwiftBar silently renders NO icon for
    /// an undecodable image, which is exactly how a hand-mangled base64 const
    /// once shipped. Decoding [`splash_icon`] back to the committed PNG bytes
    /// (and checking the PNG signature + SwiftBar's single-line requirement)
    /// pins the whole embed path.
    #[test]
    fn splash_icon_decodes_back_to_the_committed_png() {
        let icon = splash_icon();
        assert!(
            !icon.contains(['\n', '\r', ' ']),
            "SwiftBar image= must be a single unbroken line"
        );
        let decoded = BASE64.decode(icon).expect("splash icon must be valid base64");
        assert_eq!(decoded, SPLASH_PNG, "encode/decode must round-trip");
        assert_eq!(&SPLASH_PNG[..8], b"\x89PNG\r\n\x1a\n", "asset must be a PNG");
    }
}
