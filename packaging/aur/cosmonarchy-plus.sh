#!/usr/bin/env bash
# cosmonarchy-plus — launch Cosmonarchy Plus through umu + GE-Proton.
#
#   cosmonarchy-plus            Solo (Plus gameplay, no rollback netcode)
#   cosmonarchy-plus host       rollback match, you create the lobby
#   cosmonarchy-plus guest      rollback match, you join the lobby
#
# Mirrors the Windows "separate copy" model: the AUR package keeps the pristine
# tree in /opt/cosmonarchy-plus, and this launcher mirrors it into a user-owned
# folder (so the game can write logs/saves). Package updates take effect on the
# next launch, exactly like the git reset --hard the dev launcher performs.
# The Wine tinkering that made the game work (ddraw override, Windows 10 mode,
# cnc-ddraw next to the exe) is applied by provision-prefix.sh before launch.

set -euo pipefail

APP_DIR="${COSMO_PLUS_APP_DIR:-/opt/cosmonarchy-plus}"
DATA_DIR="${COSMO_PLUS_DATA_DIR:-${XDG_DATA_HOME:-$HOME/.local/share}/cosmonarchy-plus}"
PREFIX="${COSMO_PLUS_PREFIX:-$DATA_DIR/prefix}"
GAMEDIR="$DATA_DIR/game"

MODE="${1:-solo}"
case "$MODE" in
  solo|host|guest) ;;
  *) echo "usage: cosmonarchy-plus [solo|host|guest]" >&2; exit 1 ;;
esac

die() { echo "cosmonarchy-plus: $*" >&2; exit 1; }

[[ -d "$APP_DIR/Release" ]] || die "missing $APP_DIR/Release - is cosmonarchy-plus-bin installed?"

# ---- working copy ------------------------------------------------------------
# First run (or after an update): mirror the pristine Release tree. Exclusions
# are the runtime files the game writes; they survive syncs like untracked
# files survive the dev launcher's git reset --hard.
mkdir -p "$DATA_DIR/game"
if [[ ! -d "$GAMEDIR/Release" ]]; then
  echo "cosmonarchy-plus: first run - copying game files to $GAMEDIR ..."
  cp -a "$APP_DIR/Release" "$GAMEDIR/Release"
else
  rsync -a --delete \
    --exclude '*.log' --exclude 'cosmo_saves/' --exclude 'temp.mpq' \
    --exclude 'cosmo_rollback.conf' \
    "$APP_DIR/Release/" "$GAMEDIR/Release/" 2>/dev/null \
    || cp -au "$APP_DIR/Release/." "$GAMEDIR/Release/"
fi

# ---- Starcraft base game -----------------------------------------------------
# Copied once from an existing Cosmonarchy install (the AUR package does not
# redistribute the base game assets). Set COSMO_PLUS_STARCRAFT_SRC to skip the
# search.
if [[ ! -d "$GAMEDIR/Starcraft" ]]; then
  src="${COSMO_PLUS_STARCRAFT_SRC:-}"
  if [[ -z "$src" ]]; then
    for c in \
      "$HOME/Games/cosmonarchy/prefix/drive_c/Cosmonarchy/Starcraft" \
      "$HOME/Games/Cosmonarchy/Starcraft" \
      "$HOME/Cosmonarchy/Starcraft" \
      "$HOME/Documents/Cosmonarchy/Starcraft" \
      "$HOME/.wine/drive_c/Cosmonarchy/Starcraft" \
      "$HOME/.local/share/wineprefixes/"*"/drive_c/Cosmonarchy/Starcraft"; do
      [[ -d "$c" ]] && { src="$c"; break; }
    done
  fi
  if [[ -z "$src" ]]; then
    cat >&2 <<'EOF'
cosmonarchy-plus: could not find an existing Cosmonarchy install to copy the
base game (Starcraft) from. Install Cosmonarchy first (Fraud Launcher 2 from
fraudsclub.com), or set:

    COSMO_PLUS_STARCRAFT_SRC=/path/to/Starcraft

then run this again.
EOF
    exit 1
  fi
  echo "cosmonarchy-plus: copying Starcraft from $src ..."
  cp -a "$src" "$GAMEDIR/Starcraft"
  # Plus ships the rollback-capable CrownLink.snp; keep the stock one as backup.
  if [[ -f "$APP_DIR/Starcraft/CrownLink.snp" ]]; then
    [[ -f "$GAMEDIR/Starcraft/CrownLink.snp.stock-backup" ]] \
      || cp -a "$GAMEDIR/Starcraft/CrownLink.snp" "$GAMEDIR/Starcraft/CrownLink.snp.stock-backup" 2>/dev/null || true
    cp -a "$APP_DIR/Starcraft/CrownLink.snp" "$GAMEDIR/Starcraft/CrownLink.snp"
  fi
fi

# ---- Wine prefix -------------------------------------------------------------
# Everything that had to be tinkered into place by hand: Windows 10 mode,
# ddraw=native,builtin (cnc-ddraw), and cnc-ddraw copied next to the exe.
export WINEPREFIX="$PREFIX"
WINE="${WINE:-wine}" GAMEDIR="$GAMEDIR" \
  "$APP_DIR/provision-prefix.sh" >/dev/null

# ---- heal a truncated temp.mpq ----------------------------------------------
# If the game was killed during the hotloader's rebuild (~1 min black screen),
# temp.mpq is left incomplete and map start fails with "Bad length". Removing
# it makes the game rebuild it cleanly on this launch.
if [[ -f "$GAMEDIR/Release/temp.mpq" ]] && command -v python3 >/dev/null; then
  if ! python3 "$APP_DIR/cosmo-tempmpq-check" "$GAMEDIR/Release/temp.mpq" "$GAMEDIR/Release/mpq" >/dev/null 2>&1; then
    echo "cosmonarchy-plus: temp.mpq is incomplete; rebuilding game data on this launch (~1 min black screen)."
    rm -f "$GAMEDIR/Release/temp.mpq"
  fi
fi

# ---- arm the rollback netcode (host/guest only) ------------------------------
CONF="$GAMEDIR/Release/cosmo_rollback.conf"
rm -f "$CONF"
case "$MODE" in
  host)  printf 'CM_ROLLBACK_LIVE=1\nCM_ROLLBACK_SLOT=0\n' > "$CONF" ;;
  guest) printf 'CM_ROLLBACK_LIVE=1\nCM_ROLLBACK_SLOT=1\n' > "$CONF" ;;
esac

# ---- launch ------------------------------------------------------------------
export PROTONPATH=GE-Proton
export GAMEID="umu-cosmonarchy-plus"
export PROTON_VERB=waitforexitandrun
cd "$GAMEDIR/Release"
exec umu-run "$GAMEDIR/Release/Cosmonarchy BW.exe"
