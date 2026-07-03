export PATH="$HOME/.local/bin:$PATH"
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"

# Launch Claude CLI in auto permission mode
alias cca='claude --permission-mode auto'

# Inbound ssh sessions land in a persistent tmux session (same as zsh/.zshrc);
# on Linux bash is the login shell, so skiff/ssh reconnects resume here too.
if [[ $- == *i* && -n "$SSH_TTY" && -z "$TMUX" ]] && command -v tmux >/dev/null; then
  exec tmux new-session -A -s main
fi
