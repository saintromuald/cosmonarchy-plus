#!/usr/bin/env bash
# tools/cosmo-restart.sh — fast restart of a Cosmonarchy build: kill the
# current game and relaunch it immediately, skipping the slow parts where
# possible. This is the "code change" half of hot reload; config-only changes
# use the in-game REPL (`reload-config`) instead.
#
#   tools/cosmo-restart.sh [build]      relaunch on the current display
set -euo pipefail

BUILD="${1:-versions/release-my-build-latest}"
PREFIX="${COSMO_GAME_ROOT:-$HOME/Games/cosmonarchy/prefix}"
V="$PREFIX/drive_c/Cosmonarchy/$BUILD"

[ -f "$V/Cosmonarchy BW.exe" ] || { echo "no build at $V" >&2; exit 1; }

# Kill the current game in this prefix only.
for p in $(pgrep -f 'Starcraft\.exe' 2>/dev/null); do
    if [ -r "/proc/$p/environ" ] && tr '\0' '\n' <"/proc/$p/environ" 2>/dev/null | grep -q "$PREFIX"; then
        kill "$p" 2>/dev/null || true
    fi
done
sleep 2
WINEPREFIX="$PREFIX" wineserver -k 2>/dev/null || true
sleep 2

export WINEPREFIX="$PREFIX"
export PROTONPATH=GE-Proton
export PROTON_VERB=waitforexitandrun
export GAMEID="umu-cosmonarchy-restart"
cd "$V"
exec "$HOME/.local/bin/cosmo-display-guard" \
    umu-run "C:\\Cosmonarchy\\$BUILD\\Cosmonarchy BW.exe"
