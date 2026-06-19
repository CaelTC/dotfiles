//! **Throughput** — the per-**Session** facet of **Usage**: tokens (input /
//! output / cache-read / cache-creation) per **Model**.
//!
//! Captured live by the **Proxy** from a tee of the `/v1/messages` response
//! body. Anthropic streams that response as SSE: the `message_start` event
//! carries `message.usage` (input / cache-creation / cache-read tokens) and the
//! `message.model`, while the final `message_delta` event carries the
//! `usage.output_tokens`. This module parses those out of the (possibly
//! chunk-split) byte stream and exposes a [`Throughput`] reading.
//!
//! **Throughput** is exact and per-**Session** — kept a distinct type from
//! **Budget** (account-wide, authoritative) so the two facets of **Usage** are
//! never conflated. It is also shown as a *rolling-window rate* (tokens completed
//! in the last ~60s), computed by the pure [`rolling_rate`] function so the TUI's
//! rate math is testable without a terminal or filesystem.

use serde::{Deserialize, Serialize};

/// A **Throughput** reading: the token counts and **Model** captured from one
/// `/v1/messages` response body. All token fields are counts for that single
/// request.
///
/// Kept distinct from [`crate::budget::Budget`] — **Throughput** is the exact,
/// per-**Session** facet of **Usage**, never conflated with the account-wide
/// **Budget**.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Throughput {
    /// Input tokens (`usage.input_tokens` from `message_start`).
    #[serde(rename = "in")]
    pub input: u64,
    /// Output tokens (`usage.output_tokens`, finalised in `message_delta`).
    #[serde(rename = "out")]
    pub output: u64,
    /// Cache-read input tokens (`usage.cache_read_input_tokens`).
    #[serde(rename = "cache_r")]
    pub cache_read: u64,
    /// Cache-creation input tokens (`usage.cache_creation_input_tokens`).
    #[serde(rename = "cache_w")]
    pub cache_write: u64,
    /// The **Model** that served the request (e.g. `claude-opus-4-8`).
    pub model: String,
}

impl Throughput {
    /// Total tokens for this request — the sum the rolling-window rate meters.
    pub fn total(&self) -> u64 {
        self.input + self.output + self.cache_read + self.cache_write
    }
}

/// Parse a [`Throughput`] reading from a complete `/v1/messages` response body.
///
/// Tolerant by design: works on the SSE stream (`event: message_start` /
/// `event: message_delta` lines, each `data: { … }`) *and* on a single
/// non-streaming JSON body. Missing fields default to `0` / empty rather than
/// panicking, and a body carrying no `usage` at all yields `None` so a request
/// with no token data simply records no **Throughput**.
///
/// Pure (bytes in, reading out) so it is unit-testable without the **Proxy**.
pub fn parse_usage(body: &[u8]) -> Option<Throughput> {
    let text = String::from_utf8_lossy(body);

    let mut model = String::new();
    let mut input = 0u64;
    let mut cache_read = 0u64;
    let mut cache_write = 0u64;
    let mut output = 0u64;
    let mut saw_usage = false;

    // Walk every JSON object we can find in the body. For SSE that's the
    // payload after each `data:` prefix; for a single JSON body it's the whole
    // thing. We merge across objects because `message_start` carries the input
    // side and `message_delta` carries the final output_tokens.
    for json in json_payloads(&text) {
        // Only `message_start` / `message_delta` carry `usage`; the bulk of an
        // SSE response is `content_block_delta` text with none. A substring scan
        // skips those before the (allocating) JSON parse.
        if !json.contains("usage") {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
            continue;
        };

        // `message_start` nests the model + usage under `message`; a plain
        // non-streaming body has `model` + `usage` at the top level. Accept
        // either shape.
        let model_node = value
            .get("message")
            .and_then(|m| m.get("model"))
            .or_else(|| value.get("model"));
        if let Some(m) = model_node.and_then(|m| m.as_str()) {
            if !m.is_empty() {
                model = m.to_string();
            }
        }

        let usage = value
            .get("message")
            .and_then(|m| m.get("usage"))
            .or_else(|| value.get("usage"));
        let Some(usage) = usage else {
            continue;
        };
        saw_usage = true;

        // Take the max seen for each field: `message_start` reports the input
        // side and an initial output_tokens, `message_delta` reports the final
        // (larger) output_tokens. Max is robust to either ordering and to a
        // single JSON body carrying all of them at once.
        input = input.max(usage_u64(usage, "input_tokens"));
        cache_read = cache_read.max(usage_u64(usage, "cache_read_input_tokens"));
        cache_write = cache_write.max(usage_u64(usage, "cache_creation_input_tokens"));
        output = output.max(usage_u64(usage, "output_tokens"));
    }

    if !saw_usage {
        return None;
    }

    Some(Throughput {
        input,
        output,
        cache_read,
        cache_write,
        model,
    })
}

