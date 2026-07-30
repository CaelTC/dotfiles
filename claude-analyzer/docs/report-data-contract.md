# Report data contract (Rust core ↔ HTML template)

The Rust binary embeds `assets/report.html` (via `include_str!`) and replaces
the single token `__CLAUDE_ANALYZER_DATA__` with a JSON object of this shape.
The template must also work standalone in a browser by shipping a small inline
sample payload behind the same token (i.e. `const DATA = __CLAUDE_ANALYZER_DATA__;`
with a dev fallback if the token is still present).

All token numbers are integers. `fresh` = input + cache_creation + output.
`cacheRead` = cache_read (reported separately, never summed into fresh).
Categories are exactly: `"Prompt" | "Tool call" | "MCP" | "Skill" | "Agent" | "Overhead"`.

```json
{
  "generatedAt": "2026-07-29T18:30:00Z",
  "weekStart": "2026-07-22T18:30:00Z",
  "weekEnd": "2026-07-29T18:30:00Z",
  "prevWeekStart": "2026-07-15T18:30:00Z",
  "totals": { "fresh": 0, "cacheRead": 0, "requests": 0, "sessions": 0, "prevFresh": 0 },
  "skippedRecords": 0,
  "categories": [
    {
      "name": "MCP",
      "fresh": 0,
      "cacheRead": 0,
      "requests": 0,
      "prevFresh": 0,
      "servers": [
        { "name": "playwright", "fresh": 0, "cacheRead": 0, "requests": 0, "prevFresh": 0 }
      ]
    }
  ],
  "models": [
    { "name": "claude-opus-4-8", "fresh": 0, "cacheRead": 0, "requests": 0, "prevFresh": 0 }
  ],
  "days": [
    {
      "date": "2026-07-16",
      "week": "previous",
      "partial": true,
      "fresh": 0,
      "cacheRead": 0,
      "byCategory": { "Prompt": 0, "Tool call": 0, "MCP": 0, "Skill": 0, "Agent": 0, "Overhead": 0 }
    },
    {
      "date": "2026-07-23",
      "week": "current",
      "partial": false,
      "fresh": 0,
      "cacheRead": 0,
      "byCategory": { "Prompt": 0, "Tool call": 0, "MCP": 0, "Skill": 0, "Agent": 0, "Overhead": 0 }
    }
  ],
  "sessions": [
    {
      "id": "0582669e",
      "project": "dev/nootka-sop",
      "firstPrompt": "first user prompt, truncated to ~80 chars",
      "start": "2026-07-28T09:00:00Z",
      "end": "2026-07-28T11:00:00Z",
      "fresh": 0,
      "cacheRead": 0,
      "requests": 0,
      "byCategory": { "Prompt": 0, "Tool call": 0, "MCP": 0, "Skill": 0, "Agent": 0, "Overhead": 0 },
      "agents": [
        { "id": "a2b1e156", "model": "claude-haiku-4-5", "fresh": 0, "cacheRead": 0, "requests": 0 }
      ]
    }
  ],
  "skills": [
    {
      "name": "simplify",
      "runs": 0,
      "fresh": 0,
      "cacheRead": 0,
      "avgFreshPerRun": 0,
      "prevFresh": 0,
      "composition": { "main": 0, "agents": 0 }
    }
  ]
}
```

Notes:
- **Previous Week** (`prevFresh`, `prevWeekStart`): the trailing window
  immediately before the Week (now−14d → now−7d, anchored to today — never a
  calendar week). `prevFresh` = Fresh tokens in that window under the same
  attribution, present on `totals`, `categories` rows (and MCP `servers`),
  `models`, and `skills` — NOT on `sessions` (the sessions list is
  current-Week only). Rows that existed only in the Previous Week still
  appear (current fields 0, `prevFresh` > 0) so disappearances are visible.
  `categories`/`models`/`skills` `prevFresh` each sum to `totals.prevFresh`.
- `days` covers every local calendar day from `prevWeekStart` to now, oldest
  first; `week` marks which window each row belongs to; only the oldest may
  have `partial: true`. The boundary date straddles both windows, so it can
  appear TWICE — an earlier row tagged `"previous"` (present only when it
  actually carries Previous-Week traffic) followed by its `"current"` row.
  Renderers must iterate rows in array order and key by (date, week), never
  by date alone. Rows with `week: "current"` sum to `totals`; rows with
  `week: "previous"` sum to `totals.prevFresh` on `fresh`.
- `sessions` sorted by `fresh` desc; include every session with ≥1 request in
  the Week. Agent usage is nested under its parent session AND counted in the
  session's `byCategory.Agent` (session totals include agents).
- `skills` sorted by `fresh` desc; append a final `"(no skill)"` row so the
  grouping sums to the Week's total.
- Every grouping (`categories`, `days`, `sessions`, `skills`, `models`)
  independently sums to `totals`.
- `servers` appears only on the `"MCP"` category row: per-MCP-server breakdown
  (server = the `<server>` segment of `mcp__<server>__<tool>`), sorted by
  `fresh` desc, summing to the MCP row. If one request was triggered by
  results from multiple servers at once, attribute it to the first MCP server
  appearing in the triggering results (deterministic, no apportionment).
- `models` sorted by `fresh` desc; `name` is `message.model` verbatim
  (no normalization, keep date suffixes).
- `skippedRecords` counts unparseable/unknown lines (lenient parsing —
  surfaced in the Report footer, never a crash).
