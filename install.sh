#!/usr/bin/env bash
set -euo pipefail

DOTFILES_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BACKUP_DIR="$DOTFILES_DIR/.backup/$(date +%Y%m%d_%H%M%S)"

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

info()    { echo -e "${GREEN}[✓]${NC} $*"; }
warn()    { echo -e "${YELLOW}[!]${NC} $*"; }
error()   { echo -e "${RED}[✗]${NC} $*"; }

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

# ── Homebrew ─────────────────────────────────────────────────────────────────
if ! command -v brew &>/dev/null; then
  info "Installing Homebrew..."
  /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
else
  info "Homebrew already installed"
fi

# ── Ghostty ──────────────────────────────────────────────────────────────────
if ! command -v ghostty &>/dev/null; then
  info "Installing Ghostty..."
  brew install --cask ghostty
else
  info "Ghostty already installed"
fi

symlink "$DOTFILES_DIR/ghostty/config" "$HOME/.config/ghostty/config"

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
  info "Building claude-dash (release)..."
  cargo build --release --manifest-path "$DOTFILES_DIR/claude-dash/Cargo.toml" --quiet
  symlink "$DOTFILES_DIR/claude-dash/target/release/claude-dash" "$HOME/.local/bin/claude-dash"
  symlink "$DOTFILES_DIR/claude-dash/bin/cca" "$HOME/.local/bin/cca"
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
for pkg in lavish-axi gh-axi chrome-devtools-axi; do
  if ! command -v "$pkg" &>/dev/null; then
    info "Installing $pkg..."
    npm install -g "$pkg"
  else
    info "$pkg already installed"
  fi
  "$pkg" setup hooks
done

# ── Neovim ───────────────────────────────────────────────────────────────────
if ! command -v nvim &>/dev/null; then
  info "Installing Neovim..."
  brew install neovim
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

# ── wt ───────────────────────────────────────────────────────────────────────
if ! command -v cargo &>/dev/null; then
  warn "Rust/cargo not found — skipping wt install. Install Rust from https://rustup.rs and re-run."
else
  info "Installing wt..."
  cargo install --path "$DOTFILES_DIR/wt" --quiet
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