/// Read an unsigned token count from a `usage` object, defaulting to `0` when
/// the field is absent or null (Anthropic omits cache fields when unused).
fn usage_u64(usage: &serde_json::Value, field: &str) -> u64 {
    usage.get(field).and_then(|v| v.as_u64()).unwrap_or(0)
}

/// Yield each JSON payload embedded in a response body.
///
/// For SSE that is the text after each `data:` line prefix (Anthropic emits one
/// `data:` line per event); a chunk-split stream is fine because we parse the
/// reassembled body. For a single non-streaming JSON body — which has no
/// `data:` lines — the whole trimmed body is yielded once.
fn json_payloads(text: &str) -> Vec<&str> {
    let mut payloads: Vec<&str> = text
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(|rest| rest.trim())
        .filter(|p| !p.is_empty() && *p != "[DONE]")
        .collect();

    // No SSE `data:` lines ⇒ treat the body as one JSON document.
    if payloads.is_empty() {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            payloads.push(trimmed);
        }
    }
    payloads
}

/// The result of [`rolling_rate`]: a smoothed tokens-per-minute rate plus the
/// per-bucket token sums that drive the braille sparkline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RollingRate {
    /// Smoothed **Throughput** rate over the window, in tokens per minute.
    pub tokens_per_min: u64,
    /// Per-bucket token sums across the window, oldest first — the sparkline
    /// samples. Always [`WINDOW_BUCKETS`] long.
    pub buckets: Vec<u64>,
}

/// Length of the rolling window, in seconds (~60s, per the domain definition of
/// **Throughput** as "tokens completed in the last ~60s").
pub const WINDOW_SECS: i64 = 60;
/// Number of buckets the window is divided into for the sparkline (one per ~6s).
pub const WINDOW_BUCKETS: usize = 10;

