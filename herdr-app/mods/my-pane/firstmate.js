// Firstmate's pirate flag: which terminals are a machine's firstmate primary, and the
// flag "program" to draw for them instead of Claude's icon. Minions (terminals the
// `minion` CLI starts, labelled minion=<name>) get the minion picture and their own name.
//
// A primary is a live agent whose own process (the terminal's foreground) runs in a
// firstmate home root: a folder with AGENTS.md, bin/fm-session-start.sh and state/.
// Crewmates run in worktrees, and treehouse worktrees of the firstmate repo have no
// state/ (it is gitignored), so they never match. The terminal's top-level cwd is only
// its shell's; the foreground cwd is the agent's.
//
// The flag's color is the machine's: the `firstmate` entry install.sh writes into that
// machine's programs.json (herdr-app/gen-programs.sh).
//
// Identical copies live in my-agents, my-tabs and my-pane: a mod may only import files
// inside its own folder (a symlink out of it is refused). herdr-app/check-mods.sh
// checks they stay identical.

import { svg, image } from "gpui-kit";
import { terminals, exec, readTextFile, appHome } from "herdr";

const FLAG = "my-icons/firstmate-flag.svg";
const MINION = "my-icons/minion.svg";
// A folder that wasn't a firstmate home is asked about again after this long.
const RECHECK_MS = 60000;

const machineOf = (t) => t.machine ?? (t.address.includes("/") ? t.address.split("/")[0] : null);

export class FirstmateFlag {
  constructor() {
    this.homes = new Map();  // `${machine}\0${cwd}` -> { yes, at, busy }
    this.colors = new Map(); // machine ("" here) -> { color } once read
  }

  // address -> flag program for every firstmate primary (and minion program for every
  // minion). Unknown folders and colors are looked up in the background; the copy redraws
  // once they are known.
  scan(cx) {
    const out = new Map();
    for (const t of terminals()) {
      const minion = t.labels?.minion;
      if (minion && t.state === "live") {
        out.set(t.address, {
          ...t.program, key: "minion", name: minion, icon: MINION, tint: false,
          tile: null, color: null, minion: true,
        });
        continue;
      }
      const cwd = t.foreground?.cwd;
      // Sub-agents (started with hc run) are helpers, not the primary.
      if (t.state !== "live" || !t.program?.agent || t.parent || !cwd) continue;
      const machine = machineOf(t);
      if (!this.isHome(machine, cwd, cx)) continue;
      const color = this.color(machine, cx);
      if (color === undefined) continue;
      out.set(t.address, {
        ...t.program, key: "firstmate", name: "Firstmate", icon: FLAG, tint: true,
        tile: null, color, flag: true,
      });
    }
    return out;
  }

  isHome(machine, cwd, cx) {
    const key = `${machine ?? ""}\0${cwd}`;
    const known = this.homes.get(key);
    if (known?.yes) return true;
    if (!known || (!known.busy && Date.now() - known.at > RECHECK_MS)) {
      this.homes.set(key, { yes: false, at: Date.now(), busy: true });
      cx.spawn(async (cx) => {
        let yes = false;
        try {
          const r = await exec("test", ["-f", `${cwd}/AGENTS.md`, "-a", "-f", `${cwd}/bin/fm-session-start.sh`, "-a", "-d", `${cwd}/state`],
            { on: machine, timeout: 5000 });
          yes = r.code === 0;
        } catch { /* unreachable machine: not known as a home */ }
        this.homes.set(key, { yes, at: Date.now(), busy: false });
        if (yes) cx.notify();
      });
    }
    return false;
  }

  // The machine's flag color: a color string, null (no entry: the text color), or
  // undefined while it is being read.
  color(machine, cx) {
    const key = machine ?? "";
    const known = this.colors.get(key);
    if (known) return known.color;
    if (known === undefined) {
      this.colors.set(key, null);
      cx.spawn(async (cx) => {
        let color = null;
        try {
          const text = machine
            ? (await exec("sh", ["-c", 'cat "${HERDR_APP_HOME:-$HOME/.config/herdr-app}/programs.json"'], { on: machine, timeout: 5000 })).stdout
            : await readTextFile(`${appHome()}/programs.json`);
          color = JSON.parse(text ?? "{}")?.programs?.firstmate?.color ?? null;
        } catch { /* no programs.json there: the text color */ }
        this.colors.set(key, { color });
        cx.notify();
      });
    }
    return undefined;
  }
}

// A program's icon: a one-color glyph (the flag in its own color) or a picture.
export function programIcon(p, size, color) {
  if (!p?.icon) return null;
  return p.tint ? svg(p.icon).size(size).flex_none().text_color(p.flag && p.color ? p.color : color) : image(p.icon).w(size).h(size).flex_none();
}
