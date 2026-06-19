# Capture subscription usage via a local proxy, not transcripts or the hidden usage API

We want a live, authoritative view of how much of the Claude **Max 5x** subscription is consumed, across several concurrent Claude Code instances on this machine, without opening the Anthropic website. There is no *supported* local API for subscription budget: rate-limit data is returned only as HTTP response headers on requests the Claude Code process makes, and the endpoint its `/usage` view calls is undocumented and deliberately obfuscated.

**Decision:** `cca` (a thin zsh wrapper, evolved from the `claude --permission auto` alias) runs each `claude` **Instance** behind a per-instance local logging **Proxy** by pointing `ANTHROPIC_BASE_URL` at it. The Proxy records, per request, Anthropic's rate-limit response **headers** (account-wide **Budget**) and the response-body `usage` blocks (per-Instance **Throughput**), tagged with an instance id minted by `cca`, into a shared store. `claude-dash` is a read-only TUI that tails the store and renders one aggregate Budget gauge plus per-Instance Throughput. We observe our own client's standard response metadata on our own machine — no token handling, no hidden endpoint.

## Considered options

- **Transcripts only** (`~/.claude/projects/**/*.jsonl`): fully supported and durable, but contains no rate-limit data — Budget would be a guess of token-throughput against an unpublished ceiling, and this-machine only. Rejected: doesn't deliver real budget, so it wouldn't actually replace the website.
- **The undocumented OAuth usage endpoint** (what Claude Code's `/usage` calls): real account numbers, but the endpoint is dynamically constructed and partly obfuscated, requires reusing/refreshing the OAuth token, breaks on any Claude Code release, and is ToS-grey. Rejected: maximally brittle for the value.
- **Local proxy capture** (chosen): real numbers from standard rate-limit response headers on traffic Claude Code already makes; no reverse-engineering, no token custody; per-instance identity falls out of the per-instance proxy; `ANTHROPIC_BASE_URL` / `HTTPS_PROXY` / `NODE_EXTRA_CA_CERTS` are all honored by the Claude Code binary.

## Consequences

- Capture happens **only for instances launched via `cca`**; bare `claude` is invisible. Mitigated by making `cca` the normal launch path.
- **Budget freshness equals the last request** any instance made; idle instances yield a stale-but-labeled reading (and idle means nothing is being consumed anyway).
- The shared store is **append-only JSONL, one file per instance** (chosen over SQLite): daemonless, crash-safe, and contention-free across concurrent writers, at the cost of small-file cleanup. `claude-dash` stays a pure reader, file-watched for liveness.
- **Confirmed by spike (2026-06-19):** a `claude -p` request routed through a reverse proxy on `ANTHROPIC_BASE_URL` returned `anthropic-ratelimit-unified-{5h,7d}-utilization` (0–1 fractions), `-reset` (epoch), `-status`, and `-representative-claim` on the `POST /v1/messages` response, scoped to the organization. Budget is therefore **authoritative**, not an estimate — utilization is given directly, so no Ceiling/denominator guess is needed. The token refresh happened out-of-band (not through the proxy), confirming `ANTHROPIC_BASE_URL` redirects only inference.
