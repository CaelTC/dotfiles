//! Records the **Proxy** appends to the shared store, and `claude-dash` reads.
//!
//! The store is append-only JSONL, one file per **Session**, at
//! `~/.cca/sessions/<id>.jsonl`. Records are tagged with a record-type field
//! `"t"` so record types (`start`/`req`/`end`) coexist in the same file. The
//! `start` record is written by `cca` when it launches a **Session**, the `req`
//! record is appended by the **Proxy** per `/v1/messages` response, and the
//! `end` record is written by `cca` when `claude` exits — its `ts` is the
//! **Session**'s end time, used to move it from the active box to **Session
//! History**.

use serde::{Deserialize, Serialize};

use crate::budget::Budget;
use crate::throughput::Throughput;

/// A **Session**'s **Origin**: who launched it. `Human` sessions come from `cca`
/// (an interactive human at the keyboard); `Agent` sessions come from `cc`
/// (firstmate's unattended background agents). Both flow through the same capture
/// **Proxy** so agent usage keeps the account-wide **Budget** fresh; the TUI
/// tells them apart to headline human vs agent activity separately.
///
/// `Human` is the default so a `start` record written before Origin existed (it
/// carries no `origin` field) deserializes as `Human` — old sessions were all
/// human. Serialized snake_case (`"human"` / `"agent"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// A human's interactive session, launched by `cca`.
    #[default]
    Human,
    /// An unattended agent session, launched by `cc`.
    Agent,
}

/// A tagged store record. The `t` field discriminates the variant so additional
/// record types can be appended to the same JSONL file in later slices without
/// breaking readers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t")]
pub enum Record {
    /// The opening record of a **Session**, written by `cca` the moment it
    /// launches `claude` behind a **Proxy**. Carries the **Session**'s identity
    /// (`project`, `cwd`, `pid`) so the TUI can label its panel `project · model ·
    /// id` and a later slice can judge liveness from `pid`.
    #[serde(rename = "start")]
    Start(StartRecord),
    /// One `/v1/messages` response observed by the **Proxy**, carrying the
    /// account-wide **Budget** reading captured from the response headers.
    #[serde(rename = "req")]
    Req(ReqRecord),
    /// The closing record of a **Session**, written by `cca` when `claude`
    /// exits. Its `ts` is the **Session**'s end time — the signal that moves the
    /// **Session** out of the active box and into **Session History**.
    #[serde(rename = "end")]
    End(EndRecord),
}

impl Record {
    /// Borrow the inner [`ReqRecord`] if this is a `req` record. Lets readers
    /// filter for `req`s while staying forward-compatible with the `start` (and
    /// later `end`) record types.
    pub fn as_req(&self) -> Option<&ReqRecord> {
        match self {
            Record::Req(req) => Some(req),
            Record::Start(_) | Record::End(_) => None,
        }
    }

    /// Borrow the inner [`StartRecord`] if this is a `start` record. Lets readers
    /// pull a **Session**'s `project`/`cwd`/`pid` without matching the variant by
    /// hand.
    pub fn as_start(&self) -> Option<&StartRecord> {
        match self {
            Record::Start(start) => Some(start),
            Record::Req(_) | Record::End(_) => None,
        }
    }

    /// Borrow the inner [`EndRecord`] if this is an `end` record. Lets the
    /// lifecycle classifier see that a **Session** ended (so it belongs in
    /// **Session History**) without matching the variant by hand.
    pub fn as_end(&self) -> Option<&EndRecord> {
        match self {
            Record::End(end) => Some(end),
            Record::Start(_) | Record::Req(_) => None,
        }
    }
}

/// A `start` record: `cca` writes one per **Session** at launch, naming the
/// **Session**'s identity. The `id` is the minted **Session** id (also the JSONL
/// file stem), `project` is the cwd basename shown in the panel label, `cwd` is
/// the absolute working directory, and `pid` is the launching process the TUI
/// will treat as the **Session**'s liveness handle in a later slice.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StartRecord {
    /// The minted **Session** id (matches the JSONL file stem `<id>.jsonl`).
    pub id: String,
    /// Launch time, epoch milliseconds.
    pub ts: i64,
    /// The **Session**'s project — the cwd basename — shown in the panel label.
    pub project: String,
    /// The absolute working directory `cca` was launched from.
    pub cwd: String,
    /// The launching process id (the **Session**'s liveness handle for slice 04).
    pub pid: i32,
    /// The **Session**'s **Origin** — `Human` (from `cca`) or `Agent` (from `cc`).
    /// `#[serde(default)]` makes older `start` records (written before Origin
    /// existed, so with no `origin` field) deserialize as `Human`.
    #[serde(default)]
    pub origin: Origin,
}

