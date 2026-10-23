@echo off
rem Cosmonarchy Plus installer/updater - double-click entry point.
rem Launches the PowerShell app with the execution policy relaxed for this run
rem (the script is plain text; open it in Notepad if you want to check it).
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0CosmonarchyPlus.ps1"
