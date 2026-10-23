=====================================================================
 COSMONARCHY PLUS - WINDOWS INSTALLER & UPDATER
=====================================================================

WHAT THIS IS
------------
Installs Cosmonarchy Plus (the Plus gameplay plugin + optional rollback
netcode) and keeps it updated from the project's releases.

Your normal Cosmonarchy is never touched. Plus lives in its own folder
("Cosmonarchy Plus", next to your existing install) and has its own
shortcuts. Fraud Launcher updates keep working for the original.

WHAT YOU NEED
-------------
An existing Cosmonarchy install (the folder containing "Release" and
"Starcraft"). If you play Cosmonarchy already, you have this.

HOW TO INSTALL
--------------
  1. Close Cosmonarchy if it is running.
  2. Double-click CosmonarchyPlus.cmd.
     (SmartScreen? "More info" -> "Run anyway". The scripts are plain
     text; open them in Notepad to check what they do.)
  3. If your Cosmonarchy folder was found, just click Install.
     Otherwise click Browse and point at it (the folder containing
     "Release" and "Starcraft").
  4. Wait for the green "Done". The first install copies the base game
     and downloads the build - this can take a few minutes.
  5. Play with the new desktop shortcuts:
       Cosmonarchy Plus            Solo (Plus gameplay, no netcode)
       Cosmonarchy Plus (Host)     rollback match, you create the lobby
       Cosmonarchy Plus (Guest)    rollback match, you join the lobby

HOW TO UPDATE
-------------
Run CosmonarchyPlus.cmd again and click the button. It only downloads
what changed - usually a ~2 MB plugin update; the big game build is
re-downloaded only when the project moves its pinned build.

HOW TO CHECK IT IS THE PLUS VERSION
-----------------------------------
The build hash bottom-right of the main menu must be the one shown in
the updater after install. Rollback matches additionally write
Release\cosmo_rollback.log ("armed from cosmo_rollback.conf",
"hashes agree").

TROUBLESHOOTING
---------------
"Not a Cosmonarchy folder"
  You selected the wrong folder. It must contain "Release" and
  "Starcraft" as subfolders.

Update download fails
  The updater verifies checksums and refuses a bad download. Just run
  it again. Log: %TEMP%\cosmonarchy-plus.log

SmartScreen warning
  The installer is an unsigned script. "More info" -> "Run anyway".

A new updater version is out
  The installer tells you the latest version before you click. When the
  installer itself gets new features, re-download the Setup zip from
  the latest release:
  https://github.com/saintromuald/cosmonarchy-plus/releases/latest

HOW TO REMOVE IT
----------------
Run the installer, click "Remove Cosmonarchy Plus...". That deletes the
"Cosmonarchy Plus" folder and its three shortcuts. Your normal install
is untouched.
=====================================================================
