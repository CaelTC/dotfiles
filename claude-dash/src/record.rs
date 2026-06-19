//! Records the **Proxy** appends to the shared store, and `claude-dash` reads.
//!
//! The store is append-only JSONL, one file per **Session**, at
//! `~/.cca/sessions/<id>.jsonl`. Records are tagged with a record-type field
//! `"t"` so future record types (`start`/`end` in later slices) can coexist in
//! the same file. This slice defines only the `req` record.

use serde::{Deserialize, Serialize};

use crate::budget::Budget;

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

/// A `req` record: a timestamped **Budget** reading the **Proxy** captured from
/// one `/v1/messages` response's `anthropic-ratelimit-unified-*` headers. A `req`
/// *is* a [`Budget`] plus a capture time, so it embeds the reading rather than
/// restating its fields — the on-disk JSONL stays flat via `#[serde(flatten)]`.
///
/// **Utilization** is stored as the raw 0–1 fraction (the TUI renders it as a
/// percentage). Token fields (`in`/`out`/`cache_*`/`model`) are **slice 02** and
/// are deliberately absent here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReqRecord {
    /// Capture time, epoch milliseconds. Used to pick the newest **Budget**.
    pub ts: i64,
    /// The captured **Budget** reading, flattened into the record's fields
    /// (`b5_util`, `b5_reset`, `b7_util`, `b7_reset`, `rep`, `status`).
    #[serde(flatten)]
    pub budget: Budget,
}

impl ReqRecord {
    /// Build a `req` record from a captured **Budget** reading and a capture
    /// timestamp (epoch milliseconds).
    pub fn from_budget(budget: &Budget, ts: i64) -> ReqRecord {
        ReqRecord {
            ts,
            budget: budget.clone(),
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

    #[test]
    fn req_record_serializes_with_type_tag_and_fields() {
        let rec = Record::Req(ReqRecord::from_budget(&sample_budget(), 1_750_000_123_000));
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

        // Token fields are slice 02 — must NOT be present here.
        assert!(value.get("in").is_none());
        assert!(value.get("out").is_none());
        assert!(value.get("model").is_none());
    }

    #[test]
    fn req_record_round_trips() {
        let original = Record::Req(ReqRecord::from_budget(&sample_budget(), 1_750_000_123_000));
        let json = serde_json::to_string(&original).unwrap();
        let parsed: Record = serde_json::from_str(&json).unwrap();
        assert_eq!(original, parsed);
    }
}
