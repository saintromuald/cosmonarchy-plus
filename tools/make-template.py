#!/usr/bin/env python3
"""Generate the "Anti Meta" lobby template from the stock Melee template.

The `.got` format (deduced from the shipped templates and confirmed against the
template loader in StarCraft.exe 1.16.1, funcs at 0x4AB750/0x4AB150):

  0x00          u8    version marker; every shipped template has 3 (the loader rejects
                       anything else: `cmp cl, 3` at 0x4AB7B1)
  0x01..0x20    char  name block 1 (null-terminated, 32 bytes) — the string shown in the
                       Create Game game-type dropdown
  0x21..0x40    char  name block 2 (null-terminated, 32 bytes) — secondary label,
                       appended to block 1 in the chat-room label when non-empty
  0x41          u8    game type byte  (-> GameTypeList node +0x48 -> g_game_data.got_game_type)
  0x42          u8    unused (0)
  0x43..0x44    u16   subtype, little-endian (-> node +0x4A). Only templates whose
                       subtype == 1 are listed in the dropdown (compile_list 0x4AAD00);
                       the loader rejects subtype >= 8
  0x45..0x48    4 B   unused (0)
  0x49          u8    "mode" byte (-> node +0x50). 1 makes `is_melee_game` (0x4AAFA0)
                       return true; it is also `g_game_data.victory_condition` (+0x75):
                       1 == VictoryCondition::Melee. Inherited from Melee unchanged.
  0x4A..0x60    ..    the rest of the settings block, copied verbatim from Melee.

Everything except the name and the game type byte is copied verbatim from `Melee(1).got`,
so the mode inherits Melee's victory condition, starting units, fog and team settings.

The template only shows up if `templates.lst` lists it (the engine enumerates that file);
this script's `--install` appends the entry, and `cosmo-variant` stages both files at
launch time.

Usage:
  tools/make-template.py <Melee(1).got> <out.got> [type]
  tools/make-template.py --install [Release|Prerelease]   # game tree install + lst entry
"""
import struct
import sys
from pathlib import Path

NAME = "Anti Meta"
MARKER_TYPE = 46  # 45 is taken by Mindshare; see docs/antimeta-mode-notes.md
GAME_ROOT = Path.home() / "Games/cosmonarchy/prefix/drive_c/Cosmonarchy"


def build(src: Path, dst: Path, gtype: int = MARKER_TYPE) -> None:
    data = bytearray(src.read_bytes())
    if len(data) < 0x61:
        raise SystemExit(f"{src}: too short ({len(data)} bytes)")
    if data[0] != 3:
        raise SystemExit(f"{src}: unexpected version byte {data[0]} (expected 3)")
    if not (0 < gtype < 0x80):
        raise SystemExit(f"game type {gtype} out of range (1..127)")

    name = NAME.encode("ascii")
    if len(name) > 31:
        raise SystemExit("name too long")
    data[1:0x21] = b"\0" * 0x20  # name block 1 (32 bytes)
    data[1 : 1 + len(name)] = name
    data[0x41] = gtype  # game type byte
    data[0x42] = 0
    data[0x43:0x45] = struct.pack("<H", 1)  # subtype 1 (only value the dropdown lists)
    dst.write_bytes(data)


def update_lst(lst: Path, got_name: str) -> bool:
    """Insert the template right after the Unranked entry (or append), idempotently."""
    text = lst.read_text()
    line = f'"{got_name}"'
    if line in text:
        return False
    lines = text.splitlines()
    out = []
    inserted = False
    for l in lines:
        out.append(l)
        if not inserted and '"Unranked(1).got"' in l:
            out.append(line)
            inserted = True
    if not inserted:
        out.append(line)
    lst.write_text("\n".join(out) + "\n")
    return True


def install(build_name: str = "Release") -> None:
    root = GAME_ROOT / build_name
    tpl = root / "mpq/templates"
    src = tpl / "Melee(1).got"
    if not src.exists():
        raise SystemExit(f"missing stock template: {src}")
    out = tpl / f"{NAME}(1).got"
    build(src, out)
    lst = tpl / "templates.lst"
    changed = update_lst(lst, out.name)
    print(f"wrote {out}")
    print(f"{'updated' if changed else 'already listed in'} {lst}")


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--install":
        install(sys.argv[2] if len(sys.argv) > 2 else "Release")
    elif len(sys.argv) >= 3:
        build(Path(sys.argv[1]), Path(sys.argv[2]),
              int(sys.argv[3], 0) if len(sys.argv) > 3 else MARKER_TYPE)
        print(f"wrote {sys.argv[2]}")
    else:
        raise SystemExit(__doc__)
