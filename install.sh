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
    warn "Backing up existing $dst → $BACKUP_DIR/"
    mkdir -p "$BACKUP_DIR/$(dirname "${dst/#$HOME\//}")"
    cp -r "$dst" "$BACKUP_DIR/${dst/#$HOME\//}"
    rm -rf "$dst"
  fi

  ln -s "$src" "$dst"
  info "Linked: $dst → $src"
}

echo ""
echo "Installing dotfiles from $DOTFILES_DIR"
echo "────────────────────────────────────────"
echo ""

# ── Claude Code ──────────────────────────────────────────────────────────────
symlink "$DOTFILES_DIR/claude/settings.json" "$HOME/.claude/settings.json"

# ── Ghostty ──────────────────────────────────────────────────────────────────
symlink "$DOTFILES_DIR/ghostty/config" "$HOME/.config/ghostty/config"

echo ""
echo "────────────────────────────────────────"
echo "Done."
echo ""
warn "Review ghostty/config — 'working-directory' is set to a machine-specific path."
echo ""
