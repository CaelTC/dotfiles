# Read transcripts, not traffic — no proxy, no wrapper

claude-analyzer answers "where did my tokens go this week" for individual
colleagues on their own Windows and macOS machines, with per-Category
(Prompt / Tool call / MCP / Skill / Agent) attribution. The sibling project
`claude-dash` (this repo) captures usage via a local proxy, and the initial
pitch for this tool was likewise to "study all traffic from Claude Code" —
so choosing the opposite architecture needs explaining.

**Decision:** claude-analyzer is a pure reader of the transcripts Claude Code
already writes (`~/.claude/projects/**/*.jsonl`). No proxy, no launch wrapper,
no traffic interception. Every reported token comes from a transcript
`usage` block; categories come from classifying transcript message content
(trigger attribution).

## Considered options

- **Proxy capture (claude-dash's ADR 0001 choice)**: rejected here because
  the one thing only the proxy can see — account-wide Budget from rate-limit
  headers — is not in this tool's requirements, while everything that *is*
  (per-request usage, tool names, skill invocations, sidechains) is already
  in transcripts. The proxy also requires launch ceremony (a wrapper
  colleagues must always use, and `cca` is zsh — a Windows rewrite), captures
  nothing retroactively, and a proxy bug can break live Claude sessions.
- **Transcripts (chosen)**: zero ceremony (install a binary, run it), works
  on weeks of pre-existing history, cross-platform by construction, and
  cannot interfere with Claude Code. Its blind spot — no rate-limit /
  Budget data — is accepted and recorded in CONTEXT.md (**Week** is a
  trailing-7-day approximation of Anthropic's rolling window, not the
  billing window itself).

## Consequences

- claude-analyzer can never show remaining subscription budget or exact
  window resets; colleagues wanting that use claude-dash (macOS) or the
  website. Do not bolt header capture onto this tool later — that need is
  claude-dash's territory.
- Per-component token figures are an attribution model over per-request
  usage blocks (trigger attribution; Skill runs), not raw measurements —
  the definitions live in CONTEXT.md and are the contract to keep stable.
- The tool inherits Claude Code's transcript format with no contract:
  a format change can break parsing at any release. Parsing must be
  lenient (skip-and-count unknown records, never crash the Report).
