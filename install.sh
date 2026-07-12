#!/usr/bin/env bash
set -euo pipefail

DOTFILES_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BACKUP_DIR="$DOTFILES_DIR/.backup/$(date +%Y%m%d_%H%M%S)"
OS="$(uname -s)"   # Darwin | Linux

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

info()    { echo -e "${GREEN}[✓]${NC} $*"; }
warn()    { echo -e "${YELLOW}[!]${NC} $*"; }
error()   { echo -e "${RED}[✗]${NC} $*"; }

# brew on macOS, native package manager on Linux
pkg_install() {
  if [ "$OS" = "Darwin" ]; then
    brew install "$@"
  elif command -v apt-get &>/dev/null; then
    sudo apt-get install -y "$@"
  elif command -v dnf &>/dev/null; then
    sudo dnf install -y "$@"
  elif command -v pacman &>/dev/null; then
    sudo pacman -S --needed --noconfirm "$@"
  else
    error "No supported package manager — install manually: $*"
    return 1
  fi
}

symlink() {
  local src="$1"
  local dst="$2"

  mkdir -p "$(dirname "$dst")"

  if [ -L "$dst" ]; then
    local current_target
    current_target="$(readlink "$dst")"
    if [ "$current_target" = "$src" ]; then
      info "Already linked: $dst"
      return
    else
      warn "Relinking $dst (was → $current_target)"
      rm "$dst"
    fi
  elif [ -e "$dst" ]; then
    local backup_path="$BACKUP_DIR/${dst/#$HOME\//}"
    warn "Backing up existing $dst → $backup_path"
    mkdir -p "$(dirname "$backup_path")"
    cp -r "$dst" "$backup_path"
    rm -rf "$dst"
  fi

  ln -s "$src" "$dst"
  info "Linked: $dst → $src"
}

echo ""
echo "Installing dotfiles from $DOTFILES_DIR"
echo "────────────────────────────────────────"
echo ""

# ── Homebrew (macOS only) ────────────────────────────────────────────────────
if [ "$OS" = "Darwin" ]; then
  if ! command -v brew &>/dev/null; then
    info "Installing Homebrew..."
    /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
  else
    info "Homebrew already installed"
  fi
fi

# ── Ghostty ──────────────────────────────────────────────────────────────────
if [ "$OS" != "Darwin" ]; then
  info "Skipping Ghostty install (GUI terminal, macOS cask only)"
elif ! command -v ghostty &>/dev/null; then
  info "Installing Ghostty..."
  brew install --cask ghostty
else
  info "Ghostty already installed"
fi

symlink "$DOTFILES_DIR/ghostty/config" "$HOME/.config/ghostty/config"

# ── Node/npm (needed for Claude Code + axi tools) ────────────────────────────
if ! command -v npm &>/dev/null; then
  info "Installing node/npm..."
  if [ "$OS" = "Darwin" ]; then pkg_install node; else pkg_install nodejs npm; fi
fi
if [ "$OS" = "Linux" ] && [ ! -w "$(npm config get prefix)" ]; then
  # ponytail: system node's global prefix (/usr) needs sudo; ~/.local/bin is already on PATH
  npm config set prefix "$HOME/.local"
fi

# ── Claude Code ──────────────────────────────────────────────────────────────
if ! command -v claude &>/dev/null; then
  info "Installing Claude Code..."
  npm install -g @anthropic-ai/claude-code
else
  info "Claude Code already installed"
fi

symlink "$DOTFILES_DIR/claude/settings.json" "$HOME/.claude/settings.json"
symlink "$DOTFILES_DIR/claude/statusline-command.sh" "$HOME/.claude/statusline-command.sh"
symlink "$DOTFILES_DIR/claude/agents" "$HOME/.claude/agents"

# ── claude-dash (cca) ────────────────────────────────────────────────────────
if ! command -v cargo &>/dev/null; then
  warn "Rust/cargo not found — skipping claude-dash install. Install Rust from https://rustup.rs and re-run."
else
  # cca/ccagent have a zsh shebang (and use zsh-only syntax); Linux boxes
  # default to bash and may ship without it.
  command -v zsh &>/dev/null || { info "Installing zsh (cca/ccagent need it)..."; pkg_install zsh; }
  info "Building claude-dash (release)..."
  cargo build --release --manifest-path "$DOTFILES_DIR/claude-dash/Cargo.toml" --quiet
  symlink "$DOTFILES_DIR/claude-dash/target/release/claude-dash" "$HOME/.local/bin/claude-dash"
  symlink "$DOTFILES_DIR/claude-dash/bin/cca" "$HOME/.local/bin/cca"
  symlink "$DOTFILES_DIR/claude-dash/bin/ccagent" "$HOME/.local/bin/ccagent"
