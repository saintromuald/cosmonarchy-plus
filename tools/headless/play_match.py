#!/usr/bin/env python3
"""Drive the Cosmonarchy menus headlessly to start a solo match.

The game window is 1920x1080 with the 4:3 play area centered (offset x=240,
scale 2.25): main menu -> Single Player -> profile Ok -> Play Custom ->
melee -> 1v1 -> Class 0 -> Standard-Bearers -> map Ok -> tip Ok.

Takes a screenshot after every step so a failing step is obvious.
"""
import subprocess
import sys
import time

HARNESS = "/tmp/opencode/harness"

# (label, x, y, clicks)
STEPS = [
    ("01-single-player", 730, 266, 1),
    ("02-profile-ok", 1400, 882, 1),
    ("03-play-custom", 550, 882, 1),
    ("04-melee", 480, 402, 2),
    ("05-1v1", 440, 312, 2),
    ("06-class0", 660, 442, 2),
    ("07-standard-bearers", 540, 312, 2),
    ("08-map-ok", 1400, 882, 1),
    ("09-tip-ok", 680, 586, 1),
]


def sh(cmd):
    return subprocess.run(cmd, shell=True, capture_output=True, text=True)


def find_window(display: str) -> str:
    out = sh(f"DISPLAY=:{display} xdotool search --name 'Cosmonarchy'")
    ids = [l for l in out.stdout.splitlines() if l.strip()]
    return ids[0].strip() if ids else ""


def main(display: str) -> int:
    win = find_window(display)
    if not win:
        # The window can take a while to appear after the plugin reports in.
        for _ in range(20):
            time.sleep(2)
            win = find_window(display)
            if win:
                break
    if not win:
        print("no game window", file=sys.stderr)
        return 1
    sh(f"DISPLAY=:{display} xdotool windowfocus {win}")
    time.sleep(1)
    for i, (label, x, y, clicks) in enumerate(STEPS):
        if clicks == 2:
            sh(
                f"DISPLAY=:{display} xdotool mousemove --window {win} {x} {y} "
                f"click --repeat 2 --delay 120 1"
            )
        else:
            sh(f"DISPLAY=:{display} xdotool mousemove --window {win} {x} {y} click 1")
        time.sleep(2.5)
        sh(f"DISPLAY={display} import -window {win} {HARNESS}/{label}.png")
        print(f"{label} done")
    # Give the match a few seconds to load before the caller takes over.
    time.sleep(6)
    return 0


if __name__ == "__main__":
    if len(sys.argv) != 2:
        print(f"usage: {sys.argv[0]} <display>", file=sys.stderr)
        sys.exit(2)
    sys.exit(main(sys.argv[1]))
