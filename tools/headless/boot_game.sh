#!/usr/bin/env bash
# Boot the game headless on a private Xvfb display and wait for the plugin's
# boot report (`cosmo_plus.state.json`, written when the plugin loads).
#
# Uses the dev prefix (cosmonarchy-agent2) by default so the player's build is
# never touched. Exit 0 once the report exists.
set -u

DISPLAY_NUM="${1:-99}"
PREFIX="${COSMO_VERIFY_PREFIX:-$HOME/Games/cosmonarchy-agent2/prefix}"
BUILD="${COSMO_VERIFY_BUILD:-Release}"
V="$PREFIX/drive_c/Cosmonarchy/$BUILD"
HARNESS=/tmp/opencode/harness
mkdir -p "$HARNESS"

[ -f "$V/Cosmonarchy BW.exe" ] || { echo "no build at $V" >&2; exit 1; }

# The player's live prefix is off-limits by default: a harness game left
# running on it holds the Wine prefix lock and hangs every real launch until
# it is killed. Only an explicit override allows it.
case "$PREFIX" in
  */Games/cosmonarchy/prefix)
    [ "${COSMO_VERIFY_FORCE_LIVE:-0}" = 1 ] || {
      echo "refusing the live prefix $PREFIX (set COSMO_VERIFY_FORCE_LIVE=1 to override)" >&2
      exit 1
    }
    ;;
esac

# Start the private Xvfb if it is not already up.
X_STARTED=0
if ! pgrep -f "Xvfb :$DISPLAY_NUM " >/dev/null; then
    (Xvfb ":$DISPLAY_NUM" -screen 0 1920x1080x24 -nolisten tcp \
        >"$HARNESS/xvfb-$DISPLAY_NUM.log" 2>&1 &)
    X_STARTED=1
    sleep 2
fi

export WINEPREFIX="$PREFIX"
export PROTONPATH=GE-Proton
export PROTON_VERB=waitforexitandrun
export GAMEID="umu-cosmo-verify"
export DISPLAY=":$DISPLAY_NUM"
export CM_PLUS_SELFTEST=1
export CM_PLUS_REPL="${CM_PLUS_REPL:-1}"
export CM_PLUS_VERIFY="${CM_PLUS_VERIFY:-}"

rm -f "$V/cosmo_plus.state.json" "$V/cosmo_plus.selftest"

# The game runs in its own process group (setsid) so cleanup can kill the
# whole tree — umu/pressure-vessel children survive a plain parent kill.
GAME_PID=""
cleanup() {
  if [ -n "$GAME_PID" ] && [ "${KEEP_GAME:-0}" != 1 ]; then
    kill -TERM -- "-$GAME_PID" 2>/dev/null || true
    sleep 2
    kill -KILL -- "-$GAME_PID" 2>/dev/null || true
  fi
  if [ "$X_STARTED" = 1 ] && [ "${KEEP_GAME:-0}" != 1 ]; then
    pkill -f "Xvfb :$DISPLAY_NUM " 2>/dev/null || true
  fi
}
trap cleanup EXIT

cd "$V" || exit 1
setsid umu-run "C:\\Cosmonarchy\\$BUILD\\Cosmonarchy BW.exe" \
    >"$HARNESS/game-$DISPLAY_NUM.log" 2>&1 &
GAME_PID=$!
echo "$GAME_PID" >"$HARNESS/game-$DISPLAY_NUM.pid"

for _ in $(seq 1 120); do
    if [ -f "$V/cosmo_plus.state.json" ]; then
        # Booted: leave the game running for play_match.py, which reaps the
        # process group when it is done.
        KEEP_GAME=1
        echo "STATE-READY"
        exit 0
    fi
    sleep 2
done
echo "TIMEOUT: no cosmo_plus.state.json after 240s" >&2
exit 1
