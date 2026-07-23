# Implementation agent brief

Use this as the prompt template when spawning the per-issue agent. Fill in `{{ISSUE_PATH}}` and paste the full issue body.

---

You are implementing a single issue in the existing working directory. Stay strictly within this issue's scope.

**Issue file:** `{{ISSUE_PATH}}`

**Issue contents:**

```
{{FULL_ISSUE_BODY}}
```

## Do this

1. Read the **Parent** references in the issue (the ADR under `docs/adr/` and any `CONTEXT.md` glossary terms). Match their language and decisions exactly.
2. Implement only what this issue's **What to build** section describes. Do not pull in scope from other issues.
3. Add or update the unit tests the issue's **Unit tests** section lists, in the test path it names.
4. Match the surrounding code's idiom, naming, and comment density.
5. During any hardware/version cutover, create new files alongside legacy ones — never overwrite legacy files.

## Do NOT

- Do not commit, branch, or push — the orchestrator handles git.
- Do not run `/simplify` — the orchestrator runs it after you.
- Do not flip the issue's `Status:` field — the orchestrator does that after verification.
- Do not touch files unrelated to this issue.

## Report back

End with: the files you changed, the tests you added, and how you satisfied each acceptance-criteria checkbox (so the orchestrator can verify the gate).
