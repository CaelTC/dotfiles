# Project agent memory

This file is the project's committed home for project-intrinsic agent knowledge: build, test, release, architecture, and sharp-edge notes that should travel with the code.

- Add durable project-specific notes here as they are discovered through real work.

## Remote machines (skiff)

`skiff/` is a Rust CLI that ferries between tailscale machines. Agents (e.g.
firstmate) can use it to run Claude sessions on other machines:

- `skiff ls` — list machines on the tailnet
- `skiff claude <host> <dir> -- "<prompt>"` — start claude in a detached tmux
  session on that machine (named after the dir; survives disconnect)
- `skiff ssh <host> [session]` — attach to a session interactively
- `skiff sessions <host>` — list tmux sessions on a machine
- `skiff setup <host> [--user <user>] [--nick <nick>]` — interactively prompts
  for the SSH username and nickname (nickname defaults to `<host>`'s first DNS
  label on empty input), shows the resulting block, and confirms before
  writing a `Host <nick>` block (HostName = resolved tailscale IP, User =
  `<user>`) into `~/.ssh/config`, so `ssh <nick>` connects as the right user.
  Idempotent: the block is wrapped in `# >>> skiff <nick>` / `# <<< skiff
  <nick>` markers and replaced in place on re-run. Passing both `--user` and
  `--nick` skips all prompts for non-interactive/scripted use.

All verbs resolve a `<host>` argument through `resolve()`, which checks
`~/.ssh/config` for a matching `Host` entry *before* falling back to the
tailnet lookup — so a `skiff setup`-written nickname (including one that
collides with a real tailscale machine name) is honored verbatim and gets
its configured `User`, rather than being resolved to a bare tailscale IP that
drops the user and connects as the local account. An explicit `user@name`
still overrides the config's `User`, matching ssh's own precedence.

Names resolve via `tailscale status --json` to tailscale IPs, so MagicDNS in
the system resolver is not required. Each machine needs `install.sh` run on it
— works on macOS (brew) and Linux (apt/dnf/pacman + official tailscale
installer); installs tmux + tailscaled and enables Tailscale SSH. Inbound ssh
shells auto-attach to a persistent `main` tmux session via `zsh/.zshrc` on
macOS and `bash/.bashrc` on Linux (bash is the default login shell there).

`ssh()`, `claude()`, and `sessions()` each call `ensure_remote_terminfo(&target)`
right after resolving the target and before running the remote tmux/command.
It best-effort pipes `infocmp -x -- "$TERM"` into `ssh <target> 'tic -x -'` so
an unusual local `TERM` (e.g. Ghostty's `xterm-ghostty`) has a terminfo entry
on the remote — without that, remote `tmux`/`ssh -t` reject the TERM outright.
Every failure (missing `infocmp`/`tic`, dead ssh, non-zero exit, empty `TERM`)
is swallowed; it never blocks or fails the actual connect, and it never forces
`TERM` to a fallback like `xterm-256color` (that would downgrade Ghostty on
remotes that just need the terminfo pushed).
