#!/usr/bin/env bash
# Install the freshly built Plus plugin into a Cosmonarchy build's plugins dir.
#
# Load order: `zz-cosmo-plus.qdp` sorts before `zz-cosmo-rollback.qdp`, so the
# netcode plugin keeps the outermost frame hook. Plus does not hook the frame.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BUILD="${1:-Release}"
PROFILE="${PROFILE:-debug}"

DLL="$ROOT/plus/target/i686-pc-windows-msvc/$PROFILE/cosmo_plus.dll"
GAME_ROOT="${COSMO_GAME_ROOT:-$HOME/Games/cosmonarchy}"
PLUGDIR="$GAME_ROOT/prefix/drive_c/Cosmonarchy/$BUILD/plugins"

[[ -f "$DLL" ]] || { echo "missing $DLL (run tools/build.sh first)" >&2; exit 1; }
[[ -d "$PLUGDIR" ]] || { echo "missing plugins dir $PLUGDIR" >&2; exit 1; }

cp "$DLL" "$PLUGDIR/zz-cosmo-plus.qdp"
echo "installed: $PLUGDIR/zz-cosmo-plus.qdp"

# Uninstall: rm "$PLUGDIR/zz-cosmo-plus.qdp"
