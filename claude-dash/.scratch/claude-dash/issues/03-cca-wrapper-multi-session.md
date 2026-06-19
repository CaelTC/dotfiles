# cca wrapper + multiple concurrent sessions

Status: done

## What to build

Introduce **`cca`**, the thin zsh wrapper invoked in place of `claude` (evolved from the `claude --permission auto` alias), and support multiple concurrent sessions. Per invocation `cca`:

1. mints a **session id**,
2. starts a per-session proxy on a free port and writes a `start` record (`id`, `ts`, `project` = cwd basename, `cwd`, `pid`),
3. exports `ANTHROPIC_BASE_URL` to that proxy,
4. runs `claude --permission auto "$@"` (existing behavior, untouched).

Each session writes its own `~/.cca/sessions/<id>.jsonl`. The TUI renders **N concurrent Active Session panels** labelled `project · model · id`; Budget takes the freshest reading across all of them.

## Acceptance criteria

- [ ] `cca` launches `claude` through a per-session proxy with a minted id and writes a `start` record (`id`, `ts`, `project`, `cwd`, `pid`)
- [ ] Running `cca` in two terminals shows two distinct Active Session panels labelled `project · model · id`
- [ ] Budget gauge uses the newest `req` across all sessions
- [ ] Each session's throughput is tracked independently and correctly
- [ ] `cca` passes through arbitrary `claude` args and preserves `--permission auto`

## Blocked by

- 02-per-session-throughput
