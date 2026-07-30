# claude-analyzer

A local, self-serve token-usage analyzer for Claude Code. A pure **Reader**: it
parses the **Transcript**s Claude Code already writes and renders a usage
breakdown for the person whose machine it runs on. No proxy, no traffic
interception, no central collection — each colleague checks their own numbers.

Sibling project: `claude-dash` (live Budget/Throughput TUI, proxy-based, this
repo). The two share the **Transcript** concept but answer different questions:
claude-dash answers "how much subscription is left right now"; claude-analyzer
answers "where did my tokens go".

## Language

**Transcript**:
The append-only `.jsonl` file Claude Code writes per session under
`~/.claude/projects/...` on every platform. The sole data source for
claude-analyzer. Each assistant message in it records the API request's token
`usage`; message content identifies prompts, tool calls, and MCP activity.
_Avoid_: "traffic", "logs" (nothing is captured off the wire)

**Reader**:
claude-analyzer's role: it only ever reads **Transcript**s. It never wraps,
launches, or proxies Claude Code, and works retroactively on transcripts that
existed before it was installed.

**Request**:
One API call recorded as one assistant message in a **Transcript**, carrying
the authoritative token `usage` figures. The atomic unit of accounting: every
token claude-analyzer reports comes from a Request's usage block, never from
estimation.

**Category**:
The single bucket a **Request** is attributed to. Five exist: **Prompt**
(fresh user text), **Tool call** (a tool_result from a built-in tool), **MCP**
(a tool_result from an `mcp__*` tool), **Skill** (a skill invocation), and
**Agent** (any request made by a subagent/sidechain rather than the main
conversation). Non-Agent requests are categorized by trigger attribution —
what was newly added to context immediately before them. Every Request lands
in exactly one Category; when parallel tool results of mixed kinds precede a
request, priority is MCP > Skill > Tool call. Background housekeeping calls
(e.g. title generation) fall into a residual **Overhead** slice.
_Avoid_: "source", "type"

**Agent** (Category):
All **Request**s issued by a subagent/sidechain. Never folded into the
triggering Category — agent fan-out must be visible, since it is often the
largest consumer. Agent usage is always shown *within the session that
spawned it*, broken down per agent, so the per-session view reads as one
story: prompt → agents → total.

**Fresh tokens**:
A **Request**'s `input + cache_creation + output` tokens — what was newly
processed at full price. The number attributed to a **Category**.

**Grouping**:
One of five pivots of the same **Week** of **Request**s, switchable inside the
**Report**: **Session**, **Category**, **Day**, **Skill**, or **Model**.
Single-level only — groupings never compose. Each Grouping independently
accounts for 100% of the Week's tokens (with a residual bucket like
"(no skill)" where a Request matches nothing). Two Groupings have one fixed
drill-down inside a row: a Session expands into its agents, and the MCP
**Category** row expands into MCP servers.

**MCP server**:
The `<server>` segment of an `mcp__<server>__<tool>` tool name (e.g.
`playwright`). The MCP **Category** breaks down per MCP server, so a hungry
server is identifiable, not just "MCP" in aggregate.

**Model**:
The Claude model that served a **Request** (`claude-opus-4-8`,
`claude-fable-5`, …), recorded verbatim on every Request. One of the five
**Grouping**s.

**Day**:
A calendar day (local time) within the **Week**. The oldest Day is partial —
the Week starts mid-day seven days ago — and is labelled as such rather than
hidden.

**Skill run**:
The unit of the Skill **Grouping**: every **Request** — including **Agent**
requests it sets in motion — attributed to one invocation of a skill.
Main-conversation attribution is authoritative, not heuristic: Claude Code
stamps each assistant record with the active skill (`attributionSkill`), and
consecutive same-skill spans form one run. Agent requests join the run of
the skill active when they were spawned. Measures what invoking the skill
actually costs, not how large its instructions are. A skill's row aggregates
its runs: count, average per run, total, and composition (how much of it was
agents vs tools).

**Report**:
The artifact a colleague actually looks at: a one-shot, self-contained
rendering of one **Week**'s breakdown — overall **Category** shares plus the
per-session split — regenerated on each run. Static, not live: it shows the
Week as of the moment it was generated. Aimed at non-power-users, so it must
explain itself without a manual.

**Week**:
The default reporting window: the trailing 7 days ending now. An approximation
of Anthropic's 7-day rolling limit chosen deliberately — the exact billing
window's reset time only exists in rate-limit headers, which **Transcript**s
do not carry. Not a calendar week.
_Avoid_: "billing window" (we can't see it), "calendar week"

**Previous Week**:
The trailing window immediately before the **Week**: 14 to 7 days ago,
anchored to today — never the last full calendar week. Exists only for
comparison (usage shifts fast, model mix especially), so it carries Fresh
totals per row, not its own full breakdown. Sessions are not compared —
a session belongs to one window.
_Avoid_: "last week" (calendar connotation)

**Cache reads**:
A **Request**'s `cache_read` tokens — context replayed from cache at ~10%
cost. Reported alongside but never blended into **Fresh tokens**, so a
Category's share reflects what it actually costs.

## Relationships

- One **Transcript** per Claude Code session; the **Reader** consumes many
  **Transcript**s across all projects on the machine.

## Flagged ambiguities

- "study all traffic" (initial pitch) — resolved: no traffic is studied;
  transcripts on disk carry everything required. See ADR (pending).
- "token usage by MCP / skills / chat input" — resolved: tokens are billed
  per-request over the whole context, so per-component usage is an
  attribution model (trigger attribution for **Category**, `attributionSkill`
  spans for **Skill run**), not a raw number.
- "skill run boundary" — we expected to need an invocation→next-prompt
  heuristic; resolved (spike, 2026-07-29): transcripts already carry
  authoritative per-request skill attribution.
