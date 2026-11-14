export PATH="$HOME/.local/bin:$PATH"
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"

# Omarchy (Linux): load its env, prompt (starship), aliases, completions, mise, etc.
if [[ -r /usr/share/omarchy/default/bash/env-bootstrap ]]; then
  source /usr/share/omarchy/default/bash/env-bootstrap
  [[ $- == *i* ]] && source "${OMARCHY_PATH:-/usr/share/omarchy}/default/bash/rc"
fi

# cca is the claude-dash wrapper at ~/.local/bin/cca (passes --permission-mode
# auto through to claude behind the capture proxy). No alias — an alias would
# shadow the wrapper and silently disable dashboard capture.

# Inbound ssh sessions land in a persistent tmux session (same as zsh/.zshrc);
# on Linux bash is the login shell, so skiff/ssh reconnects resume here too.
if [[ $- == *i* && -n "$SSH_TTY" && -z "$TMUX" ]] && command -v tmux >/dev/null; then
  exec tmux new-session -A -s main
fi
