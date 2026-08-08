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
When prompted to explore or plan a feature, new project or new idea. If this is a bigger endeavor use the /grill-me skill to make sure the user and you operate under the same understanding. If the change is minimal or clear and do not need deeper exploration or grilling, ask relevant questions and ask for the users confirmation before starting the implementation.
