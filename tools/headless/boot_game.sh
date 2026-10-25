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

# Start the private Xvfb if it is not already up.
if ! pgrep -f "Xvfb :$DISPLAY_NUM " >/dev/null; then
    (Xvfb ":$DISPLAY_NUM" -screen 0 1920x1080x24 -nolisten tcp \
        >"$HARNESS/xvfb-$DISPLAY_NUM.log" 2>&1 &)
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

cd "$V" || exit 1
nohup umu-run "C:\\Cosmonarchy\\$BUILD\\Cosmonarchy BW.exe" \
    >"$HARNESS/game-$DISPLAY_NUM.log" 2>&1 &
echo $! >"$HARNESS/game-$DISPLAY_NUM.pid"

for _ in $(seq 1 120); do
    if [ -f "$V/cosmo_plus.state.json" ]; then
        echo "STATE-READY"
        exit 0
    fi
    sleep 2
done
echo "TIMEOUT: no cosmo_plus.state.json after 240s" >&2
exit 1
