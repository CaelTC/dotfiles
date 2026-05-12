alias g="git"

eval "$(starship init zsh)"
source $(brew --prefix)/share/zsh-autosuggestions/zsh-autosuggestions.zsh
PQ_LIB_DIR="$(brew --prefix libpq)/lib"

export NVM_DIR="$([ -z "${XDG_CONFIG_HOME-}" ] && printf %s "${HOME}/.nvm" || printf %s "${XDG_CONFIG_HOME}/nvm")"
[ -s "$NVM_DIR/nvm.sh" ] && \. "$NVM_DIR/nvm.sh"


eval "$(frum init)"
  export LDFLAGS="-L/opt/homebrew/opt/openssl@1.1/lib"
  export CPPFLAGS="-I/opt/homebrew/opt/openssl@1.1/include"


export LDFLAGS="-L/opt/homebrew/opt/llvm/lib"
export CPPFLAGS="-I/opt/homebrew/opt/llvm/include"
export PATH="opt/homebrew/bin:$PATH"
export PATH="opt/homebrew/sbin:$PATH"
export PATH="$HOME/.cargo/bin:$PATH"
cd() {
    builtin cd "$@" && ls -C
}
eval "$(~/.local/bin/mise activate)"

# bun completions
[ -s "/Users/macbook/.bun/_bun" ] && source "/Users/macbook/.bun/_bun"

# bun
export BUN_INSTALL="$HOME/.bun"
export PATH="$BUN_INSTALL/bin:$PATH"

# >>> conda initialize >>>
# !! Contents within this block are managed by 'conda init' !!
__conda_setup="$('/Users/macbook/miniconda3/bin/conda' 'shell.zsh' 'hook' 2> /dev/null)"
if [ $? -eq 0 ]; then
    eval "$__conda_setup"
else
    if [ -f "/Users/macbook/miniconda3/etc/profile.d/conda.sh" ]; then
        . "/Users/macbook/miniconda3/etc/profile.d/conda.sh"
    else
        export PATH="/Users/macbook/miniconda3/bin:$PATH"
    fi
fi
unset __conda_setup
# <<< conda initialize <<<

