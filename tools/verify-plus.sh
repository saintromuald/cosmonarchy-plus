#!/usr/bin/env bash
# One-command verification: host tests -> cross build -> headless boot ->
# boot-state validation + plugin self-test verdict.
#
# Runs against the *dev* prefix (cosmonarchy-agent2), never the player's build.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PREFIX="${COSMO_VERIFY_PREFIX:-$HOME/Games/cosmonarchy-agent2/prefix}"
BUILD="${COSMO_VERIFY_BUILD:-Release}"
V="$PREFIX/drive_c/Cosmonarchy/$BUILD"
DISPLAY_NUM="${COSMO_VERIFY_DISPLAY:-99}"

cd "$ROOT/plus"
echo "== host tests (cosmo-plus-core) =="
cargo test -p cosmo-plus-core 2>&1 | tail -4

cd "$ROOT"
echo "== cross build =="
PROFILE=release tools/build.sh 2>&1 | tail -1

echo "== install into dev prefix ($BUILD) =="
COSMO_GAME_ROOT="$HOME/Games/cosmonarchy-agent2" \
    PROFILE=release tools/install-plus.sh "$BUILD"

echo "== headless boot (Xvfb :$DISPLAY_NUM) =="
tools/headless/stop_game.sh "$DISPLAY_NUM" >/dev/null
if ! tools/headless/boot_game.sh "$DISPLAY_NUM"; then
    echo "BOOT FAILED; game log tail:"
    tail -20 "/tmp/opencode/harness/game-$DISPLAY_NUM.log" || true
    exit 1
fi

echo "== boot state =="
tools/headless/check_state.py "$V/cosmo_plus.state.json" || exit 1

echo "== plugin self-test =="
cat "$V/cosmo_plus.selftest"
grep -q '^PASS' "$V/cosmo_plus.selftest" || exit 1

echo "== all green =="
tools/headless/stop_game.sh "$DISPLAY_NUM" >/dev/null
