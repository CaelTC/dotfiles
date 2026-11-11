#!/bin/bash
# SwiftBar plugin: Whistler freezing level + peak wind in the macOS menu bar.
# SwiftBar re-runs this every 15 min (the `.15m.` in the filename) and renders
# the `fzl --swiftbar` output. Same script as the Omarchy bar widgets on Linux.
#
# Install: `brew install --cask swiftbar`, then symlink this into SwiftBar's
# plugins folder (install.sh does both):
#   ln -s "$PWD/fzl/swiftbar/fzl.15m.sh" ~/Library/Application\ Support/SwiftBar/Plugins/

BIN="$(command -v fzl || echo "$HOME/.local/bin/fzl")"
exec "$BIN" --swiftbar
