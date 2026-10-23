#!/bin/bash
# Stage one map at a time into the agent2 rig's test folder and run the headless
# in-game load test. Sequential (one wine prefix).
set -u
R=/tmp/opencode/ums-port
OUTDIR=$R/logs/gametests
TESTDIR=/home/saintbellarmine/Games/cosmonarchy-agent2/prefix/drive_c/Cosmonarchy/Starcraft/Maps/CMBW-Root/CMBW-maps/test
DISP=8

mkdir -p "$OUTDIR"
SAMPLES=(
"Strategy/Desert Strike Classic ....scx"
"Arena/Income Arena.scx"
"TowerDefense/EUD CoronaTeamTowerDef 4.21.scx"
"Puzzle/Zerg Soccer.scx"
"TowerDefense/bunker wars mass attack.scx"
"Puzzle/03 The Trump Card.scx"
"Other/firing line.scx"
"Defense/ind random d gold v 2.scx"
"Micro/random unit control v1 9 up.scx"
"TowerDefense/p o k er t em p le.scx"
"Other/vortex turret pro.scx"
"Other/u the lost temple.scx"
"Other/8 nukes away.scx"
)
START=${1:-0}
for i in $(seq $START $(( ${#SAMPLES[@]} - 1 ))); do
  FILE="${SAMPLES[$i]}"
  SRC="$R/out/CMPlus/$FILE"
  [ -f "$SRC" ] || { echo "[$i] MISSING $SRC" | tee -a "$OUTDIR/summary.txt"; continue; }
  rm -f "$TESTDIR"/!ums-test.scx
  cp "$SRC" "$TESTDIR/!ums-test.scx"
  OUTF="$OUTDIR/test_$i.out"
  echo "[$i] testing $FILE" | tee -a "$OUTDIR/summary.txt"
  bash "$R/tools/test_ums.sh" "$DISP" "ums-test" "$OUTF"
  grep -E "GAME STARTED|GAME DID NOT START|NOT FOUND|NOT REACHED" "$OUTF" >> "$OUTDIR/summary.txt"
  echo >> "$OUTDIR/summary.txt"
done
echo "ALL DONE" | tee -a "$OUTDIR/summary.txt"
