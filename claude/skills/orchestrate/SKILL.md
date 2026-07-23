---
name: orchestrate
description: Orchestrate coding work by delegating to subagents with a token-efficient model tier system (sonnet research, opus implementation, haiku/sonnet small fixes, fable review loop on big changes). Use when the user asks to orchestrate a task, delegate work to subagents, or requests a feature/fix that spans research + implementation.
---

# Orchestrate

You are the orchestrator. Do not read/edit code yourself beyond light triage —
delegate via the Agent tool with the right model tier, then synthesize.

## Model tiers

| Work | Agent | Model |
|------|-------|-------|
| Research / codebase exploration | `Explore` | `sonnet` |
| Implementation (features, logic, multi-file) | `general-purpose` | `opus` |
| Small fixes (typo, one-liner, minor css, rename) | `general-purpose` | `haiku` (use `sonnet` if any logic is involved) |
| Code review of big changes | `general-purpose` | `fable` |

Never use opus for a small fix. Never skip the fable review on a big change.

## Workflow

1. **Classify the request.**
   - *Small*: typo, tiny css adjustment, rename, comment, config value → one haiku/sonnet agent, no research, no review loop. Done.
   - *Big*: everything else (new feature, logic change, refactor, bugfix touching flow, multi-file).

2. **Research (big only).** Spawn sonnet `Explore` agent(s) — parallel when the
   questions are independent. Ask for: files involved, existing patterns/helpers
   to reuse, constraints, and the exact insertion points. You keep the map; the
   implementor gets a distilled brief, not raw dumps.

3. **Implement.** Spawn an opus agent with a self-contained brief: goal,
   files + line anchors from research, patterns to follow, what NOT to build.
   Independent pieces → parallel opus agents; overlapping files → one agent.

4. **Review loop (big only).** After implementation, spawn a fable review agent:
   - Input: the diff (`git diff`) + the original goal.
   - Ask it to check the Coding Principles below and report concrete findings
     (file:line, problem, suggested fix) — no praise, findings only.
   - If findings: relaunch implementation subagents to apply them — opus for
     substantive changes, haiku/sonnet for small ones. Then re-review.
   - Cap at 2 review rounds. Remaining nits after round 2: report to the user
     instead of looping.

5. **Report.** Summarize what shipped, what the review caught, and anything
   deliberately skipped.

## Coding principles (review checklist)

<!-- TODO(user): refine these -->
- Reuse existing helpers/patterns in the codebase before writing new ones.
- No speculative abstractions — build only what the task needs (YAGNI).
- Single responsibility per function; names say what they do.
- Root-cause fixes, not symptom patches — check all callers.
- Validate input at trust boundaries; handle errors that lose data.
- Shortest diff that is correct and readable wins.

## Rules

- Briefs to subagents must be self-contained — subagents don't see this conversation.
- Batch independent Agent calls in one message so they run concurrently.
- Relay subagent results faithfully, including failures.
