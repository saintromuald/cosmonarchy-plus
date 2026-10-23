#!/bin/bash
# Cosmonarchy Plus - macOS installer & updater.
#
# Double-click this file (right-click -> Open the first time, then Open again).
# It installs Cosmonarchy Plus into its own folder and keeps it updated:
#
#   ~/Library/Application Support/CosmonarchyPlus/
#       game/     the game (Release + Starcraft)
#       prefix/   the Wine prefix
#
# Your normal Cosmonarchy is never touched. Three launchers appear on the
# Desktop: Cosmonarchy Plus (.command), (Host).command, (Guest).command.
# Running this installer again later = update.
#
# Needs a Wine runtime: Whisky (free, Apple Silicon) or CrossOver, or any
# wine 9+ with 32-bit support on Intel Macs. It is only used to RUN the game;
# you never need to open Whisky/CrossOver to play.

set -u

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
BASE="$HOME/Library/Application Support/CosmonarchyPlus"
GAME="$BASE/game"
PREFIX="$BASE/prefix"
TMPDIR_LOCAL="${TMPDIR:-/tmp}/cosmonarchy-plus-install"
LOGFILE="$BASE/install.log"
REPO="saintromuald/cosmonarchy-plus"
API="https://api.github.com/repos/$REPO"

mkdir -p "$BASE" "$TMPDIR_LOCAL"

log()  { echo "$*"; echo "[$(date '+%F %T')] $*" >> "$LOGFILE"; }
die()  { echo; echo "INSTALL FAILED: $*" >&2; echo "Log: $LOGFILE" >&2; exit 1; }

echo "================================================================="
echo " Cosmonarchy Plus - installer & updater (macOS)"
echo "================================================================="

# ---------------- 1. find a wine runtime -------------------------------------
find_wine() {
  [[ -n "${WINE:-}" && -x "$WINE" ]] && { echo "$WINE"; return 0; }
  local c w
  for c in \
    "/Applications/Whisky.app/Contents/Resources/Libraries/Wine/bin/wine" \
    "$HOME/Library/Application Support/com.isaacmarovitz.Whisky/Libraries/Wine/bin/wine" \
    "$HOME/Library/Application Support/com.franke.Whisky/Libraries/Wine/bin/wine" \
    "/Applications/CrossOver.app/Contents/SharedSupport/CrossOver/bin/wine" \
    "/opt/homebrew/bin/wine" "/usr/local/bin/wine" "/usr/local/bin/wine-stable" \
    "/Applications/Wine Staging.app/Contents/Resources/wine/bin/wine" \
    "/Applications/Wine Stable.app/Contents/Resources/wine/bin/wine"; do
    [[ -x "$c" ]] && { echo "$c"; return 0; }
  done
  # Deeper scan inside Whisky/CrossOver bundles (path layouts change).
  for d in \
    "/Applications/Whisky.app/Contents/Resources/Libraries" \
    "$HOME/Library/Application Support/com.isaacmarovitz.Whisky/Libraries" \
    "$HOME/Library/Application Support/com.franke.Whisky/Libraries"; do
    [[ -d "$d" ]] || continue
    w="$(find "$d" -maxdepth 5 -type f -name wine -perm -u+x 2>/dev/null | head -1)"
    [[ -n "$w" ]] && { echo "$w"; return 0; }
  done
  return 1
}

if ! WINE_BIN="$(find_wine)"; then
  echo
  echo "No Wine runtime found. Cosmonarchy is a Windows game, so one is needed"
  echo "to run it - you will not need to touch it after this."
  echo
  if [[ "$(uname -m)" == "arm64" ]]; then
    echo "Recommended (free): Whisky - https://getwhisky.app  (or: brew install --cask whisky)"
    echo "Install it, open it ONCE so it downloads its Wine runtime, then run"
    echo "this installer again."
  else
    echo "Install a Wine with 32-bit support, e.g.:"
    echo "  brew install --cask wine-stable"
    echo "then run this installer again."
  fi
  exit 1
fi
log "wine: $WINE_BIN"

# ---------------- 2. provision the prefix (the wine tinkering) ----------------
export WINEPREFIX="$PREFIX"
if [[ -f "$SCRIPT_DIR/provision-prefix.sh" ]]; then
  WINE="$WINE_BIN" GAMEDIR="$GAME" bash "$SCRIPT_DIR/provision-prefix.sh" || true
