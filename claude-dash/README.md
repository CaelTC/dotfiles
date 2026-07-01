# claude-dash

A live terminal dashboard for watching consumption of a Claude subscription —
**Budget** (Usage %) and **Throughput** — across concurrent **Session**s. Run
`claude-dash` for the TUI; the capture **Proxy** and record subcommands are
wired up by the `cca` wrapper. See `CONTEXT.md` for the domain language.

## macOS menu-bar readout (SwiftBar)

`claude-dash status` prints the current **Budget** as [SwiftBar](https://github.com/swiftbar/SwiftBar)
menu-bar output (headline = the **Representative Window**'s **Utilization** %,
coloured by severity, prefixed by a monochrome radial-burst "splash" glyph)
straight from the store, so nothing needs to be running. The glyph ships as
`assets/splash.png` (rasterized from `assets/splash.svg`) embedded as a SwiftBar
`templateImage=`, so it tints to the menu bar and adapts to light/dark mode.
Install with `brew install --cask swiftbar`, then symlink the plugin into
SwiftBar's plugins folder and refresh (⌥-click the menu-bar item → Refresh):
`ln -s "$PWD/bin/swiftbar/claude-usage.15s.sh" ~/Library/Application\ Support/SwiftBar/Plugins/`.
The `.15s.` in the filename tells SwiftBar to re-run it every 15 seconds; the %
only moves when the **Proxy** captures a response, so between requests it shows
the last captured reading. The plugin resolves `claude-dash` from `PATH`, falling
back to `~/.local/bin/claude-dash`.
