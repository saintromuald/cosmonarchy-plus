#!/usr/bin/env bash
# Stop any verify-run game and reset the dev prefix's wineserver, so the next
# headless boot starts clean.
set -u

PREFIX="${COSMO_VERIFY_PREFIX:-$HOME/Games/cosmonarchy-agent2/prefix}"
DISPLAY_NUM="${1:-99}"

# Kill the game launched by boot_game.sh, and any Starcraft.exe in the dev prefix.
if [ -f "/tmp/opencode/harness/game-$DISPLAY_NUM.pid" ]; then
    kill "$(cat "/tmp/opencode/harness/game-$DISPLAY_NUM.pid")" 2>/dev/null || true
fi
for p in $(pgrep -f 'Starcraft\.exe'); do
    if [ -r "/proc/$p/environ" ] && tr '\0' '\n' <"/proc/$p/environ" 2>/dev/null | grep -q "$PREFIX"; then
        kill "$p" 2>/dev/null || true
    fi
done
sleep 2
WINEPREFIX="$PREFIX" wineserver -k 2>/dev/null || true
sleep 2
# Leave Xvfb running (cheap); restarting it churns window ids.
echo "stopped"
