=====================================================================
 COSMONARCHY PLUS - macOS INSTALLER & UPDATER
=====================================================================

WHAT THIS IS
------------
Installs Cosmonarchy Plus into its own folder and keeps it updated:

  ~/Library/Application Support/CosmonarchyPlus/

Your normal Cosmonarchy is never touched. Three launchers appear on
your Desktop:

  Cosmonarchy Plus (Solo).command   Plus gameplay, no netcode
  Cosmonarchy Plus (Host).command   rollback match, you create the lobby
  Cosmonarchy Plus (Guest).command  rollback match, you join the lobby

Running the installer again later = update (it only downloads what
changed).

WHAT YOU NEED
-------------
- A Mac. Apple Silicon (M1/M2/M3/M4) is recommended.
- A Wine runtime, used only to run the game:
    Apple Silicon: Whisky (free) - https://getwhisky.app
                   Open it once so it downloads its Wine, then close it.
                   (CrossOver also works.)
    Intel:         wine-stable (brew install --cask wine-stable)
- An existing Cosmonarchy install, to copy the base game (Starcraft)
  from. The installer finds it or asks you to pick the folder.

HOW TO INSTALL
--------------
  1. Close Cosmonarchy if it is running.
  2. Right-click install-cosmonarchy-plus.command -> Open.
     (Gatekeeper asks the first time: Open again. Or run from Terminal:
      bash install-cosmonarchy-plus.command)
  3. Answer the one question if it asks for your Cosmonarchy folder.
  4. Wait for "Cosmonarchy Plus <version> is installed." The installer
     also boots the game once to verify it works.
  5. Play with the Desktop launchers.

WHAT THE INSTALLER SETS UP (the Wine tinkering, done for you)
-------------------------------------------------------------
- A Wine prefix with Windows 10 mode
- ddraw = native,builtin  (so the game uses cnc-ddraw)
- cnc-ddraw copied next to the game exe
- The rollback netcode armed only for Host/Guest launches

TROUBLESHOOTING
---------------
"The game did not stay running" / bad CPU type in the boot log
  Your Wine cannot run 32-bit Windows apps (Cosmonarchy is 32-bit).
  Apple Silicon: install Whisky and re-run - its Wine is used.
  Intel: install wine-stable.

Black screen in-game
  Edit Release/ddraw.ini in the CosmonarchyPlus game folder and set
  renderer=gdi (slow but always works), or try renderer=opengl.

Gatekeeper blocks the .command file
  Right-click -> Open, or: xattr -dr com.apple.quarantine *.command

Update failed mid-download
  Run the installer again. Downloads are checksum-verified.

HOW TO REMOVE IT
----------------
Delete the folder ~/Library/Application Support/CosmonarchyPlus and the
three Desktop .command files. Your normal Cosmonarchy is untouched.
=====================================================================
