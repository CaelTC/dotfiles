# cca fail-open

Status: done

## What to build

Realize ADR-0002: **the dashboard must never block `claude`.** Before running `claude`, `cca` health-checks that its proxy actually started and is reachable on its port. If the proxy is healthy, traffic is captured as normal. If it failed to start/bind, `cca` launches `claude` **directly** — without `ANTHROPIC_BASE_URL`, so no capture for that run — rather than failing. Capture is best-effort; an un-captured session is acceptable, a blocked session is not.

## Acceptance criteria

- [ ] When the proxy starts cleanly, the session is captured as normal
- [ ] When the proxy fails to start/bind, `cca` still launches a working `claude` directly (claude runs; the session simply does not appear in the dashboard)
- [ ] There is no scenario in which a proxy problem prevents `claude` from running

## Blocked by

- 03-cca-wrapper-multi-session
