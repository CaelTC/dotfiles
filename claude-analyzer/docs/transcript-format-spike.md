# Spike: Claude Code transcript format (verified 2026-07-29, real corpus)

Verified against ~/.claude/projects on macOS, Claude Code writing `version`-stamped
records. 64 project dirs, 135 files active in trailing 7 days, 226MB.

## Layout

- Main sessions: `~/.claude/projects/<flattened-project-path>/<sessionId>.jsonl`
- Subagents: `~/.claude/projects/<project>/<sessionId>/subagents/agent-<id>.jsonl`
  — parent session = the `<sessionId>` directory name. Subagent assistant
  records have `isSidechain: true`, their own real `usage`, own `model`
  (often haiku/sonnet), and `attributionSkill: null` even when a skill
  spawned them.

## Record types observed (one file's distribution)

`assistant`, `user`, plus non-billable: `ai-title`, `attachment`,
`file-history-snapshot`, `last-prompt`, `mode`, `system`. Only `assistant`
records carry `message.usage`. Parse leniently: skip unknown types, never crash.

## Critical: deduplication

One API response is written as **multiple assistant records** (one per content
block), each repeating the identical `message.id`, `requestId`, and full
`usage`. Observed dup factor 3–4×. **Count each unique `message.id` once.**
Dedupe across files too (session resume/compaction can copy history).

## Assistant record fields that matter

- `message.usage`: `input_tokens`, `cache_creation_input_tokens`,
  `cache_read_input_tokens`, `output_tokens` (ignore the rest:
  `iterations`, `server_tool_use`, `cache_creation` detail, …)
- `message.model`: e.g. `claude-fable-5`, `claude-opus-4-8`; **`<synthetic>`
  appears** — skip records with synthetic/missing model or missing usage.
- `attributionSkill`: authoritative skill tag (e.g. `simplify`,
  `mattpocock-skills:code-review`), null when no skill active.
- `requestId`, `timestamp` (ISO), `sessionId`, `isSidechain`, `uuid`,
  `parentUuid`.
- `message.content[]`: `tool_use` blocks carry `id` + `name`
  (`Bash`, `Edit`, `Read`, `Agent`, `Skill`, `mcp__<server>__<tool>`, …).

## User record content shapes (for trigger classification)

`message.content` is one of: a plain **string**; an array of **text** blocks;
an array of **tool_result** blocks (each with `tool_use_id` referencing a
prior assistant `tool_use.id` — the tool *name* lives only on the tool_use,
so classification requires the id→name join). `isMeta: true` user records
exist (system-injected, not a human prompt).

## Category classification (per unique request, in file line order)

1. In a `subagents/*.jsonl` file → **Agent**.
2. Preceding user record has tool_result blocks → join ids to names:
   any `mcp__*` → **MCP**; any from `Skill` tool → **Skill**;
   else → **Tool call**. Priority MCP > Skill > Tool call.
3. Preceding user record is fresh text: contains `<command-name>` /
   `<command-message>` (slash-command skill invocation) → **Skill**;
   `isMeta` or unclassifiable → **Overhead**; else → **Prompt**.

## Skill runs

Consecutive same-`attributionSkill` spans in a main session = one run.
Subagent usage joins the run whose span (in the parent session) covers the
subagent's spawn.

## Implementation-time findings (2026-07-29, confirmed on the same corpus)

- **Subagent linkage is direct**: every `agent-<id>.jsonl` has a sibling
  `agent-<id>.meta.json` (305/305 observed) with `toolUseId`, `agentType`,
  `model`, `spawnDepth`. Join `toolUseId` → the parent's `Agent`/`Task`
  tool_use → that request's `attributionSkill`. Timestamp containment is
  fallback only.
- **A skill invocation writes TWO user records** before its first request:
  the `Skill` tool_result, then the injected SKILL.md body as an `isMeta`
  text record. Classifying by only the last preceding user record mislabels
  every skill request — take the strongest trigger among all user records
  since the last counted request (MCP > Skill > Tool call > Prompt > Overhead).
- **`cwd` is on every record** — use it for the project label; decoding the
  flattened dir name is fallback only. Windows cwds are `C:\Users\...`.
- **More non-billable record types in the wild**: `permission-mode`,
  `file-history-delta`, `queue-operation`, `frame-link`, `custom-title`,
  `summary`, `x-claude-code-hook`.
- **Title generation is non-billable** (`ai-title` records carry no usage),
  so the Overhead category is legitimately ~0 on real corpora.
- A trailing-7-day window touches **8 local calendar days** (oldest partial).
