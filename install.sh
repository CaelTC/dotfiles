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

# Per-skill symlinks so shared skills coexist with tool-managed ones in ~/.claude/skills
for skill in "$DOTFILES_DIR"/claude/skills/*/; do
  symlink "${skill%/}" "$HOME/.claude/skills/$(basename "$skill")"
done

# nootkify skill lives in a private repo (brand assets), not in this public one
NOOTKIFY_DIR="$HOME/Projects/nootkify"
if [ ! -d "$NOOTKIFY_DIR/.git" ]; then
  info "Cloning nootkify..."
  git clone git@github.com:CaelTC/nootkify.git "$NOOTKIFY_DIR" || warn "nootkify clone failed (private repo — need ssh access)"
else
  info "Updating nootkify..."
  git -C "$NOOTKIFY_DIR" pull --ff-only --quiet || warn "nootkify pull failed"
fi
[ -d "$NOOTKIFY_DIR" ] && symlink "$NOOTKIFY_DIR" "$HOME/.claude/skills/nootkify"

# ── claude-dash (cca) ────────────────────────────────────────────────────────
if ! command -v cargo &>/dev/null; then
  warn "Rust/cargo not found — skipping claude-dash install. Install Rust from https://rustup.rs and re-run."
else
  info "Building claude-dash (release)..."
  cargo build --release --manifest-path "$DOTFILES_DIR/claude-dash/Cargo.toml" --quiet
  symlink "$DOTFILES_DIR/claude-dash/target/release/claude-dash" "$HOME/.local/bin/claude-dash"
  symlink "$DOTFILES_DIR/claude-dash/bin/cca" "$HOME/.local/bin/cca"
  symlink "$DOTFILES_DIR/claude-dash/bin/ccagent" "$HOME/.local/bin/ccagent"
fi

# ── Omarchy (Linux desktop) ──────────────────────────────────────────────────
# Only the files that differ from Omarchy's shipped defaults. The replaced
# defaults land in .backup/, so diff against them after an Omarchy upgrade.
# Theme backgrounds are personal photos and stay out of this public repo:
# restore them into ~/.config/omarchy/themes/highline/backgrounds/ by hand.
if command -v omarchy &>/dev/null; then
  for f in "$DOTFILES_DIR"/omarchy/hypr/*.lua; do
    symlink "$f" "$HOME/.config/hypr/$(basename "$f")"
  done
  for f in shell.json shell.toml lockscreen.jpg branding/screensaver.txt defaults/agent; do
    symlink "$DOTFILES_DIR/omarchy/$f" "$HOME/.config/omarchy/$f"
  done
  for plugin in "$DOTFILES_DIR"/omarchy/plugins/*/; do
    symlink "${plugin%/}" "$HOME/.config/omarchy/plugins/$(basename "$plugin")"
  done
  for f in "$DOTFILES_DIR"/omarchy/themes/highline/*; do
    symlink "$f" "$HOME/.config/omarchy/themes/highline/$(basename "$f")"
  done
  # Theme templates and theme-set hooks (herdr-app.json.tpl + 20-herdr-app
  # carry the Omarchy theme into the Herdr desktop app).
  for f in "$DOTFILES_DIR"/omarchy/themed/*.tpl; do
    symlink "$f" "$HOME/.config/omarchy/themed/$(basename "$f")"
  done
  for f in "$DOTFILES_DIR"/omarchy/hooks/theme-set.d/*; do
    symlink "$f" "$HOME/.config/omarchy/hooks/theme-set.d/$(basename "$f")"
  done
  # Clock screensaver; hypr/hyprland.lua puts this dir ahead of $OMARCHY_PATH/bin.
  symlink "$DOTFILES_DIR/omarchy/bin/omarchy-screensaver" "$HOME/.local/share/omarchy-overrides/bin/omarchy-screensaver"
  for f in "$DOTFILES_DIR"/omarchy/webapps/*.desktop; do
    symlink "$f" "$HOME/.local/share/applications/$(basename "$f")"
  done
  for f in "$DOTFILES_DIR"/omarchy/webapps/icons/*.png; do
    symlink "$f" "$HOME/.local/share/icons/hicolor/256x256/apps/$(basename "$f")"
  done
  symlink "$DOTFILES_DIR/tmux/tmux.conf" "$HOME/.config/tmux/tmux.conf"
  symlink "$DOTFILES_DIR/voxtype/config.toml" "$HOME/.config/voxtype/config.toml"
fi

# ── fzl (Whistler freezing level / wind bar widget) ──────────────────────────
symlink "$DOTFILES_DIR/fzl/fzl" "$HOME/.local/bin/fzl"
if [ "$OS" = "Darwin" ]; then
  [ -d /Applications/SwiftBar.app ] || { info "Installing SwiftBar..."; brew install --cask swiftbar; }
  symlink "$DOTFILES_DIR/fzl/swiftbar/fzl.15m.sh" "$HOME/Library/Application Support/SwiftBar/Plugins/fzl.15m.sh"
else
  # Omarchy bar: ~/.config/omarchy/shell.json runs it as `command` widgets fzl / wind.
  symlink "$DOTFILES_DIR/fzl/fzl" "$HOME/.config/omarchy/bar/scripts/fzl"
fi

# ── herdr ────────────────────────────────────────────────────────────────────
if ! command -v herdr &>/dev/null; then
  info "Installing herdr..."
  curl -fsSL https://herdr.dev/install.sh | sh
else
  info "herdr already installed"
fi

symlink "$DOTFILES_DIR/herdr/config.toml" "$HOME/.config/herdr/config.toml"

herdr plugin link "$DOTFILES_DIR/herdr/plugins/two-tabs" &>/dev/null || true

# ── Herdr app (and herdr-core, `hc`) ─────────────────────────────────────────
# The desktop app, separate from the herdr TUI above. Its installer puts the app
# in ~/.local/share/herdr-app (Linux) or ~/Applications/Herdr.app (macOS, Apple
# silicon only), links `herdr-app` into ~/.local/bin, and installs herdr-core
# (`hc`) there if there's none yet. After that the app updates itself, so it is
# only installed when missing.
if command -v herdr-app &>/dev/null || [ -d "$HOME/Applications/Herdr.app" ] || [ -d /Applications/Herdr.app ]; then
  info "Herdr app already installed"
else
  info "Installing the Herdr app..."
  curl -fsSL https://releases.herdr.dev/app/install.sh | sh
fi

# ── minions (the `minion` CLI) ───────────────────────────────────────────────
# Long-lived Claude agents across machines (github.com/CaelTC/minions). Every
# machine keeps the framework at ~/Projects/minions (minions import its
# framework/FRAMEWORK.md from there) and the CLI from its cli/ crate. The Herdr
# mods below (minion-inbox, the Minions group in my-agents) call
# ~/.cargo/bin/minion. Hosting minions or running a seat gate is set up by hand:
# see the minions README.
MINIONS_DIR="$HOME/Projects/minions"
if [ ! -d "$MINIONS_DIR/.git" ]; then
  info "Cloning minions..."
  git clone git@github.com:CaelTC/minions.git "$MINIONS_DIR" || warn "minions clone failed (need ssh access to github.com/CaelTC/minions)"
else
  info "Updating minions..."
  git -C "$MINIONS_DIR" pull --ff-only --quiet || warn "minions pull failed (local changes?) — leaving it as it is"
fi
if [ ! -d "$MINIONS_DIR/cli" ]; then
  :
elif ! command -v cargo &>/dev/null; then
  warn "Rust/cargo not found — skipping the minion CLI. Install Rust from https://rustup.rs and re-run."
else
  info "Installing the minion CLI..."
  cargo install --path "$MINIONS_DIR/cli" --quiet
fi

# ── Herdr app: mods, layout, firstmate pirate-flag icon ──────────────────────
# The Herdr desktop app's agents list, tab bar and pane header are mods
# (herdr-app/mods/my-{agents,tabs,pane}, used by the my-main layout). They show
# a pirate flag instead of Claude's icon for an agent running in a firstmate
# home (see herdr-app/mods/my-pane/firstmate.js), tinted with this machine's
# color: the `firstmate` entry merged into programs.json (gen-programs.sh).
# my-agents also groups minions under a Minions header with the minion icon,
# and the minion-inbox mod is the mail badge in my-main's status row.
if ! command -v jq &>/dev/null; then
  info "Installing jq..."
  pkg_install jq
fi

# Herdr copies, not links: it refuses to load icons (and mod imports) through
# a symlink that leads out of ~/.config/herdr-app. A local copy that differs is
# backed up first, never clobbered; the app hot-reloads what changes. Re-run
# install.sh after editing these files in dotfiles.
copy_install() {
  local src="$1" dst="$2"
  if [ ! -L "$dst" ] && [ -e "$dst" ] && diff -rq -x gpui-kit.d.ts -x jsconfig.json "$src" "$dst" &>/dev/null; then
    info "Already up to date: $dst"
    return
  fi
  if [ -L "$dst" ]; then
    rm "$dst"
  elif [ -e "$dst" ]; then
    local backup_path="$BACKUP_DIR/${dst/#$HOME\//}"
    warn "Backing up existing $dst → $backup_path"
    mkdir -p "$(dirname "$backup_path")"
    cp -R "$dst" "$backup_path"
  fi
  mkdir -p "$(dirname "$dst")"
  if [ -d "$src" ]; then
    mkdir -p "$dst"
    cp -R "$src/." "$dst/"
  else
    cp "$src" "$dst"
  fi
  info "Installed: $dst"
}

HERDR_APP_HOME="${HERDR_APP_HOME:-$HOME/.config/herdr-app}"
copy_install "$DOTFILES_DIR/herdr-app/firstmate-flag.svg" "$HERDR_APP_HOME/my-icons/firstmate-flag.svg"
copy_install "$DOTFILES_DIR/herdr-app/minion.svg" "$HERDR_APP_HOME/my-icons/minion.svg"
for mod_dir in "$DOTFILES_DIR"/herdr-app/mods/*/; do
  mod_dir="${mod_dir%/}"
  copy_install "$mod_dir" "$HERDR_APP_HOME/mods/$(basename "$mod_dir")"
