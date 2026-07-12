//! **Budget** — the account-wide facet of **Usage**: how much of each
//! **Rolling Window** is consumed and when it resets.
//!
//! Captured live by the **Proxy** from Anthropic's `anthropic-ratelimit-unified-*`
//! response headers, which give **Utilization** directly as a 0–1 fraction per
//! window plus reset times — authoritative, not an estimate.

/// The set of `anthropic-ratelimit-unified-*` headers the **Proxy** reads off a
/// `/v1/messages` response. Spans the two **Rolling Window**s (5-hour, 7-day),
/// each with its own **Utilization** and reset, plus the **Representative
/// Window** claim and an overall status.
///
/// **Utilization** is the raw 0–1 fraction reported by Anthropic (no denominator
/// is assumed — the fraction is given outright). `reset` is epoch seconds.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Budget {
    /// 5-hour **Rolling Window** **Utilization** (0–1 fraction).
    pub b5_util: f64,
    /// 5-hour **Rolling Window** reset, epoch seconds.
    pub b5_reset: i64,
    /// 7-day **Rolling Window** **Utilization** (0–1 fraction).
    pub b7_util: f64,
    /// 7-day **Rolling Window** reset, epoch seconds.
    pub b7_reset: i64,
    /// The **Representative Window** claim (e.g. `five_hour`) — whichever
    /// **Rolling Window** Anthropic currently flags as binding.
    pub rep: String,
    /// Overall unified rate-limit status (e.g. `allowed`).
    pub status: String,
    /// The unified *overage* status (e.g. `allowed`, `disabled`) when the
    /// account has overage configured. Empty when the headers report none.
    ///
    /// Optional + `#[serde(default)]` so `req` records written by slices 01–05
    /// — which carry no overage fields — still deserialize (the field defaults
    /// to empty).
    #[serde(default)]
    pub overage_status: String,
    /// Why overage is disabled, when it is (the unified
    /// `-overage-disabled-reason` header). Empty when not reported. Same
    /// backward-compatible default as `overage_status`.
    #[serde(default)]
    pub overage_disabled_reason: String,
    /// The unified `-fallback-percentage` — present only when the headers
    /// report a fallback. `None` (absent) for old records and for responses
    /// without the header.
    #[serde(default)]
    pub fallback_percentage: Option<f64>,
}

/// Header name carrying the 5-hour **Utilization** fraction.
pub const H_5H_UTIL: &str = "anthropic-ratelimit-unified-5h-utilization";
/// Header name carrying the 5-hour reset (epoch seconds).
pub const H_5H_RESET: &str = "anthropic-ratelimit-unified-5h-reset";
/// Header name carrying the 7-day **Utilization** fraction.
pub const H_7D_UTIL: &str = "anthropic-ratelimit-unified-7d-utilization";
/// Header name carrying the 7-day reset (epoch seconds).
pub const H_7D_RESET: &str = "anthropic-ratelimit-unified-7d-reset";
/// Header name carrying the **Representative Window** claim.
pub const H_REP: &str = "anthropic-ratelimit-unified-representative-claim";
/// Header name carrying the overall unified status.
pub const H_STATUS: &str = "anthropic-ratelimit-unified-status";
/// Header name carrying the unified *overage* status.
pub const H_OVERAGE_STATUS: &str = "anthropic-ratelimit-unified-overage-status";
/// Header name carrying why overage is disabled, when it is.
pub const H_OVERAGE_DISABLED_REASON: &str =
    "anthropic-ratelimit-unified-overage-disabled-reason";
/// Header name carrying the unified fallback percentage.
pub const H_FALLBACK_PCT: &str = "anthropic-ratelimit-unified-fallback-percentage";

impl Budget {
    /// Parse a **Budget** reading from `anthropic-ratelimit-unified-*` headers.
    ///
    /// `lookup` resolves a header name (lowercase) to its value, letting this
    /// stay pure and testable independent of any HTTP types. Returns `None`
    /// unless at least one window's **Utilization** is present — a response with
    /// no unified rate-limit headers carries no **Budget**.
    pub fn from_headers<F>(lookup: F) -> Option<Budget>
    where
        F: Fn(&str) -> Option<String>,
    {
        let b5_util = lookup(H_5H_UTIL).and_then(|v| v.trim().parse::<f64>().ok());
        let b7_util = lookup(H_7D_UTIL).and_then(|v| v.trim().parse::<f64>().ok());

        // No unified utilization at all ⇒ this response carries no Budget.
        if b5_util.is_none() && b7_util.is_none() {
            return None;
        }

        Some(Budget {
            b5_util: b5_util.unwrap_or(0.0),
            b5_reset: lookup(H_5H_RESET)
                .and_then(|v| v.trim().parse::<i64>().ok())
                .unwrap_or(0),
            b7_util: b7_util.unwrap_or(0.0),
            b7_reset: lookup(H_7D_RESET)
                .and_then(|v| v.trim().parse::<i64>().ok())
                .unwrap_or(0),
            rep: lookup(H_REP).unwrap_or_default(),
            status: lookup(H_STATUS).unwrap_or_default(),
            overage_status: lookup(H_OVERAGE_STATUS).unwrap_or_default(),
            overage_disabled_reason: lookup(H_OVERAGE_DISABLED_REASON).unwrap_or_default(),
            fallback_percentage: lookup(H_FALLBACK_PCT)
                .and_then(|v| v.trim().parse::<f64>().ok()),
        })
    }

