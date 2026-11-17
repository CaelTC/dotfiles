---
name: council
description: Trigger this if the user says 'council'.
---

# Council

Adapted from t3dotgg/tstack (skills/council).

Before an audit, address outstanding PR comments, fix known issues, and run
relevant checks. Give council the updated implementation; do not defer known
fixes until reviews return. If the user wants to discard the implementation
and explore new ideas, skip fixes to code being replaced.

Bring in two delegates to help with the user's request, usually for review
or feedback. Both are Claude Opus 5.5 at high reasoning effort: start them in
parallel with the Agent tool, `model: "opus"`, `effort: "high"`, one call each
in the same message. Call them Reviewer A and Reviewer B. Brief them
independently: give each the same request and context, but not your own
conclusions or the other's findings, so their reads stay separate. If Opus 5.5
at high effort is unavailable, report it instead of substituting.

Give each delegate the request and relevant context. For review or feedback,
keep their work read-only. Collect their responses and summarize the useful
findings, disagreements, and recommended next steps.

Do not wait idle. When a result arrives, check its findings against the code
at once and decide what to fix while the other delegate runs. Do not edit
files while a delegate still reviews the same checkout. If the user's
instructions mark a delegate as non-blocking, do not wait for it; treat a
late result as feedback and do not record its score.

For a code review, ask each delegate for a score out of 10.
Count each full review pass as one round.
A fresh review after fixes starts a new round, with two fresh delegates.

If the work ends in a pull request, put the results in the PR description.
Put this section before the model and harness line at the end:

```markdown
## Council review

Rounds: 2

| Delegate | Round 1 | Round 2 |
| --- | --- | --- |
| Reviewer A (Opus 5.5, high) | 7/10 | 9/10 |
| Reviewer B (Opus 5.5, high) | 8/10 | 10/10 |
```

If a delegate gives no score, write `no score`. Do not invent one.
If the PR is open, edit the description (`gh-axi pr edit`). Do not post a
comment for this. If there is no PR, put the same results in the final reply
only.

If you agree with their recommendations, make the changes and request a fresh
review after. Continue this process until both delegates are satisfied with the
outcome.

Do not let repeated reviews result in scope creep. It is your job to fight to
keep scope limited.
