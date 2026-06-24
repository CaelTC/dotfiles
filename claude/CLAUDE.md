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
