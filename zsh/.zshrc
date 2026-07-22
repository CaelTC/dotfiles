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
# Load full nvm only when you actually run `nvm` (e.g. to switch versions).
# The default node goes on PATH below (after the other prepends, so nvm wins).
nvm() { unset -f nvm; [ -s "$NVM_DIR/nvm.sh" ] && \. "$NVM_DIR/nvm.sh"; nvm "$@"; }


if [ -d /opt/homebrew ]; then
  export LDFLAGS="-L/opt/homebrew/opt/llvm/lib"
  export CPPFLAGS="-I/opt/homebrew/opt/llvm/include"
  export PATH="/opt/homebrew/bin:$PATH"
  export PATH="/opt/homebrew/sbin:$PATH"
fi
export PATH="$HOME/.cargo/bin:$PATH"
export PATH="$HOME/.local/bin:$PATH"

# ponytail: put the default nvm node straight on PATH — as fast as lazy-load, but
# snapshot-safe. The old self-referencing wrappers (node()->_load_nvm->node) recursed
# forever once Claude Code's shell snapshot captured them without _load_nvm. Placed
# after the prepends above so nvm's default beats Homebrew's node (matching old behaviour).
if [ -r "$NVM_DIR/alias/default" ]; then
  _nvm_bin="$NVM_DIR/versions/node/$(ls "$NVM_DIR/versions/node" 2>/dev/null | grep "^v$(cat "$NVM_DIR/alias/default")" | sort -V | tail -1)/bin"
  [ -d "$_nvm_bin" ] && PATH="$_nvm_bin:$PATH"
  unset _nvm_bin
fi

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

# gnhf needs a work tree; a bare-repo container (.bare + worktrees layout) isn't
# one, so `git status --porcelain` dies with "must be run in a work tree". If
# launched from such a container, cd into the checked-out worktree first.
# ponytail: picks the first non-bare worktree; add branch selection if you keep several.
gnhf() {
  if [ "$(git rev-parse --is-inside-work-tree 2>/dev/null)" != true ] && git rev-parse --git-dir >/dev/null 2>&1; then
    local wt
    wt=$(git worktree list --porcelain 2>/dev/null | awk '/^worktree /{p=$2;b=0} /^bare$/{b=1} /^HEAD /{if(!b){print p;exit}}')
    [ -n "$wt" ] && { echo "gnhf: bare container → running from $wt"; ( builtin cd "$wt" && command gnhf "$@" ); return; }
  fi
  command gnhf "$@"
}