done

# my-main: the main layout with the agents list, tab bar and pane header swapped
# for the my-* mods above. Pick it with ctrl-shift-l (or layout.switch my-main).
copy_install "$DOTFILES_DIR/herdr-app/layouts/my-main.json" "$HERDR_APP_HOME/layouts/my-main.json"

# Global keys for the mods, merged in so the rest of keys.json stays as it is:
# ctrl-a opens the my-prefix mod (Herdr TUI prefix keys: ctrl-a, then a key),
# ctrl-shift-m the minion-inbox (messages from minions waiting for approval).
# Any edit freezes keys.json: new app default shortcuts must then be added by
# hand. The app writes keys.json when it first opens, so on a new machine open
# Herdr once, then re-run install.sh.
KEYS_JSON="$HERDR_APP_HOME/keys.json"
MY_KEYS='{"ctrl-a": "mod.open:my-prefix", "ctrl-shift-m": "mod.open:minion-inbox"}'
if [ -f "$KEYS_JSON" ]; then
  if [ "$(jq --argjson k "$MY_KEYS" '.global as $g | $k | to_entries | all(.value == $g[.key])' "$KEYS_JSON")" != "true" ]; then
    backup_path="$BACKUP_DIR/${KEYS_JSON/#$HOME\//}"
    mkdir -p "$(dirname "$backup_path")"
    cp "$KEYS_JSON" "$backup_path"
    jq --argjson k "$MY_KEYS" '.global += $k' "$backup_path" > "$KEYS_JSON"
    info "Bound ctrl-a (my-prefix) and ctrl-shift-m (minion-inbox) in $KEYS_JSON"
  fi
