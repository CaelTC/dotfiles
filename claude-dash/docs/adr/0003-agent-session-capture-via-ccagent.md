# Agent sessions are captured too, via `ccagent`, and tagged by Origin

**Budget** (**Utilization** %) is only as fresh as the last captured request. Human sessions flow through the capture **Proxy** via `cca`, but firstmate's unattended background agents ran `claude` directly — so their usage never refreshed the account-wide reading, letting the dashboard drift stale precisely when agents were burning the subscription.

**Decision:** Route agent sessions through the same capture lifecycle with a sibling wrapper, `ccagent`, and tag each **Session**'s **Origin** (Human vs Agent) on its `start` record. `ccagent` differs from `cca` in exactly two ways: it tags Origin=Agent, and it passes the caller's args straight through instead of forcing `--permission-mode auto`, so an agent can pass `--dangerously-skip-permissions` and run unattended. The shared lifecycle (pick a port, health-check the **Proxy**, `record-start`/`record-end`, fail-open) lives in one sourced helper, `bin/lib/cca-capture.sh`; `cca` and `ccagent` are thin callers parameterized by (Origin, permission behavior). The TUI splits the active set by Origin into a **Live View** (Human) and an **Agents View** (Agent).

The wrapper is named `ccagent`, not `cc`: `cc` is the POSIX name of the system C compiler (`/usr/bin/cc`), and the installer symlinks the wrapper into `~/.local/bin`, which precedes `/usr/bin` on PATH — so a `~/.local/bin/cc` symlink would shadow the real `cc` and break `cargo build` and native compilation machine-wide.

## Consequences

- **Budget stays fresh during agent work.** Every `ccagent` request refreshes the account-wide reading exactly as a `cca` request does.
- **Backward compatible.** `Origin` defaults to Human via `#[serde(default)]`, so `start` records written before it existed still parse. `cca`'s external behavior is unchanged (still Human, still `--permission-mode auto`).
- **Fail-open is preserved for agents.** `ccagent` inherits ADR-0002: if the **Proxy** can't come up, it runs `claude` directly (uncaptured) rather than blocking the agent — the direct-launch path passes args through with no forced mode, same as the captured path.
- **One lifecycle, two callers.** The capture logic is no longer duplicated; a change to the lifecycle applies to both wrappers at once.
