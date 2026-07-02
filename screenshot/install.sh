#!/bin/bash
# curl -fsSL http://<host>:8000/install.sh | bash
set -euo pipefail

BIN="$HOME/.local/bin/screenshots_cleaning.sh"
DEST="$HOME/Desktop/ScreenShots"

mkdir -p "$(dirname "$BIN")" "$DEST"

cat > "$BIN" <<'SCRIPT'
#!/bin/bash
# ponytail: nullglob so an empty Desktop is a no-op, not an error
shopt -s nullglob
shots=("$HOME/Desktop/"Screenshot*)
[ ${#shots[@]} -eq 0 ] && exit 0
mkdir -p "$HOME/Desktop/ScreenShots"
mv "${shots[@]}" "$HOME/Desktop/ScreenShots/"
SCRIPT
chmod +x "$BIN"

# Install cron job (every 5 min), idempotent
LINE="*/5 * * * * $BIN"
( crontab -l 2>/dev/null | grep -vF "$BIN"; echo "$LINE" ) | crontab -

echo "Installed $BIN and cron job (*/5 min). Screenshots -> $DEST"
