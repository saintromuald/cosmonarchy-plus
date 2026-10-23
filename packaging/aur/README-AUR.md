# Cosmonarchy Plus (AUR)

One-command install/update for Arch users, running the same build the Windows
installer ships, through the same umu + GE-Proton stack the dev launcher uses.

## What it contains

- The pinned Cosmonarchy Plus game build (Release tree) from the GitHub release
- `zz-cosmo-plus.qdp`, `zz-cosmo-rollback.qdp`, rollback-capable `CrownLink.snp`
- `cosmonarchy-plus [solo|host|guest]` launcher + three .desktop entries
- The Wine tinkering that made the game work, applied automatically:
  - Windows 10 mode
  - `ddraw` = `native,builtin` (so cnc-ddraw loads)
  - cnc-ddraw copied next to the game exe

## Submitting / updating the AUR package

The AUR git repo contains: `PKGBUILD`, `cosmonarchy-plus.sh`, the three
`.desktop` files, `cosmonarchy-plus.install`, and `provision-prefix.sh` (copied
from `packaging/common/`). Assemble it like this from the source repo:

```bash
cd packaging/aur
cp ../common/provision-prefix.sh .
updpkgsums            # fills the sha256sums after bumping pkgver
makepkg -si           # local test
# then push to the AUR repo (aur.git) with .SRCINFO
makepkg --printsrcinfo > .SRCINFO
```

Bump `pkgver` to the release tag each time a release is cut; the asset URLs
are derived from it.

## First-run behaviour (all automatic)

1. Mirrors the pristine tree into `~/.local/share/cosmonarchy-plus/` (so the
   game can write logs/saves; `/opt` stays pristine).
2. Copies `Starcraft` from an existing Cosmonarchy install (search list in the
   launcher; override with `COSMO_PLUS_STARCRAFT_SRC`).
3. Provisions the Wine prefix (`~/.local/share/cosmonarchy-plus/prefix`):
   wineboot, Windows 10, ddraw override, cnc-ddraw placement.
4. Launches via `umu-run` with GE-Proton (`proton-ge-custom-bin`).