    /// Which **Rolling Window** is the **Representative Window** — the binding
    /// constraint Anthropic flags via `representative-claim`. Drives which window
    /// the left rail visually emphasises. An unknown/empty claim defaults to the
    /// 5-hour window (the tighter, faster-moving one — the safer headline when
    /// Anthropic doesn't say).
    pub fn representative(&self) -> Window {
        match self.rep.as_str() {
            "seven_day" => Window::SevenDay,
            // "five_hour" — and any unknown/empty claim — headline the 5h window.
            _ => Window::FiveHour,
        }
    }

    /// The `(util, reset)` of a given **Rolling Window** — the lookup that turns
    /// a [`Window`] handle into that window's stored **Utilization** fraction and
    /// reset epoch. Lets callers headline the **Representative Window** (via
    /// [`representative`](Self::representative)) without re-deriving which field
    /// belongs to which window.
    pub fn window(&self, window: Window) -> (f64, i64) {
        match window {
            Window::FiveHour => (self.b5_util, self.b5_reset),
            Window::SevenDay => (self.b7_util, self.b7_reset),
        }
    }

    /// A window's **Utilization** as it stands at `now_epoch` (seconds): the
    /// stored fraction, or `0` once the window's reset has passed. Anthropic
    /// reports **Utilization** for the *current* window, so once wall-clock
    /// crosses the stored reset the old fraction is stale — the window has rolled
    /// over to empty. We zero it locally rather than wait for the next response to
    /// carry the fresh reading, so an idle account's gauge drops at the reset time
    /// instead of hanging at its last value. A missing reset (`<= 0`) is left
    /// as-is — there's no boundary to have crossed.
    pub fn util_at(&self, window: Window, now_epoch: i64) -> f64 {
        let (util, reset) = self.window(window);
        if reset > 0 && now_epoch >= reset {
            0.0
        } else {
            util
        }
    }

    /// The **Severity** of a window, driven by the unified `status` *and* the
    /// window's **Utilization** — the single place Budget coloring is decided
    /// (subsuming the old utilization-only threshold).
    ///
    /// `status`/`overage_status` are account-wide, so they gate *both* windows:
    /// a `rejected` (or overage-`disabled`) account is [`Severity::Critical`]
    /// regardless of a given window's fraction. Otherwise utilization decides:
    /// filling past 0.9 is Critical, past 0.6 is Warning, else Ok.
    pub fn severity(&self, util: f64) -> Severity {
        // A rejected account, or an overage that's been disabled, is the binding
        // failure — Critical for both windows no matter the fraction.
        let rejected = self.status.eq_ignore_ascii_case("rejected");
        if rejected || self.overage_disabled() {
            return Severity::Critical;
        }

        // Otherwise utilization drives it (allowed → green, filling → amber/red).
        if util >= 0.9 {
            Severity::Critical
        } else if util >= 0.6 {
            Severity::Warning
        } else {
            Severity::Ok
        }
    }

    /// The **overage** indicator the rail should surface, or `None` when the
    /// headers report no overage state worth showing. Pure text — the TUI just
    /// renders the `label`, coloured by `severity`.
    pub fn overage(&self) -> Option<Overage> {
        // No overage headers at all ⇒ nothing to surface.
        if self.overage_status.is_empty()
            && self.overage_disabled_reason.is_empty()
            && self.fallback_percentage.is_none()
        {
            return None;
        }

        // A disabled overage is the loud case — show why, Critical.
        if self.overage_disabled() {
            let why = if self.overage_disabled_reason.is_empty() {
                String::new()
            } else {
                format!(" ({})", self.overage_disabled_reason)
            };
            return Some(Overage {
                label: format!("overage disabled{why}"),
                severity: Severity::Critical,
            });
        }

        // Otherwise an active/allowed overage, optionally with a fallback %.
        let status = if self.overage_status.is_empty() {
            "active".to_string()
        } else {
            self.overage_status.clone()
        };
        let fallback = match self.fallback_percentage {
            Some(pct) => format!(" · fallback {}%", (pct * 100.0).round() as i64),
            None => String::new(),
        };
        Some(Overage {
            label: format!("overage {status}{fallback}"),
            severity: Severity::Warning,
        })
    }

