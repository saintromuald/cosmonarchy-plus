#!/usr/bin/env python3
"""Generate the "Anti Meta" lobby template from the stock Melee template.

The `.got` format (deduced by diffing the shipped templates):
  0x00        u8   name length
  0x01..0x1C  char name[28]
  0x40        u16  game type (2 = Melee, 0x1E = Unranked, ...)
  0x42        u16  subtype
  0x44..      the rest of the template settings, copied verbatim from Melee

The type byte is a *marker* Plus reads at game start (`*g_game_type`). Vanilla
`is_melee_game` only accepts types present in its GameTypeList, so this template is
useful only once Plus (or a GameTypeList entry) handles the marker; until then the
mode is selected with `CM_PLUS_MODE=antimeta` in `cosmo_plus.conf`.

Usage:
  tools/make-template.py <path-to-Meleep.got> <out.got> [type]
  tools/make-template.py --install          # copy into the default game tree
"""
import shutil
import struct
import sys
from pathlib import Path

NAME = "Anti Meta"
MARKER_TYPE = 45
GAME_ROOT = Path.home() / "Games/cosmonarchy/prefix/drive_c/Cosmonarchy/Release"
SRC = GAME_ROOT / "mpq/templates/Melee(1).got"
OUT = GAME_ROOT / "mpq/templates/Anti Meta(1).got"
LST = GAME_ROOT / "mpq/templates/templates.lst"


def build(src: Path, dst: Path, gtype: int = MARKER_TYPE) -> None:
    data = bytearray(src.read_bytes())
    if len(data) < 0x61:
        raise SystemExit(f"{src}: too short ({len(data)} bytes)")
    name = NAME.encode("ascii")
    if len(name) > 27:
        raise SystemExit("name too long")
    data[0] = len(name)
    data[1:29] = b"\0" * 28
    data[1 : 1 + len(name)] = name
    struct.pack_into("<H", data, 0x40, gtype)
    struct.pack_into("<H", data, 0x42, 1)
    dst.write_bytes(data)


def install() -> None:
    if not SRC.exists():
        raise SystemExit(f"missing stock template: {SRC}")
    build(SRC, OUT)
    text = LST.read_text()
    if OUT.name not in text:
        if not text.endswith("\n"):
            text += "\n"
        text += f'"{OUT.name}"\n'
        LST.write_text(text)
    print(f"wrote {OUT}")
    print(f"updated {LST}")


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--install":
        install()
    elif len(sys.argv) >= 3:
        build(Path(sys.argv[1]), Path(sys.argv[2]),
              int(sys.argv[3], 0) if len(sys.argv) > 3 else MARKER_TYPE)
        print(f"wrote {sys.argv[2]}")
    else:
        raise SystemExit(__doc__)
