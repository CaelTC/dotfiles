#!/bin/sh
# workspace.created hook: rename the fresh tab to Agents, add a Terminal tab.
set -eu

[ -n "${HERDR_WORKSPACE_ID:-}" ] && [ -n "${HERDR_TAB_ID:-}" ] || exit 0
herdr=${HERDR_BIN_PATH:-herdr}

"$herdr" tab rename "$HERDR_TAB_ID" Agents
"$herdr" tab create --workspace "$HERDR_WORKSPACE_ID" --label Terminal --no-focus
