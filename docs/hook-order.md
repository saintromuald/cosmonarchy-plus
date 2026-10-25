# Hook order and ownership

The Cosmonarchy Plus stack shares one process and one set of hook sites. This
document is the contract; `cosmo_plus.state.json` (hooks section) and the boot
self-test enforce it.

## Load order (fixed by the game's hotloader)

1. `gptp.qdp` — the mod's C++ core: draw hooks, `helper::bw_*` stubs, the
   widescreen/zoom layer, and the `samase` plugin surface Plus consumes.
2. `zz-cosmo-plus.qdp` — this crate. Installs after gptp, before rollback.
3. `zz-cosmo-rollback.qdp` — netcode; keeps the outermost frame trampoline.

The order matters in two places:

- **Draw proc** (`g_draw_game_proc`, operand at `0x004BD68D`): Plus reads the
  current pointer, saves it (`OLD_DRAW_PROC`) and writes its own hook; the hook
  calls the saved pointer first and draws on top. At install Plus *audits* the
  pointer it chained: it must fall inside `gptp.qdp`'s loaded image. Anything
  else logs a warning and records the owner in the state report.
- **Keyboard hooks** (`WH_KEYBOARD`, thread-local): later installs run first.
  Plus installs four: idle-workers, Tab subgroup cycle, larva-pick entry,
  CapsLock camera pan. Each is registered in the state report
  (`keyboard:*`); the boot self-test requires at least the first three.

## Ownership table

| Hook site | Owner | Plus role |
| --- | --- | --- |
| `g_draw_game_proc` `0x004BD68D` | gptp (chained) | draws badges/overlay after the game |
| Step objects (frame) | samase_shim → gptp | data pass, antimeta, tab rotation |
| `WH_KEYBOARD` × 4 | Plus (thread hooks) | idle key, Tab, larva entry, cam pan |
| `gptp.qdp` zoom site | Plus (verified 5-byte patch) | player zoom floor |
| Vectored exception handler | Plus | crash self-capture |

Rules:

- Plus never writes `gptp.qdp` bytes except the verified zoom immediate, and
  verifies both the signature and the read-back before/after writing.
- Plus never installs a hook without recording it (`state::hook`).
- A hook site whose owner does not match the table is a **boot warning + state
  report entry**, never a silent failure.

## Recovering "a plugin stopped working"

Run `tools/cosmo-check.sh [build]`. It prints the latest boot report: globals
that fail plausibility, features that disarmed and why, config keys, and hook
owners. A regression is a diff between two of those reports.
