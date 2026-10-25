# The development loop (why you never restart from scratch)

The ground floor is `cosmo-plus-core` (pure, host-tested) — every change that
can live there must live there. The engine-facing crate is a thin adapter.
Pick the fastest loop that fits the change:

| Change kind | Loop |
| --- | --- |
| Pure logic (idle predicate, saturation, hotkeys, subgroup rules) | edit `cosmo-plus-core` → `cargo test -p cosmo-plus-core` — no game at all |
| Config values (`cosmo_plus.conf`, `plus.yml`) | edit the conf in the game dir → in-game `tools/cosmo-repl reload-config` — no restart |
| UI position / probe anything on screen | `tools/cosmo-repl "badge 40 280"`, `"text hello 300 100"`, `"icon 7 200 200 24 22"` on a running game (launched with `CM_PLUS_REPL=1`) |
| Engine-facing code (hooks, drawing, selection) | `tools/build.sh` → `tools/cosmo-restart.sh` — one command, no menu clicking |
| Before commit / release | `tools/verify-plus.sh` — host tests + cross build + isolated headless boot + boot report + self-test verdict |
| "Which feature stopped working?" | `tools/cosmo-check.sh [build]` — boot report diff |

## Why this replaces the old loop

The old loop crossed three expensive, fragile boundaries on every change: a
942 MB `temp.mpq` hotloader rebuild, a full game relaunch, and five menu
clicks to reach a match. The new loop crosses zero of them for most changes:

- The idle-worker predicate and its friends are pure functions over a
  `UnitView` — tested against fake units on the host.
- Config changes re-arm in place (`reload.rs`).
- UI experiments go through the polled REPL (`cosmo_plus.cmd`), which draws
  transient probes directly on the live frame.
- Code changes use `cosmo-restart.sh` — the *only* loop that still relaunches,
  and it skips the manual menu walk.

## In-place DLL hot-swap: why it is not in this loop

Unloading the plugin DLL from inside the game (`FreeLibraryAndExitThread`)
is technically possible and all hooks are installed reversibly (each install
saves what it replaced), but *reloading* the new DLL is owned by the game's
hotloader, which loads plugins once per boot. Until the hotloader grows a
reload trigger, `cosmo-restart.sh` is the reload path. Everything else about
hot reload — config re-arm, reversible hooks, the REPL — is already in place,
so adding a real swap later is one hook, not a redesign.