    /// Whether overage has been *disabled* — the loud, binding-failure case that
    /// drives both [`Severity::Critical`] and the disabled overage banner. One
    /// source of truth shared by [`severity`](Self::severity) and
    /// [`overage`](Self::overage) so they can't disagree.
    fn overage_disabled(&self) -> bool {
        self.overage_status.eq_ignore_ascii_case("disabled")
            || !self.overage_disabled_reason.is_empty()
    }
}

/// Render a 0–1 **Utilization** fraction as a whole-percent integer, clamped to
/// 0–100. The single fraction→percent rule shared by the dashboard gauge and the
/// menu-bar readout, so the two can't drift apart.
pub fn percent(util: f64) -> u16 {
    (util.clamp(0.0, 1.0) * 100.0).round() as u16
}

/// One of the two **Rolling Window**s. The render-time handle for which window
/// the left rail emphasises as the **Representative Window**.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Window {
    /// The 5-hour **Rolling Window**.
    FiveHour,
    /// The 7-day **Rolling Window**.
    SevenDay,
}

/// How alarming a **Rolling Window** (or the **overage** banner) is, driven by
/// `status` + **Utilization**. A ratatui-free enum so the *rule* is testable
/// without a terminal; the TUI maps it to a `Color` at the render edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// Healthy — well within the window (rendered green).
    Ok,
    /// Filling up — approaching the limit (rendered amber/yellow).
    Warning,
    /// At/over the limit, rejected, or overage-disabled (rendered red).
    Critical,
}

/// The **overage** indicator for the left rail: a one-line `label` and the
/// [`Severity`] to colour it. Pure presentation — no ratatui.
#[derive(Debug, Clone, PartialEq)]
pub struct Overage {
    /// The text to show, e.g. `overage active · fallback 80%` or
    /// `overage disabled (insufficient_credit)`.
    pub label: String,
    /// How alarming the overage state is.
    pub severity: Severity,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn lookup_from(map: HashMap<&'static str, &'static str>) -> impl Fn(&str) -> Option<String> {
        move |k: &str| map.get(k).map(|s| s.to_string())
    }

    #[test]
    fn parses_full_unified_headers_into_budget() {
        let map = HashMap::from([
            (H_5H_UTIL, "0.42"),
            (H_5H_RESET, "1750000000"),
            (H_7D_UTIL, "0.10"),
            (H_7D_RESET, "1750500000"),
            (H_REP, "five_hour"),
            (H_STATUS, "allowed"),
        ]);

        let budget = Budget::from_headers(lookup_from(map)).expect("budget present");

        assert_eq!(
            budget,
            Budget {
                b5_util: 0.42,
                b5_reset: 1_750_000_000,
                b7_util: 0.10,
                b7_reset: 1_750_500_000,
                rep: "five_hour".to_string(),
                status: "allowed".to_string(),
                ..Default::default()
            }
        );
    }

    #[test]
    fn parses_overage_headers_when_present() {
        let map = HashMap::from([
            (H_5H_UTIL, "0.42"),
            (H_STATUS, "allowed"),
            (H_OVERAGE_STATUS, "allowed"),
            (H_OVERAGE_DISABLED_REASON, "insufficient_credit"),
            (H_FALLBACK_PCT, "0.8"),
        ]);
        let budget = Budget::from_headers(lookup_from(map)).expect("budget present");
        assert_eq!(budget.overage_status, "allowed");
        assert_eq!(budget.overage_disabled_reason, "insufficient_credit");
        assert_eq!(budget.fallback_percentage, Some(0.8));
    }

    #[test]
    fn overage_fields_absent_when_headers_missing() {
        let map = HashMap::from([(H_5H_UTIL, "0.42"), (H_STATUS, "allowed")]);
        let budget = Budget::from_headers(lookup_from(map)).expect("budget present");
        assert_eq!(budget.overage_status, "");
        assert_eq!(budget.overage_disabled_reason, "");
        assert_eq!(budget.fallback_percentage, None);
    }