/// An `end` record: `cca` writes one per **Session** when `claude` exits. It is
/// deliberately minimal — the `id` ties it to the **Session** and `ts` is the
/// **Session**'s end time, the key field the lifecycle classifier reads to move
/// the **Session** into **Session History** and to compute its duration and
/// "ended Xm ago" label.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EndRecord {
    /// The **Session** id (matches the JSONL file stem `<id>.jsonl`).
    pub id: String,
    /// End time, epoch milliseconds — when `claude` exited.
    pub ts: i64,
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
            ..Default::default()
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

    fn sample_start() -> StartRecord {
        StartRecord {
            id: "a1b2c3d4".to_string(),
            ts: 1_750_000_000_000,
            project: "claude-dash".to_string(),
            cwd: "/Users/cael/dotfiles/claude-dash".to_string(),
            pid: 4242,
            origin: Origin::Human,
        }
    }

    #[test]
    fn start_record_serializes_with_type_tag_and_fields() {
        let rec = Record::Start(sample_start());
        let json = serde_json::to_string(&rec).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(value["t"], "start");
        assert_eq!(value["id"], "a1b2c3d4");
        assert_eq!(value["ts"], 1_750_000_000_000_i64);
        assert_eq!(value["project"], "claude-dash");
        assert_eq!(value["cwd"], "/Users/cael/dotfiles/claude-dash");
        assert_eq!(value["pid"], 4242);
    }

    #[test]
    fn start_record_round_trips() {
        let original = Record::Start(sample_start());
        let json = serde_json::to_string(&original).unwrap();
        let parsed: Record = serde_json::from_str(&json).unwrap();
        assert_eq!(original, parsed);
        assert_eq!(parsed.as_start(), Some(&sample_start()));
    }

    #[test]
    fn start_record_origin_backward_compat_and_agent_round_trip() {
        // Backward compatibility (MANDATORY): a `start` record written before
        // Origin existed carries no `origin` field, and must deserialize as Human.
        let old_json = r#"{"t":"start","id":"a1b2c3d4","ts":1750000000000,"project":"claude-dash","cwd":"/w","pid":4242}"#;
        let parsed: Record = serde_json::from_str(old_json).unwrap();
        assert_eq!(parsed.as_start().unwrap().origin, Origin::Human);

        // A new Agent start record round-trips, serializing origin snake_case.
        let mut agent = sample_start();
        agent.origin = Origin::Agent;
        let json = serde_json::to_string(&Record::Start(agent.clone())).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["origin"], "agent");
        let parsed: Record = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.as_start(), Some(&agent));
    }

    #[test]
    fn as_req_is_none_for_a_start_record() {
        let rec = Record::Start(sample_start());
        assert!(rec.as_req().is_none());
    }

    fn sample_end() -> EndRecord {
        EndRecord {
            id: "a1b2c3d4".to_string(),
            ts: 1_750_000_900_000,
        }
    }

    #[test]
    fn end_record_serializes_with_type_tag_and_fields() {
        let rec = Record::End(sample_end());
        let json = serde_json::to_string(&rec).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(value["t"], "end");
        assert_eq!(value["id"], "a1b2c3d4");
        assert_eq!(value["ts"], 1_750_000_900_000_i64);
    }

    #[test]
    fn end_record_round_trips() {
        let original = Record::End(sample_end());
        let json = serde_json::to_string(&original).unwrap();
        let parsed: Record = serde_json::from_str(&json).unwrap();
        assert_eq!(original, parsed);
        assert_eq!(parsed.as_end(), Some(&sample_end()));
    }

    #[test]
    fn as_req_and_as_start_are_none_for_an_end_record() {
        let rec = Record::End(sample_end());
        assert!(rec.as_req().is_none());
        assert!(rec.as_start().is_none());
    }
}
