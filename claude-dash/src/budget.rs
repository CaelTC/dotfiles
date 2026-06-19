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
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
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
        })
    }
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
            }
        );
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
}
