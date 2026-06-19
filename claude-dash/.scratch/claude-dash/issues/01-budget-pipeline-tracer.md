# Budget pipeline tracer

Status: done

## What to build

The thinnest end-to-end slice that proves capture → store → display for **Budget**. Two pieces:

- **`claude-dash proxy`** — a streaming reverse-proxy. The client points `ANTHROPIC_BASE_URL` at it; it forwards every request to `https://api.anthropic.com`, relays the response **body untouched and streamed** (so the client TUI stays live — do not buffer the whole response), and reads the rate-limit headers off each `/v1/messages` response. For each response it appends a `req` record to a per-session file `~/.cca/sessions/<id>.jsonl` (the proxy self-generates an `id` if none is passed). Token refresh and non-inference hosts are not redirected — only the inference base is.
- **`claude-dash`** (TUI) — globs `~/.cca/sessions/*.jsonl`, file-watches the directory (`notify`/FSEvents), and renders the **Budget left rail**: the 5-hour and 7-day windows with utilization as a percentage and a live countdown to reset. Budget is the *newest* `req` across all files (account-wide, so the freshest reading wins).

Decision-rich detail (confirmed by spike, see `docs/adr/0001`): the source headers are `anthropic-ratelimit-unified-5h-utilization` / `-5h-reset` / `-7d-utilization` / `-7d-reset` / `-representative-claim` / `-status` (utilization is a 0–1 fraction; reset is epoch seconds). The `req` record carries those as `b5_util`, `b5_reset`, `b7_util`, `b7_reset`, `rep`, `status` (token fields are added in slice 02).

## Acceptance criteria

- [ ] Proxy forwards all methods/paths to `api.anthropic.com` and returns responses unchanged (status, headers, body), streaming the body without full buffering
- [ ] A real `claude` request routed via `ANTHROPIC_BASE_URL` succeeds end-to-end with no added failure or latency
- [ ] Each `/v1/messages` response appends one `req` record containing `b5_util`, `b5_reset`, `b7_util`, `b7_reset`, `rep`, `status`
- [ ] TUI renders 5h and 7d gauges with % utilization and a countdown to reset, updating within ~1s of a new record (file-watch + tick)
- [ ] Budget reflects the newest `req` across all session files

## Blocked by

None - can start immediately
