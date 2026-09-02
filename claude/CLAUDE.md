# Global Claude Code instructions

## Isolate parallel agents in their own worktrees
When acting as an orchestrator that spawns multiple subagents to work in
parallel on the same git repo, give each agent that writes files its own
worktree so they don't collide in the shared checkout: pass
`isolation: "worktree"` to the Agent tool for every such agent.

Skip isolation when:
- only one agent is doing the work, or the agents are read-only
  (exploration, search, review), or
- the working directory is not a git repository (e.g. ~/Documents) — there's
  nothing to branch from.

Agent worktrees branch from the current HEAD (`worktree.baseRef = "head"`), so
unpushed commits and feature-branch state come along.

## Explore or planning
For small/clear changes, skip /grill-me — ask the relevant questions and
confirm before implementing.

## Always use the AXI CLIs, never the human-facing equivalent
These wrappers are the required path. Reach for them first; do not fall back to
the human tool because it is more familiar.

| Task | Use | Never |
|---|---|---|
| Anything on GitHub (issues, PRs, runs, releases, repos, labels, raw API) | `gh-axi` | `gh`, git-over-https scraping, WebFetch on github.com |
| Task/backlog state (add, start, done, block, hold, ready) | `tasks-axi` | hand-editing `backlog.md`, ad-hoc TODO lists |
| Driving a real browser (navigate, click, fill, screenshot, console/network) | `chrome-devtools-axi` | playwright/puppeteer, `open`, WebFetch on an interactive page |
| Handing back a rich/visual response for review | `lavish-axi` | pasting a wall of prose |

All four are installed in `/opt/homebrew/bin`; call them directly, no `npx` needed.
If an AXI CLI genuinely lacks a subcommand, say so explicitly and ask before
using the human tool — don't switch silently. `gh` is blocked by a deny rule in
settings.json for this reason; use `gh-axi api` for anything not covered.
