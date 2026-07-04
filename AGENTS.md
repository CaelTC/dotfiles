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
- `skiff setup <host> --user <user> [--nick <nick>]` — persist a `Host <nick>`
  block (HostName = resolved tailscale IP, User = `<user>`) into
  `~/.ssh/config`, so `ssh <nick>` connects as the right user. Idempotent: the
  block is wrapped in `# >>> skiff <nick>` / `# <<< skiff <nick>` markers and
  replaced in place on re-run. `--nick` defaults to `<host>`'s first DNS
  label.

Names resolve via `tailscale status --json` to tailscale IPs, so MagicDNS in
the system resolver is not required. Each machine needs `install.sh` run on it
— works on macOS (brew) and Linux (apt/dnf/pacman + official tailscale
installer); installs tmux + tailscaled and enables Tailscale SSH. Inbound ssh
shells auto-attach to a persistent `main` tmux session via `zsh/.zshrc` on
macOS and `bash/.bashrc` on Linux (bash is the default login shell there).
