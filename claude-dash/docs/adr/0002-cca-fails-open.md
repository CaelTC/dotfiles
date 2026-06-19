# cca fails open — the dashboard never blocks claude

Routing `claude` through the capture **Proxy** via `ANTHROPIC_BASE_URL` puts the Proxy in the critical path of real work: if it can't start or crashes, `claude` requests would fail. We refuse to let an observability tool cost a coding session.

**Decision:** Capture is best-effort and subordinate to `claude` always working. `cca` health-checks its Proxy before launch; if the Proxy can't bind/start, `cca` runs `claude` **directly** (no capture for that run) rather than failing. The Proxy is kept trivially simple (read response headers, stream the body through untouched) so there is almost nothing to fail mid-session. A missing dashboard row is acceptable; a blocked `claude` session is not.

## Consequences

- **Gaps are normal.** A session launched while the Proxy was unavailable simply won't appear in the active box or **Session History**. The dashboard is a view of captured sessions, not a guarantee of all sessions.
- **No mid-session supervision in v1.** If the Proxy dies mid-session, the in-flight request fails (the SDK retries) and subsequent requests for that session go uncaptured; we accept this rather than add a watchdog/restart loop to the wrapper. Revisit only if crashes prove common.
- `cca` stays thin but gains a small health-check + direct-launch fallback path — the one piece of real logic in the wrapper.