/// Compute the rolling-window **Throughput** rate from `(ts_ms, tokens)` samples
/// and the current time `now_ms`, both in epoch **milliseconds**.
///
/// **Smoothing choice:** we sum *all* tokens from requests whose timestamp falls
/// in the last [`WINDOW_SECS`] seconds and normalise that sum to a per-minute
/// figure (`sum * 60 / WINDOW_SECS`). Because the rate is the *window total*
/// rather than an instantaneous per-request value, a silent gap between turns
/// only decays the rate gradually as old requests age out of the window — it
/// never snaps to zero the instant a request finishes, and a burst of requests
/// reads as one smooth elevated rate rather than a spike. Records older than the
/// window are excluded entirely.
///
/// The sparkline buckets divide the window into [`WINDOW_BUCKETS`] equal slices
/// (oldest first), each holding the token sum of requests that completed in that
/// slice, so the bar heights show the shape of recent activity.
///
/// Pure over its inputs so it is unit-testable without the TUI or filesystem.
pub fn rolling_rate<I>(samples: I, now_ms: i64) -> RollingRate
where
    I: IntoIterator<Item = (i64, u64)>,
{
    let window_ms = WINDOW_SECS * 1_000;
    let window_start = now_ms - window_ms;
    let bucket_ms = window_ms / WINDOW_BUCKETS as i64;

    let mut buckets = vec![0u64; WINDOW_BUCKETS];
    let mut sum = 0u64;

    for (ts_ms, tokens) in samples {
        // Exclude anything outside the window (older than ~60s, or future).
        if ts_ms < window_start || ts_ms > now_ms {
            continue;
        }
        sum += tokens;

        // Place the sample in its time bucket, oldest first. Clamp the index so
        // a sample exactly at `now` lands in the last bucket.
        let offset = ts_ms - window_start;
        let idx = (offset / bucket_ms).clamp(0, WINDOW_BUCKETS as i64 - 1) as usize;
        buckets[idx] += tokens;
    }

    // Normalise the window total to a per-minute rate.
    let tokens_per_min = (sum as i64 * 60 / WINDOW_SECS) as u64;

    RollingRate {
        tokens_per_min,
        buckets,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MESSAGE_START: &str = "event: message_start\n\
data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"model\":\"claude-opus-4-8\",\"usage\":{\"input_tokens\":120,\"cache_creation_input_tokens\":30,\"cache_read_input_tokens\":2000,\"output_tokens\":1}}}\n\n";

    const MESSAGE_DELTA: &str = "event: message_delta\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":456}}\n\n";

    #[test]
    fn parses_usage_and_model_from_sse_stream() {
        let body = format!("{MESSAGE_START}{MESSAGE_DELTA}");
        let tp = parse_usage(body.as_bytes()).expect("usage present");
        assert_eq!(tp.input, 120);
        assert_eq!(tp.cache_write, 30);
        assert_eq!(tp.cache_read, 2000);
        assert_eq!(tp.output, 456);
        assert_eq!(tp.model, "claude-opus-4-8");
    }

    #[test]
    fn parses_usage_when_chunk_split_across_the_event_boundary() {
        // Simulate the tee reassembling two network chunks: the split lands in
        // the middle of the stream, but we parse the reassembled body.
        let full = format!("{MESSAGE_START}{MESSAGE_DELTA}");
        let (a, b) = full.split_at(full.len() / 2);
        let mut reassembled = Vec::new();
        reassembled.extend_from_slice(a.as_bytes());
        reassembled.extend_from_slice(b.as_bytes());

        let tp = parse_usage(&reassembled).expect("usage present");
        assert_eq!(tp.input, 120);
        assert_eq!(tp.output, 456);
        assert_eq!(tp.model, "claude-opus-4-8");
    }

    #[test]
    fn parses_usage_from_non_streaming_json_body() {
        let body = "{\"model\":\"claude-sonnet-4-5\",\"usage\":{\"input_tokens\":10,\"output_tokens\":20}}";
        let tp = parse_usage(body.as_bytes()).expect("usage present");
        assert_eq!(tp.input, 10);
        assert_eq!(tp.output, 20);
        assert_eq!(tp.cache_read, 0);
        assert_eq!(tp.cache_write, 0);
        assert_eq!(tp.model, "claude-sonnet-4-5");
    }

    #[test]
    fn no_usage_in_body_yields_none() {
        let body = "event: ping\ndata: {\"type\":\"ping\"}\n\n";
        assert!(parse_usage(body.as_bytes()).is_none());
    }

    #[test]
    fn rolling_rate_smooths_a_burst_within_the_window() {
        // Three requests in a quick burst near the start of the window, then
        // silence. The rate reflects the window total, not zero.
        let now = 100_000i64;
        let samples = vec![
            (now - 50_000, 1_000u64),
            (now - 49_000, 1_000),
            (now - 48_000, 1_000),
        ];
        let rate = rolling_rate(samples, now);
        // 3000 tokens over a 60s window → 3000 tokens/min.
        assert_eq!(rate.tokens_per_min, 3_000);
    }

    #[test]
    fn rolling_rate_gap_within_window_does_not_collapse_to_zero() {
        // A request 30s ago, then a silent gap right up to `now`. The window
        // still contains the request, so the rate stays positive rather than
        // snapping to zero the moment the turn ended.
        let now = 100_000i64;
        let samples = vec![(now - 30_000, 600u64)];
        let rate = rolling_rate(samples, now);
        // 600 tokens over 60s → 600 tokens/min.
        assert_eq!(rate.tokens_per_min, 600);
        assert!(rate.tokens_per_min > 0);
    }

    #[test]
    fn rolling_rate_excludes_records_outside_the_window() {
        let now = 100_000i64;
        let samples = vec![
            (now - 90_000, 5_000u64), // older than 60s → excluded
            (now - 10_000, 600),      // in-window
        ];
        let rate = rolling_rate(samples, now);
        assert_eq!(rate.tokens_per_min, 600);
    }

    #[test]
    fn rolling_rate_buckets_place_samples_oldest_first() {
        let now = 60_000i64; // window_start = 0
        // One sample near the start, one near the end of the window.
        let samples = vec![(1_000, 100u64), (59_000, 200u64)];
        let rate = rolling_rate(samples, now);
        assert_eq!(rate.buckets.len(), WINDOW_BUCKETS);
        assert_eq!(rate.buckets[0], 100); // first ~6s bucket
        assert_eq!(rate.buckets[WINDOW_BUCKETS - 1], 200); // last ~6s bucket
    }

    #[test]
    fn rolling_rate_empty_is_zero() {
        let rate = rolling_rate(Vec::<(i64, u64)>::new(), 100_000);
        assert_eq!(rate.tokens_per_min, 0);
        assert_eq!(rate.buckets, vec![0; WINDOW_BUCKETS]);
    }
}
