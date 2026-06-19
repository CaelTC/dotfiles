eval "$(/opt/homebrew/bin/brew shellenv bash)"
. "$HOME/.cargo/env"

# Source .bashrc so login/SSH shells get PATH, aliases, etc.
[ -f "$HOME/.bashrc" ] && . "$HOME/.bashrc"
