// Minion inbox: messages minions on other machines sent to Cael's seat, waiting for approval.
// The gate (`minion gate serve`) writes one JSON file per pending message under
// $XDG_STATE_HOME/minions/gate/inbox/ and removes it once handled. This mod watches that folder.
//
// Two faces, one module:
// - in the layout (the status bar): a small badge with the pending count; it posts a
//   notification for each new message, and opens the inbox when clicked;
// - as an overlay (`mod.open minion-inbox`): the list, with Allow / target picker /
//   always-allow / Deny, each running `minion inbox allow|deny`.

import { View, div, svg } from "gpui-kit";
import { v_flex, h_flex } from "gpui-base";
import { exec, home, theme, run, notify, props, terminals, watchFolder, readTextFile } from "herdr";
import * as fs from "fs/promises";

const ICON = "mods/minion-inbox/mail.svg";
const minion = () => `${home()}/.cargo/bin/minion`;

async function inboxDir() {
  const r = await exec("sh", ["-c", 'd="${XDG_STATE_HOME:-$HOME/.local/state}/minions/gate/inbox"; mkdir -p "$d" && printf %s "$d"']);
  if (r.code !== 0 || !r.stdout) throw new Error(r.stderr || "cannot make the inbox folder");
  return r.stdout;
}

async function readInbox(dir) {
  const names = (await fs.readdir(dir).catch(() => [])).filter((n) => n.endsWith(".json"));
  const msgs = [];
  for (const n of names) {
    const text = await readTextFile(`${dir}/${n}`).catch(() => null);
    if (!text) continue;
    try {
      const m = JSON.parse(text);
      if (m && typeof m.id === "string") msgs.push(m);
    } catch (_) {
      // A half-written file: the gate renames into place, so this is someone else's; skip it.
    }
  }
  return msgs.sort((a, b) => (a.received ?? 0) - (b.received ?? 0));
}

// Local live agents, as `minion gate` counts them: bare address (not machine/tN), live, and
// either reporting a status or running something other than a plain shell. `extra` holds the
// addresses `minion agents` adds (Claude started with cca shows as bash in terminals()).
const SHELLS = ["bash", "zsh", "sh", "dash", "fish", "ksh", "nu", "tcsh"];
function liveAgents(extra = new Set()) {
  return terminals()
    .filter((t) => t.address && !t.address.includes("/") && (t.state ?? "live") === "live")
    .filter((t) => extra.has(t.address) || t.status || (t.agent && !SHELLS.includes(t.agent)))
    .map((t) => ({
      address: t.address,
      label: t.name || t.where || (t.cwd ?? "").split("/").filter(Boolean).pop() || t.address,
      status: t.status,
    }));
}

// The gate's own list of agents on this seat (the same rule minions see).
async function gateAgents() {
  try {
    const r = await exec(home() + "/.cargo/bin/minion", ["agents", "--json"]);
    if (r.code !== 0) return new Set();
    return new Set(JSON.parse(r.stdout).map((a) => a.address));
  } catch (_) {
    return new Set();
  }
}

