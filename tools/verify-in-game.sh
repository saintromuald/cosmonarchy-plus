#!/usr/bin/env bash
# In-game verification: headless boot with the REPL armed, drive the menus to
# a solo match, then exercise every verifiable plugin/feature and capture
# evidence (logs + screenshots) under /tmp/opencode/harness.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PREFIX="${COSMO_VERIFY_PREFIX:-$HOME/Games/cosmonarchy-agent2/prefix}"
BUILD="${COSMO_VERIFY_BUILD:-Release}"
V="$PREFIX/drive_c/Cosmonarchy/$BUILD"
DISPLAY_NUM="${COSMO_VERIFY_DISPLAY:-99}"
H=/tmp/opencode/harness
mkdir -p "$H"

echo "== build + install =="
cd "$ROOT/plus" && cargo test -p cosmo-plus-core 2>&1 | tail -1
cd "$ROOT" && PROFILE=release tools/build.sh 2>&1 | tail -1
COSMO_GAME_ROOT="$HOME/Games/cosmonarchy-agent2" PROFILE=release \
    tools/install-plus.sh "$BUILD"

echo "== clean boot (REPL armed) =="
tools/headless/stop_game.sh "$DISPLAY_NUM" >/dev/null
tools/headless/boot_game.sh "$DISPLAY_NUM"

echo "== wait for window =="
export DISPLAY=":$DISPLAY_NUM"
for _ in $(seq 1 60); do
    WIN=$(xdotool search --name 'Cosmonarchy' 2>/dev/null | head -1 || true)
    [ -n "$WIN" ] && break
    sleep 2
done
if [ -z "${WIN:-}" ]; then
    echo "no game window" >&2
    exit 1
fi
echo "window $WIN"

echo "== drive menus to a match =="
python3 "$ROOT/tools/headless/play_match.py" "$DISPLAY_NUM"

echo "== REPL: live game state =="
export COSMO_REPL_DIR="$V"
"$ROOT/tools/cosmo-repl" frame
sleep 1
"$ROOT/tools/cosmo-repl" units
sleep 1

echo "== REPL: exercise the idle-worker hotkey path =="
"$ROOT/tools/cosmo-repl" idle
sleep 1

echo "== REPL: badge at the minimap anchor + probes, then screenshot =="
"$ROOT/tools/cosmo-repl" "badge default"
"$ROOT/tools/cosmo-repl" "icon 7 300 120 24 22"
"$ROOT/tools/cosmo-repl" "text 300 160 4 PROBE"
sleep 1
import -window "$WIN" "$H/in-game-badge.png" 2>/dev/null || true

echo "== let the AI play a while =="
sleep 45
"$ROOT/tools/cosmo-repl" units
sleep 1
import -window "$WIN" "$H/in-game-later.png" 2>/dev/null || true
"$ROOT/tools/cosmo-repl" units

echo "== REPL: in-game state report =="
"$ROOT/tools/cosmo-repl" state
sleep 1

echo "== evidence =="
echo "--- plus: idle / audit / report lines ---"
grep -E 'idle workers: selecting|draw proc owned|WARNING|state:' "$V/cosmo_plus.log" | tail -8
echo "--- selftest ---"
cat "$V/cosmo_plus.selftest" 2>/dev/null || echo "(none)"
echo "--- state report ---"
python3 "$ROOT/tools/headless/check_state.py" "$V/cosmo_plus.state.json" || true
echo "--- rollback plugin ---"
tail -5 "$V/cosmo_rollback.log" 2>/dev/null || echo "(no rollback log)"
echo "--- repl replies ---"
tail -8 "$V/cosmo_plus.repl.log" 2>/dev/null || echo "(no repl log)"
echo "== done =="
