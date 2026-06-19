//! Records the **Proxy** appends to the shared store, and `claude-dash` reads.
//!
//! The store is append-only JSONL, one file per **Session**, at
//! `~/.cca/sessions/<id>.jsonl`. Records are tagged with a record-type field
//! `"t"` so future record types (`start`/`end` in later slices) can coexist in
//! the same file. This slice defines only the `req` record.

use serde::{Deserialize, Serialize};

use crate::budget::Budget;
use crate::throughput::Throughput;

/// A tagged store record. The `t` field discriminates the variant so additional
/// record types can be appended to the same JSONL file in later slices without
/// breaking readers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t")]
pub enum Record {
    /// One `/v1/messages` response observed by the **Proxy**, carrying the
    /// account-wide **Budget** reading captured from the response headers.
    #[serde(rename = "req")]
    Req(ReqRecord),
}

impl Record {
    /// Borrow the inner [`ReqRecord`] if this is a `req` record. Lets readers
    /// filter for `req`s while staying forward-compatible with the `start`/`end`
    /// record types added in later slices.
    pub fn as_req(&self) -> Option<&ReqRecord> {
        match self {
            Record::Req(req) => Some(req),
        }
    }
}

/// A `req` record: a timestamped reading the **Proxy** captured from one
/// `/v1/messages` response. It carries the account-wide **Budget** (from the
/// `anthropic-ratelimit-unified-*` headers) and, when the response body yielded
/// `usage`, the per-**Session** **Throughput**.
///
/// **Budget** is flattened into the record's fields (the on-disk JSONL stays
/// flat via `#[serde(flatten)]`). **Throughput** is a distinct nested type, not
/// a pile of sibling primitives — keeping the two facets of **Usage** from being
/// conflated, exactly as the domain demands. Both are independent: a `req` may
/// carry a **Budget** reading but no `usage` (and in principle the reverse), so
/// **Throughput** is an [`Option`] that *flattens* its `in`/`out`/`cache_*`/
/// `model` fields onto the record when present and is absent entirely when not.
///
/// **Utilization** is stored as the raw 0–1 fraction (the TUI renders it as a
/// percentage).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReqRecord {
    /// Capture time, epoch milliseconds. Used to pick the newest **Budget** and
    /// to window **Throughput** into a rolling rate.
    pub ts: i64,
    /// The captured **Budget** reading, flattened into the record's fields
    /// (`b5_util`, `b5_reset`, `b7_util`, `b7_reset`, `rep`, `status`).
    #[serde(flatten)]
    pub budget: Budget,
    /// The captured **Throughput** reading (tokens + **Model**), when the
    /// response body carried `usage`. Flattened so its `in`/`out`/`cache_r`/
    /// `cache_w`/`model` fields sit alongside the **Budget** fields, and omitted
    /// entirely when absent (`#[serde(default, flatten)]` round-trips `None`).
    #[serde(default, flatten, skip_serializing_if = "Option::is_none")]
    pub throughput: Option<Throughput>,
}

impl ReqRecord {
    /// Build a `req` record from a captured **Budget** reading, a capture
    /// timestamp (epoch milliseconds), and the **Throughput** reading when the
    /// response body carried `usage` (`None` when it did not).
    pub fn from_budget(budget: &Budget, ts: i64, throughput: Option<Throughput>) -> ReqRecord {
        ReqRecord {
            ts,
            budget: budget.clone(),
            throughput,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_budget() -> Budget {
        Budget {
            b5_util: 0.42,
            b5_reset: 1_750_000_000,
            b7_util: 0.10,
            b7_reset: 1_750_500_000,
            rep: "five_hour".to_string(),
            status: "allowed".to_string(),
        }
    }

    fn sample_throughput() -> Throughput {
        Throughput {
            input: 120,
            output: 456,
            cache_read: 2000,
            cache_write: 30,
            model: "claude-opus-4-8".to_string(),
        }
    }

    #[test]
    fn req_record_serializes_with_type_tag_and_fields() {
        let rec = Record::Req(ReqRecord::from_budget(&sample_budget(), 1_750_000_123_000, None));
        let json = serde_json::to_string(&rec).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(value["t"], "req");
        assert_eq!(value["ts"], 1_750_000_123_000_i64);
        assert_eq!(value["b5_util"], 0.42);
        assert_eq!(value["b5_reset"], 1_750_000_000_i64);
        assert_eq!(value["b7_util"], 0.10);
        assert_eq!(value["b7_reset"], 1_750_500_000_i64);
        assert_eq!(value["rep"], "five_hour");
        assert_eq!(value["status"], "allowed");

        // A req with no Throughput omits the token fields entirely.
        assert!(value.get("in").is_none());
        assert!(value.get("out").is_none());
        assert!(value.get("cache_r").is_none());
        assert!(value.get("cache_w").is_none());
        assert!(value.get("model").is_none());
    }

    #[test]
    fn req_record_serializes_throughput_fields_when_present() {
        let rec = Record::Req(ReqRecord::from_budget(
            &sample_budget(),
            1_750_000_123_000,
            Some(sample_throughput()),
        ));
        let json = serde_json::to_string(&rec).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();

        // Budget fields stay flat alongside the new Throughput fields.
        assert_eq!(value["b5_util"], 0.42);
        // Throughput fields are present under their record names.
        assert_eq!(value["in"], 120);
        assert_eq!(value["out"], 456);
        assert_eq!(value["cache_r"], 2000);
        assert_eq!(value["cache_w"], 30);
        assert_eq!(value["model"], "claude-opus-4-8");
    }

    #[test]
    fn req_record_round_trips_without_throughput() {
        let original =
            Record::Req(ReqRecord::from_budget(&sample_budget(), 1_750_000_123_000, None));
        let json = serde_json::to_string(&original).unwrap();
        let parsed: Record = serde_json::from_str(&json).unwrap();
        assert_eq!(original, parsed);
        assert_eq!(parsed.as_req().unwrap().throughput, None);
    }

    #[test]
    fn req_record_round_trips_with_throughput() {
        let original = Record::Req(ReqRecord::from_budget(
            &sample_budget(),
            1_750_000_123_000,
            Some(sample_throughput()),
        ));
        let json = serde_json::to_string(&original).unwrap();
        let parsed: Record = serde_json::from_str(&json).unwrap();
        assert_eq!(original, parsed);
        assert_eq!(parsed.as_req().unwrap().throughput, Some(sample_throughput()));
    }
}
