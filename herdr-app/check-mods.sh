#!/usr/bin/env bash
# Checks the Herdr mods in herdr-app/mods: the shared firstmate.js copies are
# identical, and each mod loads and renders (`herdr-app check`).
set -euo pipefail
dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/mods" && pwd)"
ref="$dir/my-pane/firstmate.js"
for m in "$dir"/*/; do
  if [ -f "$m/firstmate.js" ] && ! cmp -s "$ref" "$m/firstmate.js"; then
    echo "firstmate.js differs: $m (keep it identical to $ref)" >&2
    exit 1
  fi
  herdr-app check "$m"
done