else
  warn "No $KEYS_JSON yet — open the Herdr app once, then re-run install.sh to bind ctrl-a and ctrl-shift-m"
fi

# Follow the Omarchy theme: settings.json's `theme` names themes/omarchy.json,
# which the 20-herdr-app theme-set hook keeps current. Only that key changes.
# Omarchy only: elsewhere (macOS) there is no omarchy theme, so the app keeps its own.
SETTINGS_JSON="$HERDR_APP_HOME/settings.json"
if ! command -v omarchy &>/dev/null; then
  :
elif [ -f "$SETTINGS_JSON" ]; then
  if [ "$(jq -r '.theme // empty' "$SETTINGS_JSON" 2>/dev/null)" != "omarchy" ]; then
    if merged="$(jq '.theme = "omarchy"' "$SETTINGS_JSON" 2>/dev/null)"; then
      printf '%s\n' "$merged" > "$SETTINGS_JSON"
      info "Set the Herdr app theme to omarchy in $SETTINGS_JSON"
    else
      warn "$SETTINGS_JSON is not valid JSON — set \"theme\": \"omarchy\" by hand"
    fi
  fi
else
  mkdir -p "$HERDR_APP_HOME"
  printf '{ "theme": "omarchy" }\n' > "$SETTINGS_JSON"
