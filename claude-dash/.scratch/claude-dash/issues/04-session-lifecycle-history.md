# Session lifecycle + History

Status: ready-for-agent

## What to build

Give sessions a lifecycle and add the **Session History** view. `cca` writes an `end` record when `claude` exits. `claude-dash` classifies each session:

- **Active** — has a `start`, no `end`, and its `pid` is alive
- **History** — has an `end`, **or** its pid is dead (covers `cca` being killed without writing `end`)

Ended sessions move out of the active box into **Session History**, showing the **last 10**: `project · model · total tokens · duration · ended (relative)`. History is durable across `claude-dash` restarts because it is reconstructed from the on-disk store.

## Acceptance criteria

- [ ] `end` record written on normal `claude` exit; the session moves from active box to History
- [ ] A session whose process died without an `end` is detected via pid-liveness and moved to History
- [ ] Session History shows the last 10 ended sessions with project, model, total tokens, duration, and relative end time
- [ ] History survives a `claude-dash` restart (rebuilt from the store)

## Blocked by

- 03-cca-wrapper-multi-session
