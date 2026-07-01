#!/bin/bash
# SwiftBar plugin: menu-bar readout of Claude subscription Budget (Usage %).
# SwiftBar re-runs this every 15s (the `.15s.` in the filename) and renders the
# `claude-dash status` SwiftBar output. `status` reads the store, so no proxy or
# TUI needs to be running.
#
# Install: `brew install --cask swiftbar`, then symlink this into SwiftBar's
# plugins folder — see claude-dash/README.md.

BIN="$(command -v claude-dash || echo "$HOME/.local/bin/claude-dash")"
exec "$BIN" status
