---
name: herdr
description: Use whenever a task touches Herdr, the Herdr desktop app (not the older `herdr` terminal multiplexer). Triggers - changing what the person sees (spaces, tabs, panes, focus, layouts); customizing Herdr (mods, programs.json, keys, settings, themes in ~/.config/herdr-app); or starting, reading, messaging, waiting on, or cleaning up other agents and long-running programs with the `hc` (herdr-core) CLI.
---

# Herdr

Herdr is the desktop app for terminals and coding agents. Its agent manuals are printed by the tools themselves, so this skill only says which manual to read. Do not rely on memory for Herdr commands.

- **Window, layout, and customization** (spaces, tabs, panes, focus, mods, settings): run `herdr-app --skill` first and follow the printed manual.
- **Agents and programs** (start, read, message, wait on, clean up): run `hc --skill` first and follow the printed manual.

Before customizing anything in `~/.config/herdr-app`, read `~/.config/herdr-app/AGENTS.md`.

Never stop or restart the Herdr app or herdr-core. Other work may be running in them.
