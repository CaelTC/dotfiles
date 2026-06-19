# claude-dash

A live terminal dashboard (btop-style, written in Rust) for watching consumption of a Claude subscription across several concurrent Claude Code sessions on this machine. The headline numbers are Anthropic's own rate-limit figures, captured from response headers by a local proxy — the same data the website's Usage page shows.

## Language

**Usage**:
Consumption of the Claude subscription's metered allowance. Has two facets — **Budget** and **Throughput** — which must not be conflated.
_Avoid_: "stats", "activity" (too vague)

**Budget**:
The account-wide facet of **Usage**: how much of each **Rolling Window** is consumed and when it resets. Captured live by the **Proxy** from Anthropic's `anthropic-ratelimit-unified-*` response headers, which give **Utilization** directly as a 0–1 fraction per window plus reset times — authoritative, not an estimate. Account-scoped, so any **Active Session**'s latest reading reflects the whole subscription (all machines and web chat included).
_Avoid_: "limit", "quota", "estimate"

**Utilization**:
The fraction (0–1) of a **Rolling Window**'s allowance already consumed, as reported by Anthropic. The raw material of **Budget** — no denominator is assumed because the fraction is given outright.

**Throughput**:
The per-**Session** facet of **Usage**: tokens (input / output / cache-read / cache-creation) per **Model**, captured live by the **Proxy** from response bodies. Shown as a rolling-window rate (tokens completed in the last ~60s), not an instantaneous spike. Exact.

**Session**:
One `claude` run launched under **cca** — the unit the dashboard tracks. Identified by a **Session id** minted by **cca** (the store key), and labelled `project · model · id` (e.g. `nootka-kiosk · opus-4.8 · a3f1`). **Active** while its **Proxy** is attached; moves to **Session History** when it ends. Backed by one **Transcript**.
_Avoid_: "instance" (retired — use Session)

**Active Session**:
A **Session** whose **Proxy** is currently attached. Shown in the active box, with live **Throughput** and a `live`/`idle` indicator (a request completed recently vs quiet).

**Session History**:
The set of ended **Session**s, kept after their **Proxy** tears down. Filterable (default view: the last 10).

**cca**:
The thin zsh wrapper invoked in place of `claude` (evolved from the `claude --permission auto` alias). For its **Session** it stands up a local **Proxy** via `ANTHROPIC_BASE_URL`, then runs `claude` through it. The component that *captures*; **claude-dash** only *reads*.

**Proxy**:
The local streaming reverse-proxy `cca` places between a **Session** and Anthropic's API. Relays the response body untouched (so the `claude` TUI stays live) while reading **Budget** from response headers and **Throughput** from a tee of the body. Token refresh bypasses it (only inference is redirected). Writes tagged records to the shared store **claude-dash** reads.

**Transcript**:
The append-only `.jsonl` Claude Code writes per **Session** under `~/.claude/projects/...`. The persisted conversation record; carries no rate-limit data, so it cannot inform **Budget**. Source for backfilling **Session History** from before **cca** was capturing.

**Rolling Window**:
A time interval over which the subscription meters allowance. Two exist, metered separately: the **5-hour window** and the **7-day window**. Each reports its own **Utilization**, status, and reset time. The **Representative Window** is the one Anthropic currently flags as binding.

**Representative Window**:
Whichever **Rolling Window** is the active constraint right now (`representative-claim` in the headers, e.g. `five_hour`). The window the dashboard headlines.

**Model**:
The Claude model that served a request (e.g. `claude-opus-4-8`). **Throughput** breaks down per **Model**.

## Relationships

- **cca** runs one **Session** behind one **Proxy**; many **Session**s run at once, each with its own **Proxy**, all writing to one shared store
- A **Proxy** captures **Budget** (account-wide, from `anthropic-ratelimit-unified-*` headers) and **Throughput** (per-**Session**, from response bodies)
- **Budget** spans two **Rolling Window**s (5-hour, 7-day), each with its own **Utilization** and reset; the **Representative Window** is the binding one
- A **Session** is **Active** while its **Proxy** is attached, then enters **Session History**
- **claude-dash** reads the shared store and renders: one **Budget** (left rail), the **Active Session**s (live throughput panels), and **Session History**
- An **Active Session** has one **Transcript**
- **Usage** = **Budget** (authoritative, account-wide) + **Throughput** (exact, per-**Session**)

## Example dialogue

> **Dev:** "If I've got three claude sessions running, which one's budget does the dashboard show?"
> **Domain expert:** "Budget isn't per-session — it's the whole account. Each session's **Proxy** reads the same account-wide `unified` rate-limit headers, so the dashboard shows one Budget and takes the freshest reading. What's per-session is **Throughput** — how fast *that* session is burning tokens."

## Flagged ambiguities

- "usage" meant both token throughput and remaining-subscription-budget — resolved: **Throughput** (per-**Session**, exact) and **Budget** (account-wide, authoritative from captured headers).
- "which session's budget" — resolved: **Budget** is account-scoped, never per-**Session**; only **Throughput** is per-**Session**.
- "instance" vs "session" — resolved: one unit, the **Session**; **Active** while running, then **Session History**. "Instance" retired.
- We expected **Budget** to need a guessed ceiling — resolved (spike, 2026-06-19): the headers report **Utilization** as a fraction directly, so Budget is a reading, not an estimate.
