alias g="git"
# cca is the claude-dash wrapper at ~/.local/bin/cca (passes --permission-mode
# auto through to claude behind the capture proxy). No alias — an alias would
# shadow the wrapper and silently disable dashboard capture.

command -v starship >/dev/null && eval "$(starship init zsh)"
if command -v brew >/dev/null; then
  source "$(brew --prefix)/share/zsh-autosuggestions/zsh-autosuggestions.zsh"
  PQ_LIB_DIR="$(brew --prefix libpq)/lib"
elif [ -f /usr/share/zsh-autosuggestions/zsh-autosuggestions.zsh ]; then
  source /usr/share/zsh-autosuggestions/zsh-autosuggestions.zsh
fi

export NVM_DIR="$([ -z "${XDG_CONFIG_HOME-}" ] && printf %s "${HOME}/.nvm" || printf %s "${XDG_CONFIG_HOME}/nvm")"
# ponytail: lazy-load nvm — eager load was ~400ms/shell (the startup hang).
# First node/npm/npx/nvm call sources nvm.sh, then runs the real command.
_load_nvm() { unset -f nvm node npm npx _load_nvm; [ -s "$NVM_DIR/nvm.sh" ] && \. "$NVM_DIR/nvm.sh"; }
nvm()  { _load_nvm; nvm "$@"; }
node() { _load_nvm; node "$@"; }
npm()  { _load_nvm; npm "$@"; }
npx()  { _load_nvm; npx "$@"; }


if [ -d /opt/homebrew ]; then
  export LDFLAGS="-L/opt/homebrew/opt/llvm/lib"
  export CPPFLAGS="-I/opt/homebrew/opt/llvm/include"
  export PATH="/opt/homebrew/bin:$PATH"
  export PATH="/opt/homebrew/sbin:$PATH"
fi
export PATH="$HOME/.cargo/bin:$PATH"
export PATH="$HOME/.local/bin:$PATH"

# Inbound ssh sessions land in a persistent tmux session: disconnects don't
# kill work, and reconnecting (ssh or skiff) resumes where you left off.
# $TMUX guard stops recursion once inside tmux.
if [[ -o interactive && -n "$SSH_TTY" && -z "$TMUX" ]] && command -v tmux >/dev/null; then
  exec tmux new-session -A -s main
fi

cd() {
    if [[ $1 =~ '^\.{3,}$' ]]; then
        local dots=${#1}
        local target=".."
        for ((i=2; i<dots; i++)); do
            target+="/.."
        done
        builtin cd "$target" && ls -C
    else
        builtin cd "$@" && ls -C
    fi
}
command -v mise >/dev/null && eval "$(mise activate zsh)"

# initialize completion system (must run before sourcing completion files)
autoload -Uz compinit && compinit

# bun completions
[ -s "$HOME/.bun/_bun" ] && source "$HOME/.bun/_bun"

# bun
export BUN_INSTALL="$HOME/.bun"
export PATH="$BUN_INSTALL/bin:$PATH"

# >>> conda initialize >>>
# !! Contents within this block are managed by 'conda init' !!
__conda_setup="$("$HOME/miniconda3/bin/conda" 'shell.zsh' 'hook' 2> /dev/null)"
if [ $? -eq 0 ]; then
    eval "$__conda_setup"
else
    if [ -f "$HOME/miniconda3/etc/profile.d/conda.sh" ]; then
        . "$HOME/miniconda3/etc/profile.d/conda.sh"
    else
        export PATH="$HOME/miniconda3/bin:$PATH"
    fi
fi
unset __conda_setup
# <<< conda initialize <<<

export PATH="$HOME/.local/bin:$PATH"

# no-mistakes
export PATH="$HOME/.no-mistakes/bin:$PATH"