function age(received) {
  if (!received) return "";
  const s = Math.max(0, Math.floor(Date.now() / 1000 - received));
  if (s < 60) return "just now";
  if (s < 3600) return `${Math.floor(s / 60)}m ago`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ago`;
  return `${Math.floor(s / 86400)}d ago`;
}

const short = (text, n) => {
  const one = (text ?? "").replace(/\s+/g, " ").trim();
  return one.length > n ? one.slice(0, n - 1) + "…" : one;
};

export default class MinionInbox extends View {
  init(_props, cx) {
    this.overlay = props()?.overlay ?? null;
    this.msgs = [];
    this.dir = null;
    this.error = null;
    this.seen = null; // ids already announced (badge only); null until the first read
    this.pick = {}; // message id -> chosen target address
    this.always = {}; // message id -> bool
    this.busy = {}; // message id -> "allow" | "deny"
    this.failed = {}; // message id -> stderr of the last failed action
    this.extra = new Set(); // agent addresses only `minion agents` knows about
    this.open = {}; // message id -> showing the full message
    cx.spawn(async (cx) => {
      try {
        this.dir = await inboxDir();
      } catch (e) {
        this.error = String(e?.message ?? e);
        cx.notify();
        return;
      }
      await this.reload(cx);
      for (;;) {
        try {
          await watchFolder(this.dir);
        } catch (_) {
          // The watch can fail (folder removed, too many folders): fall back to a slow re-read.
          await new Promise((resolve) => cx.timer.after(10000, () => resolve()));
        }
        await this.reload(cx);
      }
    });
    // Ages only; the list itself comes from the folder watch.
    if (this.overlay) cx.timer.every(30000, (cx) => cx.notify());
  }

  async reload(cx) {
    this.msgs = await readInbox(this.dir);
    if (this.overlay) this.extra = await gateAgents();
    const ids = new Set(this.msgs.map((m) => m.id));
    for (const k of Object.keys(this.busy)) if (!ids.has(k)) delete this.busy[k];
    if (!this.overlay) this.announce();
    cx.notify();
  }

  // Badge only: one notification per new message (a summary when many arrive at once).
  announce() {
    const fresh = this.seen === null ? [] : this.msgs.filter((m) => !this.seen.has(m.id));
    const first = this.seen === null;
    this.seen = new Set(this.msgs.map((m) => m.id));
    const open = { name: "mod.open", args: { name: "minion-inbox" } };
    if (first && this.msgs.length > 0) {
      notify({ title: "Minion messages", body: `${this.msgs.length} waiting for you`, level: "info", icon: ICON, action: open });
      return;
    }
    if (fresh.length > 3) {
      notify({ title: "Minion messages", body: `${fresh.length} new messages waiting`, level: "info", icon: ICON, action: open });
      return;
    }
    for (const m of fresh) {
      notify({
        title: "Minion message",
        body: `from ${m.from} → ${m.to ?? "Cael"}: ${m.project ? `[${m.project}] ` : ""}${short(m.goal || m.text, 80)}`,
        level: "info",
        icon: ICON,
        action: open,
      });
    }
  }

  target(m, agents) {
    const picked = this.pick[m.id];
    if (picked && agents.some((a) => a.address === picked)) return picked;
    if (m.to && agents.some((a) => a.address === m.to)) return m.to;
    return null;
  }

  act(m, action, target, cx) {
    const args = action === "allow" ? ["inbox", "allow", m.id] : ["inbox", "deny", m.id];
    if (action === "allow") {
      if (target) args.push("--to", target);
      if (this.always[m.id]) args.push("--always");
    }
    this.busy[m.id] = action;
    delete this.failed[m.id];
    cx.notify();
    cx.spawn(async (cx) => {
      let r;
      try {
        r = await exec(minion(), args, { timeout: 60000 });
      } catch (e) {
        r = { code: 1, stdout: "", stderr: String(e?.message ?? e) };
      }
      delete this.busy[m.id];
      if (r.code !== 0) {
        const why = (r.stderr || r.stdout || `exit ${r.code}`).trim();
        this.failed[m.id] = why;
        notify({ title: `Could not ${action} ${m.from}'s message`, body: why, level: "error" });
      }
      await this.reload(cx);
    });
  }

  // ---- badge ----------------------------------------------------------------

  renderBadge() {
    const t = theme();
    const n = this.msgs.length;
    return h_flex().id("minion-inbox-badge").size_full().px(12).gap(6).items_center().justify_end()
      .bg(t.chrome).border_t(1).border_color(t.border).text_size(11.8)
      .text_color(n > 0 ? t.accent : t.dim)
      .hover((s) => s.text_color(n > 0 ? t.accent : t.muted))
      .on_click(() => run("mod.open", { name: "minion-inbox" }))
      .child(svg(ICON).size(13))
      .when(n > 0, (el) => el.child(div().font_semibold().child(`${n}`)));
  }

  // ---- overlay --------------------------------------------------------------

  button(id, label, colors, onClick) {
    return div().id(id).px(12).py(5).rounded(6).text_size(12.5).font_semibold()
      .bg(colors.bg).text_color(colors.fg).border(1).border_color(colors.border ?? colors.bg)
      .hover((s) => s.opacity(0.85))
      .on_click((_e, cx) => onClick(cx))
      .child(label);
  }

  renderMessage(m, agents) {
    const t = theme();
    const target = this.target(m, agents);
    const busy = this.busy[m.id];
    const gone = m.to && !agents.some((a) => a.address === m.to);
    const kindColor = m.kind === "REQUEST" ? t.working : m.kind === "REPLY" ? t.done : t.muted;

    const chip = (a) => {
      const on = a.address === target;
      return div().id(`pick-${m.id}-${a.address}`).px(8).py(3).rounded(5).text_size(12)
        .border(1).border_color(on ? t.accent : t.border)
        .bg(on ? t.selected : t.surface).text_color(on ? t.foreground : t.muted)
        .hover((s) => s.bg(t.hover))
        .on_click((_e, cx) => { this.pick[m.id] = a.address; cx.notify(); })
        .child(h_flex().gap(6).items_center()
          .child(div().size(6).rounded(3).bg(t[a.status] ?? t.unknown))
          .child(`${a.label}`)
          .child(div().text_color(t.dim).child(a.address)));
    };

    const always = Boolean(this.always[m.id]);
    return v_flex().id(`msg-${m.id}`).gap(10).p(14).rounded(8).bg(t.surface).border(1).border_color(t.border)
      .child(h_flex().gap(8).items_center().text_size(12.5)
        .child(div().px(6).py(1).rounded(4).text_size(11).font_semibold().text_color(kindColor).border(1).border_color(kindColor).child(m.kind ?? "MSG"))
        .child(div().font_semibold().text_color(t.foreground).child(m.from ?? "?"))
        .child(div().text_color(t.dim).child(m.machine ? `on ${m.machine}` : ""))
        .child(div().text_color(t.dim).child("→"))
        .child(div().text_color(gone ? t.blocked : t.muted).child(m.to ? (gone ? `${m.to} (gone)` : m.to) : "Cael"))
        .child(div().flex_1())
        .child(div().text_color(t.dim).child(`#${m.msg_id ?? ""}  ${age(m.received)}`)))
      .child(this.renderBody(m))
      .child(v_flex().gap(6)
        .child(div().text_size(11.5).text_color(t.dim).child(agents.length ? "Deliver to" : "No live agents on this computer to deliver to"))
        .child(h_flex().flex_wrap().gap(6).children(agents.map(chip))))
      .child(h_flex().gap(8).items_center()
        .child(h_flex().id(`always-${m.id}`).gap(6).items_center().text_size(12).text_color(always ? t.foreground : t.muted)
          .on_click((_e, cx) => { this.always[m.id] = !always; cx.notify(); })
          .child(div().size(14).rounded(3).border(1).border_color(always ? t.accent : t.input).bg(always ? t.accent : t.surface)
            .text_color(t.accent_foreground).text_size(10).items_center().justify_center().flex().child(always ? "✓" : ""))
          .child(target ? `Always allow ${m.from} → ${target}` : `Always allow ${m.from} → this agent`))
        .child(div().flex_1())
        .when(Boolean(busy), (el) => el.child(div().text_size(12).text_color(t.dim).child(busy === "allow" ? "Delivering…" : "Denying…")))
        .when(!busy, (el) => el
          .child(this.button(`deny-${m.id}`, "Deny", { bg: t.surface, fg: t.blocked, border: t.border }, (cx) => this.act(m, "deny", null, cx)))
          .child(target
            ? this.button(`allow-${m.id}`, `Allow → ${agents.find((a) => a.address === target)?.label ?? target}`, { bg: t.solid, fg: t.solid_foreground }, (cx) => this.act(m, "allow", target, cx))
            : div().px(12).py(5).rounded(6).text_size(12.5).text_color(t.dim).border(1).border_color(t.border).child("Pick an agent"))))
      .when(Boolean(this.failed[m.id]), (el) => el.child(div().text_size(12).text_color(t.blocked).child(this.failed[m.id])));
  }

  // Project, Goal and Solution at a glance; Open shows everything, details included.
  renderBody(m) {
    const t = theme();
    const open = Boolean(this.open[m.id]);
    const brief = Boolean(m.project || m.goal || m.solution);
    const row = (label, value, max) => h_flex().w_full().gap(10).items_start().text_size(13)
      .child(div().w(64).flex_none().text_size(11.5).pt(1).text_color(t.dim).child(label))
      .child(div().flex_1().min_w(0).text_color(t.foreground).child(open || !max ? value : short(value, max)));
    const hasMore = brief
      ? Boolean(m.text) || (m.goal ?? "").length > 190 || (m.solution ?? "").length > 95
      : (m.text ?? "").length > 190;
    return v_flex().w_full().gap(6)
      .when(brief, (el) => el
        .when(Boolean(m.project), (el) => el.child(row("Project", m.project, 0)))
        .when(Boolean(m.goal), (el) => el.child(row("Goal", m.goal, 190)))
        .when(Boolean(m.solution), (el) => el.child(row("Solution", m.solution, 95)))
        .when(open && Boolean(m.text), (el) => el.child(row("Details", m.text, 0))))
      .when(!brief, (el) => el.child(div().w_full().text_size(13).text_color(t.foreground)
        .child(open ? (m.text ?? "") : short(m.text ?? "", 190))))
      .when(hasMore, (el) => el.child(h_flex().id(`open-${m.id}`).gap(4).items_center().text_size(12)
        .text_color(t.accent).hover((s) => s.text_color(t.foreground))
        .on_click((_e, cx) => { this.open[m.id] = !open; cx.notify(); })
        .child(open ? "▾ Close" : (brief && m.text ? "▸ Open full message (details)" : "▸ Open full message"))));
  }

  renderInbox() {
    const t = theme();
    const agents = liveAgents(this.extra);
    const n = this.msgs.length;
    return v_flex().id("minion-inbox").size_full().bg(t.background).text_color(t.foreground)
      .child(h_flex().px(18).py(14).gap(10).items_center().border_b(1).border_color(t.border)
        .child(svg(ICON).size(16).text_color(t.accent))
        .child(div().text_size(15).font_semibold().child("Minion inbox"))
        .child(div().text_size(12.5).text_color(t.dim).child(n ? `${n} waiting` : ""))
        .child(div().flex_1())
        .child(div().id("close").px(6).rounded(4).text_color(t.dim).hover((s) => s.text_color(t.foreground))
          .on_click(() => run("overlay.close", { id: this.overlay }))
          .child(svg("mods/workspace-chrome/x.svg").size(14))))
      .child(v_flex().id("list").flex_1().p(14).gap(10).overflow_y_scrollbar()
        .when(Boolean(this.error), (el) => el.child(div().text_color(t.blocked).child(this.error)))
        .when(!this.error && n === 0, (el) => el.child(v_flex().flex_1().items_center().justify_center().gap(6).py(40)
          .child(div().text_size(13.5).text_color(t.muted).child("No messages waiting"))
          .child(div().text_size(12).text_color(t.dim).child("Minions' messages to this computer show up here for you to allow or deny."))))
        .children(this.msgs.map((m) => this.renderMessage(m, agents))));
  }

  render() {
    return this.overlay ? this.renderInbox() : this.renderBadge();
  }
}
