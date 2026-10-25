#!/usr/bin/env bash
# Print the most recent boot report / self-test for a Cosmonarchy build.
#
# Usage: tools/cosmo-check.sh [build-dir-relative-to-prefix]
#   e.g. tools/cosmo-check.sh versions/release-my-build-latest
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PREFIX="${COSMO_GAME_ROOT:-$HOME/Games/cosmonarchy/prefix}"
BUILD="${1:-versions/release-my-build-latest}"
V="$PREFIX/drive_c/Cosmonarchy/$BUILD"

if [ -f "$V/cosmo_plus.selftest" ]; then
    echo "selftest: $(cat "$V/cosmo_plus.selftest")"
fi
if [ -f "$V/cosmo_plus.state.json" ]; then
    exec "$ROOT/tools/headless/check_state.py" "$V/cosmo_plus.state.json"
fi
echo "no cosmo_plus.state.json at $V (plugin has not booted with this build)"
exit 1
