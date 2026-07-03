[ -x /opt/homebrew/bin/brew ] && eval "$(/opt/homebrew/bin/brew shellenv bash)"
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"

# Source .bashrc so login/SSH shells get PATH, aliases, etc.
[ -f "$HOME/.bashrc" ] && . "$HOME/.bashrc"