    #[test]
    fn old_record_without_overage_fields_still_deserializes() {
        // A `req` budget written by slices 01–05 carries no overage fields.
        let old = r#"{
            "b5_util": 0.42, "b5_reset": 1750000000,
            "b7_util": 0.10, "b7_reset": 1750500000,
            "rep": "five_hour", "status": "allowed"
        }"#;
        let budget: Budget = serde_json::from_str(old).expect("old record deserializes");
        assert_eq!(budget.b5_util, 0.42);
        assert_eq!(budget.overage_status, "");
        assert_eq!(budget.overage_disabled_reason, "");
        assert_eq!(budget.fallback_percentage, None);
    }

    #[test]
    fn returns_none_when_no_unified_utilization_headers() {
        let map = HashMap::from([("content-type", "application/json")]);
        assert!(Budget::from_headers(lookup_from(map)).is_none());
    }

    #[test]
    fn tolerates_partial_headers() {
        // Only the 5-hour window present; 7-day fields default, rep/status empty.
        let map = HashMap::from([(H_5H_UTIL, "0.5"), (H_5H_RESET, "1750000000")]);
        let budget = Budget::from_headers(lookup_from(map)).expect("budget present");
        assert_eq!(budget.b5_util, 0.5);
        assert_eq!(budget.b5_reset, 1_750_000_000);
        assert_eq!(budget.b7_util, 0.0);
        assert_eq!(budget.b7_reset, 0);
        assert_eq!(budget.rep, "");
        assert_eq!(budget.status, "");
    }

    /// A minimal **Budget** with the given representative claim, status and 5h
    /// utilization — the rest defaulted — for the presentation-seam tests.
    fn budget_with(rep: &str, status: &str, b5_util: f64) -> Budget {
        Budget {
            b5_util,
            rep: rep.to_string(),
            status: status.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn representative_selects_window_from_claim() {
        assert_eq!(budget_with("five_hour", "allowed", 0.0).representative(), Window::FiveHour);
        assert_eq!(budget_with("seven_day", "allowed", 0.0).representative(), Window::SevenDay);
        // Unknown / empty claim defaults to the 5-hour window.
        assert_eq!(budget_with("", "allowed", 0.0).representative(), Window::FiveHour);
        assert_eq!(budget_with("weird", "allowed", 0.0).representative(), Window::FiveHour);
    }

    #[test]
    fn util_at_zeroes_a_window_once_its_reset_has_passed() {
        let b = Budget {
            b5_util: 0.8,
            b5_reset: 1_000,
            b7_util: 0.3,
            b7_reset: 2_000,
            ..Default::default()
        };
        // Before the reset: the stored fraction stands.
        assert_eq!(b.util_at(Window::FiveHour, 999), 0.8);
        // At and after the reset: zeroed locally, no fresh reading needed.
        assert_eq!(b.util_at(Window::FiveHour, 1_000), 0.0);
        assert_eq!(b.util_at(Window::FiveHour, 1_500), 0.0);
        // Independent per window: 7d still stands while its later reset is future.
        assert_eq!(b.util_at(Window::SevenDay, 1_500), 0.3);
        // A missing reset (0) has no boundary to cross ⇒ left as-is.
        let no_reset = Budget { b5_util: 0.5, b5_reset: 0, ..Default::default() };
        assert_eq!(no_reset.util_at(Window::FiveHour, 9_999), 0.5);
    }

    #[test]
    fn severity_is_driven_by_status_and_utilization() {
        // Allowed + low util ⇒ Ok.
        assert_eq!(budget_with("five_hour", "allowed", 0.1).severity(0.1), Severity::Ok);
        // Allowed + filling ⇒ Warning.
        assert_eq!(budget_with("five_hour", "allowed", 0.7).severity(0.7), Severity::Warning);
        // Allowed + near full ⇒ Critical.
        assert_eq!(budget_with("five_hour", "allowed", 0.95).severity(0.95), Severity::Critical);
        // Rejected account ⇒ Critical even at low util.
        assert_eq!(budget_with("five_hour", "rejected", 0.1).severity(0.1), Severity::Critical);
    }

    #[test]
    fn severity_is_critical_when_overage_disabled() {
        let mut b = budget_with("five_hour", "allowed", 0.1);
        b.overage_status = "disabled".to_string();
        b.overage_disabled_reason = "insufficient_credit".to_string();
        assert_eq!(b.severity(0.1), Severity::Critical);
    }

    #[test]
    fn overage_hidden_when_no_overage_headers() {
        assert_eq!(budget_with("five_hour", "allowed", 0.1).overage(), None);
    }

    #[test]
    fn overage_shows_active_with_fallback() {
        let mut b = budget_with("five_hour", "allowed", 0.1);
        b.overage_status = "allowed".to_string();
        b.fallback_percentage = Some(0.8);
        let overage = b.overage().expect("overage present");
        assert_eq!(overage.label, "overage allowed · fallback 80%");
        assert_eq!(overage.severity, Severity::Warning);
    }

    #[test]
    fn overage_shows_disabled_with_reason() {
        let mut b = budget_with("five_hour", "allowed", 0.1);
        b.overage_status = "disabled".to_string();
        b.overage_disabled_reason = "insufficient_credit".to_string();
        let overage = b.overage().expect("overage present");
        assert_eq!(overage.label, "overage disabled (insufficient_credit)");
        assert_eq!(overage.severity, Severity::Critical);
    }
}
