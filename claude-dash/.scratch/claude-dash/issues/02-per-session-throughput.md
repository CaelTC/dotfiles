# Per-session throughput

Status: done

## What to build

Capture and display **Throughput** for a session. The proxy, while relaying the response body, tees a copy and parses the final `usage` block out of the SSE stream (`message_start` / `message_delta`) **without delaying the stream to the client**. Token counts are added to each `req` record: `in`, `out`, `cache_r`, `cache_w`, plus `model`.

The TUI gains a single **Active Session** panel showing the session's model and a **rolling-window throughput rate** — tokens from requests completing in the last ~60s, shown as tokens/min plus a braille sparkline. The rate is windowed (not instantaneous) so bursty per-request data reads smoothly rather than spiking to zero between turns.

## Acceptance criteria

- [ ] Proxy extracts `usage` (input, output, cache_read, cache_creation) from streamed `/v1/messages` responses and writes them + `model` on the `req` record
- [ ] Token-by-token streaming to the client is not delayed by the tee
- [ ] TUI renders an Active Session panel with model and a rolling 60s tokens/min rate + sparkline
- [ ] The rate smooths bursts — silent gaps between requests do not cause abrupt spikes to/from zero

## Blocked by

- 01-budget-pipeline-tracer
