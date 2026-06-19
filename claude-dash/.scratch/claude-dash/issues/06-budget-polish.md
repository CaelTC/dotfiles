# Budget polish

Status: ready-for-agent

## What to build

Finish the **Budget left rail** so it reads at a glance. Highlight the **Representative Window** (the binding one, from `representative-claim`). Drive coloring from `status` and utilization (e.g. green allowed → amber as a window fills → red on rejected/overage). Surface overage state when present (`anthropic-ratelimit-unified-overage-status`, `-overage-disabled-reason`) and the `-fallback-percentage` where relevant. Finalize styling to match the btop-twin layout (left rail: both windows, status, resets).

## Acceptance criteria

- [ ] The representative (binding) window is visually emphasized over the other
- [ ] Status + utilization drive color (allowed → warning → rejected/overage)
- [ ] Overage state is shown when the headers report it
- [ ] Left rail is legible at a glance and consistent with the btop-twin layout

## Blocked by

- 01-budget-pipeline-tracer
