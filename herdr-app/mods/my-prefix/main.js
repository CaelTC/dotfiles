// The herdr TUI's prefix keys (~/.config/herdr/config.toml: prefix = ctrl+a)
// in the Herdr app, which has no key sequences of its own. keys.json binds
// ctrl-a to `mod.open:my-prefix`; this popup takes the next key, runs its
// command and closes. ctrl-a again sends a literal ctrl-a to the terminal,
// Esc (or any unbound key) cancels.

import { View, div } from "gpui-kit";
import { v_flex, h_flex } from "gpui-base";
import { run, hc, props, closeOverlay, focused, openOverlay, theme } from "herdr";

const go = (name, args = {}) => () => run(name, args);

// Same keys as herdr's TUI defaults. TUI actions with no app command
// (detach, reload config, rename pane, edit scrollback, resize mode, pane
// cycling) are left out.
const KEYS = {
  "?": ["help", go("app.help")],
  s: ["settings", go("app.settings")],
  w: ["spaces", go("mod.open", { name: "picker" })],
  g: ["goto", go("mod.open", { name: "picker" })],
  N: ["new space", go("space.new")],
  G: ["new worktree", () => { const f = focused(); if (f) openOverlay({ mod: "worktree", kind: "dialog", props: { space: f.space, action: "new" } }); }],
  W: ["rename space", go("space.rename")],
  D: ["close space", go("space.close")],
  c: ["new tab", go("tab.new")],
  T: ["rename tab", go("tab.rename")],
  p: ["prev tab", go("tab.prev")],
  n: ["next tab", go("tab.next")],
  X: ["close tab", go("tab.close")],
  h: ["←", go("pane.go", { direction: "left" })],
  j: ["↓", go("pane.go", { direction: "down" })],
  k: ["↑", go("pane.go", { direction: "up" })],
  l: ["→", go("pane.go", { direction: "right" })],
  v: ["split right", go("pane.split", { direction: "right" })],
  "-": ["split down", go("pane.split", { direction: "down" })],
  x: ["close pane", go("pane.close")],
  z: ["zoom", go("pane.zoom")],
  b: ["sidebar", go("layout.toggle", { id: "sidebar" })],
};
for (let i = 1; i <= 9; i++) KEYS[String(i)] = [`tab ${i}`, go("tab.select", { index: i })];

export default class Prefix extends View {
  init(_props, cx) {
    this.handle = cx.focus_handle();
    this.handle.focus();
  }

  press(e) {
    closeOverlay(props().overlay);
    if (e.keystroke === "ctrl-a") {
      const t = focused()?.terminal;
      if (t) hc(["keys", t, "ctrl+a"]);
      return;
    }
    const ch = e.modifiers?.control || e.modifiers?.alt || e.modifiers?.platform ? null : (e.key_char ?? e.key);
    KEYS[ch]?.[1]();
  }

  render() {
    const t = theme();
    const hint = (k, label) => h_flex().gap(4).child(div().px(5).rounded(4).bg(t.raised).text_color(t.foreground).child(k)).child(div().text_color(t.muted).child(label));
    const shown = Object.entries(KEYS).filter(([k]) => !/^[2-9]$/.test(k)).map(([k, [label]]) => (k === "1" ? hint("1-9", "tab") : hint(k, label)));
    return v_flex()
      .id("my-prefix")
      .track_focus(this.handle)
      .on_key_down((e, cx) => {
        cx.stop_propagation();
        if (["shift", "control", "alt", "platform", "function"].includes(e.key)) return;
        this.press(e);
      })
      .size_full()
      .p(12)
      .gap(8)
      .rounded(8)
      .bg(t.popover)
      .text_size(11)
      .child(h_flex().child(div().flex_1().font_semibold().text_size(13).text_color(t.accent).child("ctrl-a")).child(div().text_color(t.dim).child("Esc cancels · ctrl-a sends ctrl-a")))
      .child(h_flex().flex_wrap().gap_x(12).gap_y(4).children(shown));
  }
}
