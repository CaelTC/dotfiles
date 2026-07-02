## Agent skills

### Issue tracker

Issues and PRDs live as local markdown files under `.scratch/<feature>/`. See `docs/agents/issue-tracker.md`.

### Triage labels

Five canonical triage roles, each mapped to its default string (`needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`). See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: one `CONTEXT.md` + `docs/adr/` at the repo root. See `docs/agents/domain.md`.

### Menu-bar icon

The SwiftBar readout (`claude-dash status`) prefixes its title with the white
Claude mark, not a text emoji. Source of truth is `assets/splash.svg` (the Claude
mark recoloured white); the committed `assets/splash.png` (44×44 RGBA, @2x) is
rasterized from it and compiled in via `include_bytes!` as `SPLASH_PNG` in
`src/status.rs`, base64-encoded at first use and emitted via SwiftBar's `image=`
(NOT `templateImage=`) so it renders always-white rather than tinting to the menu
bar. The emitted title line also carries `width=18 height=18`: SwiftBar hands the
decoded PNG to `NSImage(data:)` with no size cap, and a 44px PNG with no DPI
metadata is 44 *points* (floods the menu bar); SwiftBar scales a line's image
only when BOTH params are present (`MenuLineParameters.resizedImageIfRequested`).
The 44px bitmap stays as the backing so the mark is crisp on retina.
To change the icon, edit the SVG and re-rasterize the PNG — nothing else;
the base64 is derived in code, never hand-pasted (a hand-maintained const once
shipped two mangled bytes and SwiftBar silently rendered no icon at all).