else
  # Fallback if the Setup zip is missing the shared script.
  WINEDEBUG=-all "$WINE_BIN" wineboot -u >/dev/null 2>&1 || true
  WINEDEBUG=-all "$WINE_BIN" winecfg -v win10 >/dev/null 2>&1 || true
  WINEDEBUG=-all "$WINE_BIN" reg add 'HKCU\Software\Wine\DllOverrides' /v ddraw /d 'native,builtin' /f >/dev/null
fi

# ---------------- 3. find the base game ---------------------------------------
find_game() {
  local d c
  for d in \
    "$HOME/Documents/Cosmonarchy" "$HOME/Cosmonarchy" "/Applications/Cosmonarchy" \
    "$HOME/Library/Application Support/CrossOver/Bottles/"*"/drive_c/Cosmonarchy" \
    "$HOME/Library/Containers/com.isaacmarovitz.Whisky/Bottles/"*"/drive_c/Cosmonarchy" \
    "$HOME/Library/Containers/com.franke.Whisky/Bottles/"*"/drive_c/Cosmonarchy" \
    "$HOME/Library/Containers/com.isaacmarovitz.Whisky/Bottles/"*"/drive_c/users/crossover/Documents/Cosmonarchy"; do
    [[ -d "$d/Release" && -d "$d/Starcraft" ]] && { echo "$d"; return 0; }
  done
  # One-level scan of Whisky bottles for nested Cosmonarchy folders.
  for b in "$HOME/Library/Containers/"*"/Bottles/"*"/drive_c/"*; do
    [[ -d "$b" ]] || continue
    for c in "$b"/Cosmonarchy* "$b"/*/Cosmonarchy*; do
      [[ -d "$c/Release" && -d "$c/Starcraft" ]] && { echo "$c"; return 0; }
    done
  done
  return 1
}

if [[ -d "$GAME/Starcraft" ]]; then
  SRC_GAME=""
  log "base game already present"
else
  if ! SRC_GAME="$(find_game)"; then
    echo
    echo "Could not find an existing Cosmonarchy install (the base game files"
    echo "are copied from it - the mod ships without them)."
    SRC_GAME="$(osascript -e 'POSIX path of (choose folder with prompt "Select the Cosmonarchy folder (contains Release and Starcraft)")' 2>/dev/null | sed 's#/$##')"
    [[ -n "$SRC_GAME" && -d "$SRC_GAME/Release" && -d "$SRC_GAME/Starcraft" ]] \
      || die "no Cosmonarchy folder selected. Install Cosmonarchy first (Fraud Launcher 2, fraudsclub.com), then run this again."
  fi
  log "base game source: $SRC_GAME"
  echo "Copying the base game (Starcraft) from:"
  echo "  $SRC_GAME"
  echo "(this is the only big one-time copy)"
  mkdir -p "$GAME"
  /usr/bin/ditto "$SRC_GAME/Starcraft" "$GAME/Starcraft" || die "copying Starcraft failed"
fi

# ---------------- 4. latest release + manifest --------------------------------
echo
echo "Checking for the latest Cosmonarchy Plus release..."
UA="CosmonarchyPlusMacInstaller"
rel_json="$(curl -fsSL -H "User-Agent: $UA" "$API/releases/latest")" \
  || die "cannot reach GitHub ($API). Check your internet connection."
TAG="$(printf '%s' "$rel_json" | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -1)"
VERSION="${TAG#v}"
[[ -n "$VERSION" ]] || die "could not read the latest release tag"
log "latest release: $TAG"

DLBASE="https://github.com/$REPO/releases/download/$TAG"
MODZIP="CosmonarchyPlus-mod-$VERSION.zip"
PLUGZIP="CosmonarchyPlus-plugins-$VERSION.zip"
MANIFEST="manifest.json"

curl -fsSL -H "User-Agent: $UA" -o "$TMPDIR_LOCAL/$MANIFEST" "$DLBASE/$MANIFEST" || die "no manifest in the release"
BUILD_REF="$(sed -n 's/.*"build_ref"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$TMPDIR_LOCAL/$MANIFEST" | head -1)"
curl -fsSL -H "User-Agent: $UA" -o "$TMPDIR_LOCAL/SHA256SUMS.txt" "$DLBASE/SHA256SUMS.txt" || die "no checksums in the release"

state="$GAME/plus-state.json"
INSTALLED_VERSION=""
[[ -f "$state" ]] && INSTALLED_VERSION="$(sed -n 's/.*"version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$state" | head -1)"
INSTALLED_BUILD=""
[[ -f "$state" ]] && INSTALLED_BUILD="$(sed -n 's/.*"build_ref"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$state" | head -1)"

if [[ "$INSTALLED_VERSION" == "$VERSION" && -f "$GAME/Release/Cosmonarchy BW.exe" ]]; then
  echo "You already have the latest version ($VERSION)."
  echo "Run the launchers on your Desktop to play; run this installer to update later."
  exit 0
fi

# ---------------- 5. download + verify + apply --------------------------------
fetch() { # name
  local f="$1" want got
  curl -fL --retry 3 -H "User-Agent: $UA" -o "$TMPDIR_LOCAL/$f" "$DLBASE/$f" || die "download failed: $f"
  want="$(awk -v n="$f" '$2==n {print $1}' "$TMPDIR_LOCAL/SHA256SUMS.txt")"
  if [[ -n "$want" ]]; then
    got="$(shasum -a 256 "$TMPDIR_LOCAL/$f" | awk '{print $1}')"
    [[ "$got" == "$want" ]] || die "checksum mismatch for $f (bad download - just run this again)"
  fi
}

NEED_BUILD=1
if [[ "$INSTALLED_BUILD" == "$BUILD_REF" && -f "$GAME/Release/Cosmonarchy BW.exe" ]]; then
  NEED_BUILD=0
fi

# Plugins zip: always (it is the small, frequently changing part).
echo "Downloading plugins ($PLUGZIP)..."
fetch "$PLUGZIP"
echo "Downloading game build ($MODZIP)..."  # only when the pinned build changed
if [[ "$NEED_BUILD" == 1 ]]; then fetch "$MODZIP"; fi

mkdir -p "$GAME/Release"
if [[ "$NEED_BUILD" == 1 ]]; then
  echo "Installing the game build..."
  rm -rf "$TMPDIR_LOCAL/mod-extract" && mkdir -p "$TMPDIR_LOCAL/mod-extract"
  unzip -qo "$TMPDIR_LOCAL/$MODZIP" -d "$TMPDIR_LOCAL/mod-extract" || die "extracting the mod zip failed"
  # Reset semantics like the dev launcher's git reset --hard: everything the
  # zip owns is replaced, but local runtime files (saves, logs, conf) survive.
  find "$GAME/Release" -mindepth 1 -maxdepth 1 \
    ! -name 'cosmo_saves' ! -name 'temp.mpq' ! -name 'cosmo_rollback.conf' \
    ! -name '*.log' -exec rm -rf {} +
  /usr/bin/ditto "$TMPDIR_LOCAL/mod-extract/Release" "$GAME/Release" || die "applying the build failed"
fi

echo "Installing plugins..."
rm -rf "$TMPDIR_LOCAL/plug-extract" && mkdir -p "$TMPDIR_LOCAL/plug-extract"
unzip -qo "$TMPDIR_LOCAL/$PLUGZIP" -d "$TMPDIR_LOCAL/plug-extract" || die "extracting the plugins zip failed"
mkdir -p "$GAME/Release/plugins"
cp -f "$TMPDIR_LOCAL/plug-extract/plugins/zz-cosmo-plus.qdp" "$GAME/Release/plugins/" || die "missing zz-cosmo-plus.qdp"
cp -f "$TMPDIR_LOCAL/plug-extract/plugins/zz-cosmo-rollback.qdp" "$GAME/Release/plugins/" || die "missing zz-cosmo-rollback.qdp"
if [[ -f "$TMPDIR_LOCAL/plug-extract/Starcraft/CrownLink.snp" ]]; then
  [[ -f "$GAME/Starcraft/CrownLink.snp.stock-backup" ]] || cp -a "$GAME/Starcraft/CrownLink.snp" "$GAME/Starcraft/CrownLink.snp.stock-backup" 2>/dev/null || true
  cp -f "$TMPDIR_LOCAL/plug-extract/Starcraft/CrownLink.snp" "$GAME/Starcraft/CrownLink.snp"
fi

# ---------------- 6. launchers -------------------------------------------------
cat > "$BASE/launch-cosmonarchy-plus.sh" <<EOF
#!/bin/bash
# Written by the Cosmonarchy Plus installer; rewritten on update.
ROLE="\${1:-solo}"
BASE="$BASE"
GAME="\$BASE/game"
export WINEPREFIX="\$BASE/prefix"
case "\$ROLE" in
  host)  printf 'CM_ROLLBACK_LIVE=1\\nCM_ROLLBACK_SLOT=0\\n' > "\$GAME/Release/cosmo_rollback.conf" ;;
  guest) printf 'CM_ROLLBACK_LIVE=1\\nCM_ROLLBACK_SLOT=1\\n' > "\$GAME/Release/cosmo_rollback.conf" ;;
  *)     rm -f "\$GAME/Release/cosmo_rollback.conf" ;;
esac
WINE="$WINE_BIN" GAMEDIR="\$GAME" bash "\$BASE/provision-prefix.sh" >/dev/null 2>&1 || true
cd "\$GAME/Release"
exec "$WINE_BIN" "\$GAME/Release/Cosmonarchy BW.exe"
EOF
chmod +x "$BASE/launch-cosmonarchy-plus.sh"

if [[ -f "$SCRIPT_DIR/provision-prefix.sh" ]]; then
  cp -f "$SCRIPT_DIR/provision-prefix.sh" "$BASE/provision-prefix.sh"
fi

make_launcher() { # label role
  local f="$HOME/Desktop/Cosmonarchy Plus ($1).command"
  cat > "$f" <<EOF
#!/bin/bash
exec "$BASE/launch-cosmonarchy-plus.sh" $2
EOF
  chmod +x "$f"
  log "launcher: $f"
}
make_launcher "Solo"  solo
make_launcher "Host"  host
make_launcher "Guest" guest

# ---------------- 7. record + verify ------------------------------------------
cat > "$state" <<EOF
{"product":"cosmonarchy-plus","version":"$VERSION","build_ref":"$BUILD_REF","installed":"$(date '+%F %T')"}
EOF

verify_boot() {
  [[ "${COSMO_SKIP_VERIFY:-0}" == 1 ]] && return 0
  echo
  echo "Checking that the game actually starts (about 20 seconds)..."
  : > "$GAME/Release/cosmo_plus.log"
  WINEDEBUG=-all "$WINE_BIN" "$GAME/Release/Cosmonarchy BW.exe" >"$TMPDIR_LOCAL/boot.log" 2>&1 &
  local pid=$! i
  for i in $(seq 1 20); do
    sleep 1
    pgrep -f "Cosmonarchy BW" >/dev/null 2>&1 && break
    kill -0 "$pid" 2>/dev/null || break
  done
  if pgrep -f "Cosmonarchy BW" >/dev/null 2>&1; then
    pkill -f "Cosmonarchy BW" 2>/dev/null || true
    sleep 2
    local plus_log="not written yet"
    [[ -s "$GAME/Release/cosmo_plus.log" ]] && plus_log="present"
    echo "OK - the game started. (Plus plugin log: $plus_log)"
  else
    echo
    echo "NOTE: the game did not stay running. See $TMPDIR_LOCAL/boot.log"
    if [[ "$(uname -m)" == "arm64" ]]; then
      echo "If the log mentions a 32-bit/bad CPU type error, the Wine runtime you"
      echo "have cannot run 32-bit Windows games. Install Whisky (getwhisky.app)"
      echo "and run this installer again - it will pick up Whisky's Wine."
    else
      echo "Make sure your Wine supports 32-bit apps (wine-stable does)."
    fi
    echo "You can still try the Desktop launchers - this is a warning, not a block."
  fi
}
verify_boot

# ---------------- done --------------------------------------------------------
echo
echo "================================================================="
echo " Cosmonarchy Plus $VERSION is installed."
echo
echo " On your Desktop:  Cosmonarchy Plus (Solo).command"
echo "                   Cosmonarchy Plus (Host).command"
echo "                   Cosmonarchy Plus (Guest).command"
echo
echo " Solo = Plus gameplay. Host/Guest = rollback netcode matches"
echo " (both players need Plus, same build)."
echo " Run this installer again any time to update."
echo " Your normal Cosmonarchy was not touched."
echo "================================================================="
