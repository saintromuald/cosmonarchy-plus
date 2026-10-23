# Releasing Cosmonarchy Plus

Every build is cut by GitHub Actions into **one release, one version**, published
to GitHub Releases. Windows and Mac users install/update by running a small
setup tool; Arch users update with `paru -Syu`. Nobody touches git, plugins, or
folders by hand.

```
release v0.2.0
├── CosmonarchyPlus-Setup-0.2.0.zip        Windows installer/updater (double-click)
├── CosmonarchyPlus-Mac-Setup-0.2.0.zip    macOS installer/updater (double-click)
├── CosmonarchyPlus-plugins-0.2.0.zip      ~2 MB: the two .qdp plugins + CrownLink.snp
├── CosmonarchyPlus-mod-0.2.0.zip          ~1.5 GB: the pinned game build (Release tree)
├── manifest.json                          version, pinned build SHA, hashes
└── SHA256SUMS.txt                         checksums of the zips
```

The two-zip split is what makes updates feel instant: the updaters re-download
the big mod zip only when `build_ref` in the manifest changed; a normal update
is just the plugins zip.

## One-time repository setup

The workflow expects three public GitHub repos. `cosmonarchy-bw-release` and
`CrownLink` already exist; `cosmonarchy-rollback` and this repo still need to be
created and pushed (this repo has no remote yet, and the `netcode` submodule
currently points at a local machine path).

```bash
gh auth login

# 1. rollback repo (the submodule URL in .gitmodules points here)
cd ~/Projects/cosmonarchy-rollback
git remote add origin https://github.com/saintromuald/cosmonarchy-rollback.git
gh repo create cosmonarchy-rollback --public --source . --push
# (commit any local work you want shipped first; cmscripts/ and farm.rs are
#  currently untracked)

# 2. this repo
cd ~/Projects/cosmonarchy-plus
git remote add origin https://github.com/saintromuald/cosmonarchy-plus.git
git submodule sync            # picks up the .gitmodules URL change
gh repo create cosmonarchy-plus --public --source . --push

# 3. push the pinned build branch (the workflow snapshots it)
git -C ~/Games/cosmonarchy/prefix/drive_c/Cosmonarchy/Release push mine plus:plus
```

## Pinning the build

`pins.json` is the single knob:

```json
{
  "build_repo": "saintromuald/cosmonarchy-bw-release",
  "build_ref": "plus",
  "crownlink_repo": "saintromuald/CrownLink",
  "crownlink_ref": "master"
}
```

`build_ref` is normally the `plus` branch (CI snapshots its head at release
time and records the resolved SHA in the manifest). Set it to a full commit SHA
to freeze. **Plugins are compiled in the same run that snapshots the build**, so
a release can never ship a plugin that does not match its game build — the
mismatch class of bugs is gone by construction.

## Cutting a release

```bash
# bump the version in VERSION and the crate version in plus/Cargo.toml
cd ~/Projects/cosmonarchy-plus
git tag v0.3.0
git push origin v0.3.0
```

The workflow builds both plugins (Linux runner, cargo xwin), builds the
rollback-capable `CrownLink.snp` (Windows runner, from the pinned CrownLink
ref), snapshots the pinned build, assembles the zips + manifest + checksums, and
publishes the release. Watch it under the Actions tab. Alternatively run it
manually there (`workflow_dispatch`, version defaults to the `VERSION` file).

If the release already exists (e.g. a re-run), the workflow uploads the assets
into it instead.

## After the release

- **AUR**: bump `pkgver` in `packaging/aur/PKGBUILD` to the new tag, run
  `updpkgsums`, test `makepkg -si`, then push to the AUR git repo with a fresh
  `.SRCINFO`. See `packaging/aur/README-AUR.md`.
- **Windows/Mac users**: nothing to do. Their installers poll
  `releases/latest`; next run updates them. If the *installer itself* gained
  features, users re-download the Setup zip from the latest release page.

## Distribution layout in this repo

```
.github/workflows/release.yml     CI that builds + publishes everything
pins.json                         what gets built against what
VERSION                           fallback version for manual runs
dist/windows/                     Windows installer/updater (ships in Setup zip)
dist/macos/                       macOS installer/updater (ships in Mac Setup zip)
packaging/common/provision-prefix.sh   the Wine tinkering, shared (AUR + macOS)
packaging/aur/                    AUR package (PKGBUILD, launcher, desktop files)
```

## Code signing / SmartScreen

The Windows updater is an unsigned script, so first runs see a SmartScreen
prompt ("More info" -> "Run anyway"); the README in the Setup zip walks users
through it. When distribution grows, sign the artifacts with an OV code-signing
certificate (kills the warning entirely) and/or submit a winget manifest so
users can `winget install` and get automatic updates through Windows' own
stack.