fi

# ── treehouse ────────────────────────────────────────────────────────────────
if ! command -v treehouse &>/dev/null; then
  info "Installing treehouse..."
  curl -fsSL https://kunchenguid.github.io/treehouse/install.sh | sh
else
  info "treehouse already installed"
fi

# ── no-mistakes ──────────────────────────────────────────────────────────────
if ! command -v no-mistakes &>/dev/null; then
  info "Installing no-mistakes..."
  curl -fsSL https://raw.githubusercontent.com/kunchenguid/no-mistakes/main/docs/install.sh | sh
else
  info "no-mistakes already installed"
fi

# ── Claude agent tools (axi) ─────────────────────────────────────────────────
export PATH="$(npm prefix -g)/bin:$PATH"
for pkg in lavish-axi gh-axi chrome-devtools-axi tasks-axi; do
  if ! command -v "$pkg" &>/dev/null; then
    info "Installing $pkg..."
    npm install -g "$pkg"
    hash -r
  else
    info "$pkg already installed"
  fi
  # SessionStart hooks only for the cheap dynamic-status tools; lavish-axi and
  # chrome-devtools-axi cost ~2k tokens/session and are reachable via their skills.
  case "$pkg" in gh-axi|tasks-axi) "$pkg" setup hooks ;; esac
done

# ── Neovim ───────────────────────────────────────────────────────────────────
if ! command -v nvim &>/dev/null; then
  info "Installing Neovim..."
  pkg_install neovim
else
  info "Neovim already installed"
fi

if [ ! -d "$DOTFILES_DIR/nvim" ]; then
  info "Adding nvim submodule..."
  git -C "$DOTFILES_DIR" submodule add git@github.com:CaelTC/nvim_config.git nvim
else
  info "Updating nvim submodule..."
  git -C "$DOTFILES_DIR" submodule update --init --remote nvim
fi

symlink "$DOTFILES_DIR/nvim" "$HOME/.config/nvim"

# ── ssh-ls ───────────────────────────────────────────────────────────────────
if [ ! -d "$DOTFILES_DIR/ssh-ls" ]; then
  info "Adding ssh-ls submodule..."
  git -C "$DOTFILES_DIR" submodule add git@github.com:CaelTC/ssh-ls.git ssh-ls
else
  info "Updating ssh-ls submodule..."
  git -C "$DOTFILES_DIR" submodule update --init --remote ssh-ls
fi

info "Running ssh-ls installer..."
"$DOTFILES_DIR/ssh-ls/install.sh"

# ── Remote fleet: tailscale + tmux + skiff ───────────────────────────────────
if ! command -v tmux &>/dev/null; then
  info "Installing tmux..."
  pkg_install tmux
else
  info "tmux already installed"
fi

if ! command -v tailscale &>/dev/null; then
  info "Installing tailscale (tailscaled)..."
  if [ "$OS" = "Darwin" ]; then
    # Formula (tailscaled) variant, NOT the cask — the GUI app can't run the
    # Tailscale SSH server on macOS.
    brew install tailscale
  else
    # Official installer: handles all distros, enables + starts tailscaled.
    curl -fsSL https://tailscale.com/install.sh | sh
  fi
else
  info "tailscale already installed"
fi

if tailscale status &>/dev/null; then
  if tailscale set --ssh &>/dev/null; then
    info "Tailscale SSH enabled"
  else
    warn "Could not enable Tailscale SSH — run: tailscale up --ssh"
  fi
else
  warn "tailscaled not running. Start it and join the tailnet with SSH enabled:"
  if [ "$OS" = "Darwin" ]; then
    warn "  sudo brew services start tailscale && tailscale up --ssh"
  else
    warn "  sudo systemctl enable --now tailscaled && sudo tailscale up --ssh"
  fi
fi

if ! command -v cargo &>/dev/null; then
  warn "Rust/cargo not found — skipping skiff install. Install Rust from https://rustup.rs and re-run."
else
  info "Installing skiff..."
  cargo install --path "$DOTFILES_DIR/skiff" --quiet
fi

# ── Bash ─────────────────────────────────────────────────────────────────────
symlink "$DOTFILES_DIR/bash/.bashrc" "$HOME/.bashrc"
symlink "$DOTFILES_DIR/bash/.bash_profile" "$HOME/.bash_profile"

# ── Zsh ──────────────────────────────────────────────────────────────────────
symlink "$DOTFILES_DIR/zsh/.zshrc" "$HOME/.zshrc"

echo ""
echo "────────────────────────────────────────"
echo "Done."
echo ""
