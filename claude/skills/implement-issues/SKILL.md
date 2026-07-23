---
name: implement-issues
description: Orchestrate implementation of a ready issue set in .scratch/<feature>/issues/ by spawning one agent per issue, sequentially and dependency-ordered, running /simplify after each. Use when issues are ready to implement, when the user says "implement the issues", "work the issues", "run the orchestrator", or names a .scratch feature to build.
---

# Implement Issues (orchestrator)

Drives a feature's issue set from `ready-for-agent` to `done`, one issue at a time, in the **current working directory** (not a worktree). You are the orchestrator: you spawn an implementation agent per issue using Sonnet, then close the loop yourself. Determine if the workflow can have parralel agents. If possible parralelize the work.

## Pick the feature

1. If a feature name was passed as an argument, use `.scratch/<feature>/issues/`.
2. Otherwise, list every `.scratch/*/issues/` folder and **ask the user which one** (one question, options = the feature folder names). Do not guess.

Confirm the folder exists and contains `NN-*.md` issue files before proceeding.

## Build the run order

Read every issue file in the folder. Each has `Status:`, an `NN-` numeric prefix, and a `## Blocked by` section.

- Skip issues already `Status: done`.
- Order the rest by `NN` prefix, but never start an issue whose `Blocked by` names an issue that isn't `done` yet — defer it until its blockers complete.
- If a dependency cycle or a blocker that will never resolve is detected, stop and report it.

Show the planned order to the user, then run it without pausing between issues (no per-issue approval gate).

## Per-issue loop

For each issue in order:

1. **Implement.** Spawn one agent (`subagent_type: general-purpose`) with the full issue file contents. Instruct it to: read the parent ADR / `CONTEXT.md` glossary terms the issue references; implement only this issue's scope in the working dir; add/adjust the unit tests the issue specifies; not commit. See [agent-brief.md](agent-brief.md) for the exact brief.
2. **Simplify.** Invoke the `simplify` skill (Skill tool) on the resulting diff.
3. **Verify (gate).** Spawn a Sonnet agent and run the issue's unit tests / relevant suite and confirm each `## Acceptance criteria` checkbox is genuinely met. If tests fail or any criterion is unmet, **halt the whole run** and report — do not flip status, do not commit, do not advance.
4. **Review** Spawn an Opus agent to use the skill /ponytail-review and implement the proposed change with an Opus agent.
5. **Correct** Once the review is done handoff the findings to an Sonnet agent to implement the proposed changes.
6. **Flip status.** Edit the issue file's `Status:` field to `done`.
7. **Commit.** One commit for this issue (code + tests + the status flip), message referencing the issue filename. End the message with the `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>` trailer. Commit on the current branch (branch first only if on the default branch).

## Finish

After the last unblocked issue, report: which issues completed, which were skipped (already done) or deferred, and any gate that halted the run. Do not open a PR unless asked.

## Guardrails

- A failed verify gate stops everything; surface the failing test output or unmet criterion verbatim.
- Honor the user's parallel-migration rule: agents create new files alongside legacy ones during cutovers, never overwrite legacy.
