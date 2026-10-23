#!/bin/bash
# One-shot headless UMS map load test on the agent2 rig.
# Usage: test_ums.sh <display> <mapname-grep> <outfile>
# Expects exactly one test map installed at:
#   CMBW-maps/test/!ums-test.scx   (row is found by OCR)
set -u
DISP=$1
MAPNAME=$2
OUT=$3
PFX="/home/saintbellarmine/Games/cosmonarchy-agent2/prefix"
GAME="$PFX/drive_c/Cosmonarchy/Release/Cosmonarchy BW.exe"
LOG="$PFX/drive_c/Cosmonarchy/Release/cosmo_plus.log"
OCR="/home/saintbellarmine/Projects/cosmonarchy-rollback/tools/ocr_text.py"
OCRROW="/home/saintbellarmine/Projects/cosmonarchy-rollback/tools/ocr_row.py"

kill_old() {
  WINEPREFIX="$PFX" wineserver -k >/dev/null 2>&1
  pgrep -f "Cosmonarchy.BW" | xargs -r kill -9 2>/dev/null
  pgrep -x Xvfb | xargs -r kill -9 2>/dev/null
  sleep 2
}

snap() { # snap <name> <x> <y> <w> <h>  -> OCR text
  DISPLAY=:$DISP import -window root /tmp/opencode/cyc/$1.png 2>/dev/null
  python3 "$OCR" /tmp/opencode/cyc/$1.png "$2" "$3" "$4" "$5" 6 2>/dev/null
}

find_row() { # find_row <screenshot> <text>  -> "x y"
  python3 "$OCRROW" /tmp/opencode/cyc/$1.png "$2" 200 140 700 420 2>/dev/null
}

mkdir -p /tmp/opencode/cyc
kill_old
rm -f /tmp/.X${DISP}-lock
Xvfb :$DISP -screen 0 1280x720x24 -nolisten tcp >/tmp/opencode/cyc/xvfb.log 2>&1 &
sleep 3
cd "$PFX/drive_c/Cosmonarchy/Release" || exit 1
WINEPREFIX="$PFX" PROTONPATH=GE-Proton GAMEID=umu-cosmo-ums$DISP PROTON_VERB=waitforexitandrun \
  DISPLAY=:$DISP WAYLAND_DISPLAY= setsid umu-run "C:\\Cosmonarchy\\Release\\Cosmonarchy BW.exe" \
  >/tmp/opencode/cyc/game.log 2>&1 </dev/null &

ready=0
for i in $(seq 1 60); do
  sleep 5
  pgrep -x Xvfb >/dev/null || { echo "XVFB DIED at $i" >> "$OUT"; break; }
  TXT=$(DISPLAY=:$DISP import -window root /tmp/opencode/cyc/w.png 2>/dev/null && \
        python3 "$OCR" /tmp/opencode/cyc/w.png 0 0 1280 720 6 2>/dev/null | head -6)
  if echo "$TXT" | grep -qi "Cosmonarchy\|Single Player\|Multiplayer"; then
    ready=1; break
  fi
done
[ "$ready" = "1" ] || { echo "MENU NOT REACHED" >> "$OUT"; exit 1; }
echo "menu reached" >> "$OUT"

# Single Player -> Return -> Play Custom
DISPLAY=:$DISP xdotool mousemove 490 200 click 1; sleep 4
DISPLAY=:$DISP xdotool key Return; sleep 5
DISPLAY=:$DISP xdotool mousemove 320 585 click 1; sleep 5
snap pc 200 140 700 420 > /tmp/opencode/cyc/pc.txt
echo "== play custom rows ==" >> "$OUT"
head -20 /tmp/opencode/cyc/pc.txt >> "$OUT"

# find and click the "test" folder row
ROW=$(find_row pc "test")
if [ -z "$ROW" ]; then
  echo "TEST FOLDER ROW NOT FOUND" >> "$OUT"
  exit 1
fi
echo "test folder row: $ROW" >> "$OUT"
X=${ROW%% *}; Y=${ROW##* }
DISPLAY=:$DISP xdotool mousemove $((X+40)) $Y click --repeat 2 --delay 150 1; sleep 4
snap folder 200 140 700 420 > /tmp/opencode/cyc/folder.txt
echo "== test folder rows ==" >> "$OUT"
head -20 /tmp/opencode/cyc/folder.txt >> "$OUT"

# find and click the test map row
MROW=$(find_row folder "$MAPNAME")
if [ -z "$MROW" ]; then
  echo "MAP ROW NOT FOUND" >> "$OUT"
  exit 1
fi
echo "map row: $MROW" >> "$OUT"
MX=${MROW%% *}; MY=${MROW##* }
DISPLAY=:$DISP xdotool mousemove $((MX+40)) $MY click --repeat 2 --delay 150 1
BEFORE=$(grep -cE "anti-meta: game type" "$LOG" 2>/dev/null || echo 0)
sleep 12
C1=$(grep -cE "anti-meta: game type" "$LOG" 2>/dev/null || echo 0)
if [ "$C1" -le "$BEFORE" ]; then
  # briefing dialog needs an OK; one Return confirms it (never press Return in-game)
  DISPLAY=:$DISP xdotool key Return
  sleep 12
fi
AFTER=$(grep -cE "anti-meta: game type" "$LOG" 2>/dev/null || echo 0)
echo "counter before=$BEFORE after=$AFTER" >> "$OUT"
snap result 0 0 1280 720 | head -8 >> "$OUT"
if [ "$AFTER" -gt "$BEFORE" ]; then
  ALIVE=$(pgrep -f "Cosmonarchy.BW" | wc -l)
  echo "GAME STARTED (procs=$ALIVE)" >> "$OUT"
else
  RES=$(snap result2 0 0 1280 720 2>/dev/null | head -8)
  if echo "$RES" | grep -qi "Invalid\|unreadable\|error"; then
    echo "GAME DID NOT START (error dialog: $(echo "$RES" | tr '\n' ' ' | head -c 120))" >> "$OUT"
  else
    echo "GAME DID NOT START (no error dialog)" >> "$OUT"
  fi
fi
echo "DONE" >> "$OUT"
exit 0