fi
[ -x "$HOME/.config/omarchy/hooks/theme-set.d/20-herdr-app" ] && "$HOME/.config/omarchy/hooks/theme-set.d/20-herdr-app"

# Merge into programs.json (never clobber: back up an existing one first, and
# only replace the `firstmate` entry). Re-running yields the same file.
PROGRAMS_JSON="$HERDR_APP_HOME/programs.json"
PROGRAMS_NEW="$("$DOTFILES_DIR/herdr-app/gen-programs.sh")"
if [ -f "$PROGRAMS_JSON" ]; then
  if merged="$(jq --argjson new "$PROGRAMS_NEW" '.programs = ((.programs // {}) + $new.programs)' "$PROGRAMS_JSON" 2>/dev/null)"; then
    if [ "$merged" = "$(jq . "$PROGRAMS_JSON")" ]; then
      info "Already up to date: $PROGRAMS_JSON"
    else
      backup_path="$BACKUP_DIR/${PROGRAMS_JSON/#$HOME\//}"
      warn "Backing up existing $PROGRAMS_JSON → $backup_path"
      mkdir -p "$(dirname "$backup_path")"
      cp "$PROGRAMS_JSON" "$backup_path"
      printf '%s\n' "$merged" > "$PROGRAMS_JSON"
      info "Merged firstmate entry into $PROGRAMS_JSON"
    fi
  else
    warn "$PROGRAMS_JSON is not valid JSON — leaving it alone; add the firstmate entry by hand"
  fi
else
  mkdir -p "$HERDR_APP_HOME"
  printf '%s\n' "$PROGRAMS_NEW" > "$PROGRAMS_JSON"
  info "Wrote $PROGRAMS_JSON"
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

# ── Starship ─────────────────────────────────────────────────────────────────
symlink "$DOTFILES_DIR/starship/starship.toml" "$HOME/.config/starship.toml"
# fsmonitor + untracked cache keep cold `git status` fast in big repos, so the
# starship prompt never stalls past command_timeout on cd.
git config --global core.fsmonitor true
git config --global core.untrackedCache true

git config --global alias.cam 'commit -am'
git config --global alias.pl pull
git config --global alias.ps push

echo ""
echo "────────────────────────────────────────"
echo "Done."
echo ""
