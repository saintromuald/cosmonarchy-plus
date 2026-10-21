#!/usr/bin/env bash
# Cross-build the Cosmonarchy Plus DLL for 32-bit Windows from Linux.
# Same toolchain as the netcode crate (cargo-xwin supplies MSVC CRT/SDK).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"
export CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
export PATH="$CARGO_HOME/bin:$PATH"
export XWIN_ARCH=x86

TARGET="${TARGET:-i686-pc-windows-msvc}"
PROFILE="${PROFILE:-dev}"

cd "$ROOT/plus"
if [[ "$PROFILE" == "release" ]]; then
    exec cargo xwin build --target "$TARGET" --release "$@"
else
    exec cargo xwin build --target "$TARGET" "$@"
fi
