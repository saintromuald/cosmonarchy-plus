#!/usr/bin/env bash
# provision-prefix.sh — make a Wine prefix ready for Cosmonarchy.
#
# This encodes everything that had to be tinkered into place by hand to make
# Cosmonarchy run under Wine (verified against the working GE-Proton prefix):
#
#   1. Windows 10 mode      (winecfg -v win10; Proton defaults to it, wine may not)
#   2. ddraw = native,builtin  so the game loads cnc-ddraw's ddraw.dll from the
#      game folder instead of Wine's builtin ddraw. This is the one override the
#      game needs; without it rendering falls back to Wine's ddraw and the
#      cnc-ddraw settings (window size, maxfps, vsync) are ignored.
#   3. cnc-ddraw next to the game exe  the loader is copied from Starcraft/ into
#      Release/ so Wine's DLL search (exe dir first) always finds it, regardless
#      of how the game is launched.
#
# Used by:
#   - packaging/aur/cosmonarchy-plus.sh   (Arch AUR wrapper)
#   - dist/macos/install-cosmonarchy-plus.command (Mac installer)
#
# Environment:
#   WINEPREFIX   prefix to provision (created if missing)
#   WINE         wine binary to use (default: wine from PATH)
#   GAMEDIR      game folder containing Release/ and (optionally) Starcraft/
#
# Idempotent: safe to run before every launch.

set -euo pipefail

: "${WINEPREFIX:?WINEPREFIX is required}"
WINE="${WINE:-wine}"
GAMEDIR="${GAMEDIR:-}"

if ! command -v "$WINE" >/dev/null 2>&1 && [[ "$WINE" != */* ]]; then
  echo "provision-prefix: wine binary not found: $WINE" >&2
  exit 1
fi

export WINEPREFIX

# 1. Create and initialise the prefix on first use. wineboot also writes the
#    default registry the other steps edit.
if [[ ! -d "$WINEPREFIX/drive_c" ]]; then
  echo "provision-prefix: initialising prefix $WINEPREFIX ..."
  WINEDEBUG=-all "$WINE" wineboot -u 2>/dev/null || true
fi

# 2. Windows 10. Cosmonarchy's plugins assume a modern-enough kernel/user pair.
echo "provision-prefix: setting Windows version to 10"
WINEDEBUG=-all "$WINE" winecfg -v win10 2>/dev/null \
  || WINEDEBUG=-all "$WINE" reg add 'HKCU\Software\Wine' /v Version /d win10 /f >/dev/null

# 3. Native ddraw (cnc-ddraw) for the whole prefix. Matches the known-good
#    user.reg from the working install: "ddraw"="native,builtin".
echo "provision-prefix: setting ddraw override to native,builtin"
WINEDEBUG=-all "$WINE" reg add 'HKCU\Software\Wine\DllOverrides' /v ddraw /d 'native,builtin' /f >/dev/null

# 4. cnc-ddraw next to the game exe so it is found on every launch path.
if [[ -n "$GAMEDIR" && -d "$GAMEDIR/Release" ]]; then
  if [[ -f "$GAMEDIR/Starcraft/ddraw.dll" && ! -f "$GAMEDIR/Release/ddraw.dll" ]]; then
    echo "provision-prefix: copying cnc-ddraw into Release/"
    cp -n "$GAMEDIR/Starcraft/ddraw.dll" "$GAMEDIR/Release/ddraw.dll" 2>/dev/null || true
    cp -n "$GAMEDIR/Starcraft/ddraw.ini" "$GAMEDIR/Release/ddraw.ini" 2>/dev/null || true
    cp -rn "$GAMEDIR/Starcraft/Shaders" "$GAMEDIR/Release/Shaders" 2>/dev/null || true
  elif [[ ! -f "$GAMEDIR/Release/ddraw.dll" && ! -f "$GAMEDIR/Starcraft/ddraw.dll" ]]; then
    echo "provision-prefix: WARNING: no cnc-ddraw found in the game folder;" >&2
    echo "  rendering will use Wine's builtin ddraw (playable, but no windowing/vsync fixes)." >&2
  fi
fi

echo "provision-prefix: done"
