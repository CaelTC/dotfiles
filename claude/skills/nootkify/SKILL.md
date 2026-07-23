---
name: nootkify
description: Boldly redesign an existing UI to follow the Nootka Saunas brand — cedar-barrel-sauna, Pacific Northwest aesthetic (Evergreen/Coastal Water/Reindeer Moss palette, Heat orange accent, DM Sans, arched/circular motifs, split layouts). Use when the user asks to "nootkify", apply the Nootka brand, or redesign a page/component/app to the Nootka Saunas design guidelines.
---

# Nootkify

Transform an existing UI into the **Nootka Saunas** brand: a calm, premium,
Pacific-Northwest aesthetic — cedar, fog, water, fire. The redesign must be
**extensive and bold**, not a timid recolor. Reimagine layout, type scale,
imagery framing, and motion — then re-skin to the palette and tokens below.

> **Authoritative source — always read first, every run:**
> `/Users/caeltrudeau-cauchon/Documents/nootka-design/DESIGN_GUIDELINES.md`
> (Nootka Guidebook v2022.1). This is the binding spec; it wins on any
> conflict and must be followed in full. Read it before changing any markup
> or styles — do not rely on memory or the summaries below. The bundled
> [brand-reference.md](brand-reference.md) is only a fallback if that file is
> genuinely unreadable; if so, say so before proceeding.

## Workflow

1. **Scope.** Identify the target surface(s) — file(s), components, or whole
   app. Read the current markup/styles before changing anything.
2. **Load brand (required).** Always read
   `/Users/caeltrudeau-cauchon/Documents/nootka-design/DESIGN_GUIDELINES.md`
   in full first and treat every rule in it as binding — only fall back to
   [brand-reference.md](brand-reference.md) if that file is unreadable. Inject
   tokens from [brand-tokens.css](brand-tokens.css) into `:root` and wire DM Sans.
3. **Redesign boldly** (see directive below). Restructure the layout, don't
   just swap colors.
4. **Re-skin to tokens.** Replace every hardcoded color/font/radius with a
   semantic token. No off-palette hex values survive.
5. **Verify** against the checklist. Flag — don't silently break — anything
   that conflicts with the guidelines (especially logo rules).

## The bold-redesign directive

Push hard. A nootkified surface should be unmistakably Nootka at a glance:

- **Dramatic sections.** Build full-bleed **dark brand blocks** (Evergreen or
  Coastal Water) with white/Steam text, alternating with light Steam/white
  sections. Big vertical rhythm; let it breathe.
- **Split layouts.** Solid brand-color panel beside full-bleed photo or
  content — the signature Nootka structure. Use generously.
- **Arched & circular accent.** Echo the barrel form as a subtle, deliberate
  accent on one or a few hero elements — a single circular hero image or one
  arched section top — so it reads as a signature moment, not blanket
  roundness. Default to tighter radii and flatter UI: modest rounding on
  buttons and cards, with the pill/circle shape reserved for that intentional
  hero accent.
- **Type as hierarchy.** DM Sans Bold headlines with a large size jump over
  regular body. Sentence case. Confident, sensory, place-anchored microcopy
  ("From the forested shores of the Pacific"). No hype, no exclamation marks.
- **Heat orange, sparingly.** Primary CTAs, links, focus rings, small
  highlights only. Never an orange surface or background.
- **Cinematic imagery.** Natural light, muted earthy tones, real PNW
  environments (cedar, fog, coast, fire). Full-bleed against dark blocks. If
  no real assets exist, use neutral placeholders sized for these slots — do
  not fabricate the logo. Check `~/Documents/logo` for the real logo asset.

## Hard constraints (flag conflicts, never violate silently)

- **DM Sans only.** No second font. (Monospace in the PDF is doc chrome.)
- **Logo:** look in `~/Documents/logo` for the logo asset and use only the
  supplied stacked or horizontal file with proper clear space. Do **not**
  recolor, redraw, rotate, rearrange, or typeset the wordmark in DM Sans as a
  substitute. If that directory has no usable asset, leave a clearly marked
  placeholder and ask for it.
- **Palette is exact.** Only the five brand hexes (+ white). Heat is an accent,
  never a surface.
- **Contrast:** maintain WCAG AA. Never Heat text on Steam; check Steam-on-Moss.

## Palette (quick reference)

| Token | Hex | Role |
| --- | --- | --- |
| Steam | `#CDD8DA` | light neutral bg / surfaces |
| Coastal Water | `#073D44` | dark teal — secondary dark |
| Reindeer Moss | `#919A3C` | olive — supporting blocks/tags |
| Evergreen | `#324126` | deep forest — primary dark/sections |
| Heat | `#FF821B` | orange — accent only |

Full tokens, semantic aliases, and font setup: [brand-tokens.css](brand-tokens.css).

## Pre-finish checklist

- [ ] `DESIGN_GUIDELINES.md` was read this run and every rule in it honored
- [ ] Layout genuinely restructured (dark/light blocks, splits, arches) — not just recolored
- [ ] Every color/font/radius is a semantic token; zero off-palette hex
- [ ] DM Sans Bold headlines + Regular body, sentence case, strong size jump
- [ ] Heat used only as accent (CTAs/links/focus/highlights), no orange surfaces
- [ ] Arched/circular motif used as a deliberate, restrained accent on a hero element, not applied pervasively
- [ ] Logo rules respected (real SVG or marked placeholder; no substitute wordmark)
- [ ] WCAG AA contrast holds on every text/background pair
- [ ] Copy is calm, declarative, place-specific; no hype or exclamation marks
- [ ] Any guideline conflict surfaced to the user, not silently resolved
