#!/bin/bash
# One-shot headless test: boot game, navigate to ASL Test, load map, check counter.
# Usage: test_cycle.sh <display> <map-row-y> <mapname-grep> <outfile>
set -u
DISP=$1
ROWY=$2
MAPNAME=$3
OUT=$4
PFX="/home/saintbellarmine/Games/cosmonarchy-agent2/prefix"
GAME="$PFX/drive_c/Cosmonarchy/Release/Cosmonarchy BW.exe"
LOG="$PFX/drive_c/Cosmonarchy/Release/cosmo_plus.log"
OCR="/home/saintbellarmine/Projects/cosmonarchy-rollback/tools/ocr_text.py"

kill_old() {
  WINEPREFIX="$PFX" wineserver -k >/dev/null 2>&1
  pgrep -f "Cosmonarchy.BW" | xargs -r kill -9 2>/dev/null
  pgrep -x Xvfb | xargs -r kill -9 2>/dev/null
  sleep 2
}

snap() { # snap <file> <x> <y> <w> <h>
  DISPLAY=:$DISP import -window root /tmp/opencode/cyc/$1.png 2>/dev/null
  python3 "$OCR" /tmp/opencode/cyc/$1.png "$2" "$3" "$4" "$5" 6 2>/dev/null
}

mkdir -p /tmp/opencode/cyc
kill_old
rm -f /tmp/.X${DISP}-lock
Xvfb :$DISP -screen 0 1280x720x24 -nolisten tcp >/tmp/opencode/cyc/xvfb.log 2>&1 &
sleep 3
cd "$PFX/drive_c/Cosmonarchy/Release" || exit 1
WINEPREFIX="$PFX" PROTONPATH=GE-Proton GAMEID=umu-cosmo-cyc$DISP PROTON_VERB=waitforexitandrun \
  DISPLAY=:$DISP WAYLAND_DISPLAY= setsid umu-run "C:\\Cosmonarchy\\Release\\Cosmonarchy BW.exe" \
  >/tmp/opencode/cyc/game.log 2>&1 </dev/null &
GAMEPID=$!

# wait for main menu (up to 300s)
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

# navigate: Single Player(490,200) -> Return -> Play Custom(320,585) -> campaign(320,201) -> ASL Test(320,200)
DISPLAY=:$DISP xdotool mousemove 490 200 click 1; sleep 4
DISPLAY=:$DISP xdotool key Return; sleep 5
DISPLAY=:$DISP xdotool mousemove 320 585 click 1; sleep 5
DISPLAY=:$DISP xdotool mousemove 320 201 click --repeat 2 --delay 150 1; sleep 4
DISPLAY=:$DISP xdotool mousemove 320 200 click --repeat 2 --delay 150 1; sleep 4
snap folder 200 150 700 400 | head -6 >> "$OUT"

# dismiss any lingering dialog, then click the map row
DISPLAY=:$DISP xdotool key Return; sleep 2
DISPLAY=:$DISP xdotool mousemove 340 $ROWY click --repeat 2 --delay 150 1; sleep 4
DISPLAY=:$DISP xdotool key Return; sleep 4
DISPLAY=:$DISP xdotool key Return
BEFORE=$(grep -cE "anti-meta: game type" "$LOG" 2>/dev/null || echo 0)
sleep 18
AFTER=$(grep -cE "anti-meta: game type" "$LOG" 2>/dev/null || echo 0)
echo "counter before=$BEFORE after=$AFTER (map row y=$ROWY)" >> "$OUT"
snap result 0 0 1280 720 | head -6 >> "$OUT"
echo "DONE" >> "$OUT"
exit 0
