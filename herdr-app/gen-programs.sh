#!/usr/bin/env bash
# Prints the Herdr-app programs.json fragment for this machine's firstmate:
# a pirate-flag program tinted with a per-machine color. The my-agents/my-tabs/
# my-pane mods read its icon color for firstmate's primary (mods/*/firstmate.js).
#   gen-programs.sh            -> JSON on stdout
#   FM_HOST=redphone gen-programs.sh   (override the hostname, for testing)
set -euo pipefail

host="$(printf '%s' "${FM_HOST:-$(hostname -s 2>/dev/null || hostname)}" | tr '[:upper:]' '[:lower:]')"

# Hostname-keyed colors for the known fleet; any other host derives a stable
# color from a checksum of its name (hue 0-359, fixed saturation/lightness).
case "$host" in
  macbookpro) color="#e5484d" ;;  # red
  redphone)   color="#3e63dd" ;;  # blue
  work-mac)   color="#30a46c" ;;  # green
  *)
    color="$(printf '%s' "$host" | cksum | awk '{
      h = ($1 % 360) / 60; s = 0.65; l = 0.55
      c = (1 - (2*l - 1 < 0 ? 1 - 2*l : 2*l - 1)) * s
      x = c * (1 - ((h % 2) - 1 < 0 ? 1 - (h % 2) : (h % 2) - 1))
      m = l - c / 2
      i = int(h)
      if      (i == 0) { r=c; g=x; b=0 } else if (i == 1) { r=x; g=c; b=0 }
      else if (i == 2) { r=0; g=c; b=x } else if (i == 3) { r=0; g=x; b=c }
      else if (i == 4) { r=x; g=0; b=c } else             { r=c; g=0; b=x }
      printf "#%02x%02x%02x", (r+m)*255+0.5, (g+m)*255+0.5, (b+m)*255+0.5 }')"
    ;;
esac

jq -n --arg color "$color" '{
  programs: {
    firstmate: {
      name: "Firstmate",
      icon: "my-icons/firstmate-flag.svg",
      tint: true,
      color: $color
    }
  }
}'
