## Agent skills

### Issue tracker

Issues and PRDs live as local markdown files under `.scratch/<feature>/`. See `docs/agents/issue-tracker.md`.

### Triage labels

Five canonical triage roles, each mapped to its default string (`needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`). See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: one `CONTEXT.md` + `docs/adr/` at the repo root. See `docs/agents/domain.md`.

### Menu-bar icon

The SwiftBar readout (`claude-dash status`) prefixes its title with a monochrome
radial-burst "splash" glyph, not a text emoji. Source of truth is
`assets/splash.svg`; the committed `assets/splash.png` (44×44 RGBA, @2x) is
rasterized from it and embedded base64 as `SPLASH_ICON` in `src/status.rs`,
emitted via SwiftBar's `templateImage=` so it tints to the menu bar (light/dark).
To change the icon, edit the SVG, re-rasterize the PNG, and regenerate the const.
