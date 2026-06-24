# Global Claude Code instructions

## Use a git worktree for code changes
When starting implementation work — anything that writes files — in a git
repository, first create an isolated worktree with EnterWorktree instead of
editing the main checkout.

Skip this when:
- the task is read-only (exploration, questions, reviews), or
- the working directory is not a git repository (e.g. ~/Documents) — there's
  nothing to branch from, so don't try.

New worktrees branch from the current HEAD (`worktree.baseRef = "head"`), so
unpushed commits and feature-branch state come along.
