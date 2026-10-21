#!/usr/bin/env bash
# Boot Cosmonarchy headlessly and check that the Plus plugin loaded and stayed quiet.
# Exit 0 = log found and no panic/error line; 1 = missing; 2 = suspicious log.
set -u
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BUILD="${1:-Release}"
GAME_ROOT="${COSMO_GAME_ROOT:-$HOME/Games/cosmonarchy}"
LOG="$GAME_ROOT/prefix/drive_c/Cosmonarchy/$BUILD/cosmo_plus.log"

: > "$LOG"
"$ROOT/tools/build.sh" >/dev/null
"$ROOT/tools/install-plus.sh" "$BUILD" >/dev/null
bash "$HOME/Projects/cosmonarchy-rollback/tools/headless_probe.sh" "$BUILD" none "" \
    /tmp/opencode/plus-verify.png 40 2 >/dev/null 2>&1

if [[ ! -s "$LOG" ]]; then
    echo "FAIL: no $LOG (plugin did not load)"
    exit 1
fi

echo "--- $LOG"
cat "$LOG"
if grep -qiE "panic|error|failed|exception" "$LOG"; then
    echo "FAIL: suspicious log line"
    exit 2
fi
echo "OK: Plus loaded cleanly"
