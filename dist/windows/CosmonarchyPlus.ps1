# Cosmonarchy Plus - installer and updater (Windows).
#
# One window, one button. It installs Cosmonarchy Plus into its own folder next
# to your normal Cosmonarchy (never touches it), keeps it updated from the
# project's GitHub releases, and makes three desktop shortcuts:
#
#   Cosmonarchy Plus             Solo (Plus gameplay, no rollback netcode)
#   Cosmonarchy Plus (Host)      rollback match, you create the lobby
#   Cosmonarchy Plus (Guest)     rollback match, you join the lobby
#
# Running it again later = update. The updater talks to
# https://api.github.com/repos/saintromuald/cosmonarchy-plus/releases/latest
# and only downloads what changed: usually the small plugins zip (~2 MB), and
# the big mod zip only when the pinned game build changed.
#
# Launch through CosmonarchyPlus.cmd so the execution policy does not block it.

$ErrorActionPreference = 'Stop'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$script:Repo = 'saintromuald/cosmonarchy-plus'
$script:SetupZip = 'CosmonarchyPlus-Setup-{0}.zip'
$script:PluginsZip = 'CosmonarchyPlus-plugins-{0}.zip'
$script:ModZip = 'CosmonarchyPlus-mod-{0}.zip'
$script:Here = Split-Path -Parent $MyInvocation.MyCommand.Path
$script:TempDir = Join-Path $env:TEMP 'cosmonarchy-plus'
$script:LogPath = Join-Path $env:TEMP 'cosmonarchy-plus.log'

function Write-Log([string]$message) {
    $line = '[{0}] {1}' -f (Get-Date -Format 'yyyy-MM-dd HH:mm:ss'), $message
    Add-Content -LiteralPath $script:LogPath -Value $line -Encoding UTF8
}

function Test-GameDir([string]$dir) {
    if ([string]::IsNullOrWhiteSpace($dir)) { return $false }
    return (Test-Path -LiteralPath (Join-Path $dir 'Release\Cosmonarchy BW.exe')) -and
           (Test-Path -LiteralPath (Join-Path $dir 'Starcraft'))
}

# Known locations first, then a shallow scan of each fixed drive. The game is
# usually under a folder called Cosmonarchy (or Cosmonarchy-Rollback), one or
# two levels deep.
function Find-GameDirs {
    $found = @()
    $candidates = @(
        (Join-Path $env:USERPROFILE 'Documents\Cosmonarchy'),
        (Join-Path $env:USERPROFILE 'Cosmonarchy'),
        'C:\Cosmonarchy',
        'D:\Cosmonarchy',
        (Join-Path $env:LOCALAPPDATA 'Cosmonarchy'),
        (Join-Path $env:ProgramFiles 'Cosmonarchy')
    )
    foreach ($c in $candidates) { if (Test-GameDir $c) { $found += $c } }

    foreach ($drive in [System.IO.DriveInfo]::GetDrives()) {
        if ($drive.DriveType -ne 'Fixed' -or -not $drive.IsReady) { continue }
        foreach ($base in @((Join-Path $drive.RootDirectory.FullName 'Games'), $drive.RootDirectory.FullName)) {
            if (-not (Test-Path -LiteralPath $base)) { continue }
            foreach ($sub in (Get-ChildItem -LiteralPath $base -Directory -ErrorAction SilentlyContinue)) {
                if ($sub.Name -notmatch 'osmonarch') { continue }
                if (Test-GameDir $sub.FullName) { $found += $sub.FullName }
                foreach ($sub2 in (Get-ChildItem -LiteralPath $sub.FullName -Directory -ErrorAction SilentlyContinue)) {
                    if (Test-GameDir $sub2.FullName) { $found += $sub2.FullName }
                }
            }
        }
    }
    return ($found | Select-Object -Unique)
}

function Get-InstalledState([string]$target) {
    $p = Join-Path $target 'plus-state.json'
    if (Test-Path -LiteralPath $p) {
        try { return Get-Content -LiteralPath $p -Raw | ConvertFrom-Json } catch { }
    }
    return $null
}

function Set-InstalledState([string]$target, $manifest) {
    $state = @{ version = $manifest.version; build_ref = $manifest.build_ref; installed = (Get-Date -Format 'yyyy-MM-dd HH:mm') }
    Set-Content -LiteralPath (Join-Path $target 'plus-state.json') -Value ($state | ConvertTo-Json) -Encoding UTF8
}

function Get-LatestRelease {
    $headers = @{ 'User-Agent' = 'CosmonarchyPlusUpdater' }
    $rel = Invoke-RestMethod -Uri "https://api.github.com/repos/$($script:Repo)/releases/latest" -Headers $headers
    $manifestAsset = $rel.assets | Where-Object { $_.name -eq 'manifest.json' } | Select-Object -First 1
    if (-not $manifestAsset) { throw 'the latest release has no manifest.json' }
    $manifest = Invoke-RestMethod -Uri $manifestAsset.browser_download_url -Headers $headers
    $manifest | Add-Member -NotePropertyName 'assetUrls' -NotePropertyValue @{}
    foreach ($a in $rel.assets) { $manifest.assetUrls[$a.name] = $a.browser_download_url }
    return $manifest
}

# Downloads with a live status callback; the UI stays responsive.
function Download-Asset([string]$name, [string]$url, [string]$dest, [scriptblock]$onProgress) {
    New-Item -ItemType Directory -Path (Split-Path -Parent $dest) -Force | Out-Null
    if ($onProgress) { & $onProgress ("downloading {0} ..." -f $name) }
    $wc = New-Object System.Net.WebClient
    $wc.Headers.Add('User-Agent', 'CosmonarchyPlusUpdater')
    if ($onProgress) {
        $wc.DownloadProgressChanged += {
            $pct = 0
            if ($_.TotalBytesToReceive -gt 0) { $pct = [int](100 * $_.BytesReceived / $_.TotalBytesToReceive) }
            & $onProgress ("downloading {0} ... {1}%" -f $name, $pct)
        }
    }
    $script:dlDone = $false
    $script:dlError = $null
    $wc.DownloadFileCompleted += {
        $script:dlError = $_.Error
        $script:dlDone = $true
    }
    $wc.DownloadFileAsync($url, $dest)
    while (-not $script:dlDone) {
        [System.Windows.Forms.Application]::DoEvents()
        Start-Sleep -Milliseconds 100
    }
    $wc.Dispose()
    if ($script:dlError) { throw ("download failed for {0}: {1}" -f $name, $script:dlError.Message) }
}

function Expand-Zip([string]$zip, [string]$dest) {
    New-Item -ItemType Directory -Path $dest -Force | Out-Null
    # tar.exe (bsdtar) ships with Windows 10+ and is much faster than Expand-Archive.
    $tar = Get-Command tar.exe -ErrorAction SilentlyContinue
    if ($tar) {
        & $tar.Source -xf $zip -C $dest
        if ($LASTEXITCODE -ne 0) { throw ('extract failed: {0} (tar exit {1})' -f $zip, $LASTEXITCODE) }
    } else {
        Expand-Archive -LiteralPath $zip -DestinationPath $dest -Force
    }
}

# The two lines that arm the rollback netcode. The plugin is inert without a
# cosmo_rollback.conf next to the game exe, so Solo simply has none.
function Write-ConfTemplates([string]$releaseDir) {
    $hostConf = "CM_ROLLBACK_LIVE=1`r`nCM_ROLLBACK_SLOT=0`r`n"
    $guestConf = "CM_ROLLBACK_LIVE=1`r`nCM_ROLLBACK_SLOT=1`r`n"
    [IO.File]::WriteAllText((Join-Path $releaseDir 'cosmo_rollback.conf.host'), $hostConf)
    [IO.File]::WriteAllText((Join-Path $releaseDir 'cosmo_rollback.conf.guest'), $guestConf)
}

# Shortcuts point here, not at the exe: the role argument picks which config is
# armed right before the game starts. No dialogs, nothing to get wrong.
function Write-LaunchCmd([string]$target) {
    $cmd = @'
@echo off
rem Cosmonarchy Plus launcher - maintained by the installer/updater.
cd /d "%~dp0"
if /i "%~1"=="solo"  del /q "Release\cosmo_rollback.conf" >nul 2>&1
if /i "%~1"=="host"  copy /y "Release\cosmo_rollback.conf.host"  "Release\cosmo_rollback.conf" >nul
if /i "%~1"=="guest" copy /y "Release\cosmo_rollback.conf.guest" "Release\cosmo_rollback.conf" >nul
cd /d "%~dp0Release"
start "" "Cosmonarchy BW.exe"
exit /b
'@
    # CRLF line endings: this file is consumed by cmd.exe.
    [IO.File]::WriteAllText((Join-Path $target 'CosmonarchyPlus-launch.cmd'), ($cmd -replace "`n", "`r`n"))
}

function New-Shortcut([string]$target, [string]$label, [string]$arg) {
    $lnkPath = Join-Path ([Environment]::GetFolderPath('Desktop')) ($label + '.lnk')
    $shell = New-Object -ComObject WScript.Shell
    $s = $shell.CreateShortcut($lnkPath)
    $s.TargetPath = Join-Path $target 'CosmonarchyPlus-launch.cmd'
    $s.Arguments = $arg
    $s.WorkingDirectory = $target
    $icon = Join-Path $target 'Release\Cosmonarchy BW.exe'
    if (Test-Path -LiteralPath $icon) { $s.IconLocation = $icon + ',0' }
    $s.Description = 'Cosmonarchy Plus (' + $arg + ')'
    $s.Save()
    Write-Log "created shortcut $lnkPath"
}

function New-Shortcuts([string]$target) {
    New-Shortcut $target 'Cosmonarchy Plus' 'solo'
    New-Shortcut $target 'Cosmonarchy Plus (Host)' 'host'
    New-Shortcut $target 'Cosmonarchy Plus (Guest)' 'guest'
}

# Copies the payload files from an extracted plugins zip into the game folder.
function Apply-PluginsPayload([string]$payloadDir, [string]$target) {
    $pluginsDst = Join-Path $target 'Release\plugins'
    New-Item -ItemType Directory -Path $pluginsDst -Force | Out-Null
    foreach ($qdp in @('zz-cosmo-plus.qdp', 'zz-cosmo-rollback.qdp')) {
        $src = Join-Path $payloadDir ('plugins\' + $qdp)
        if (-not (Test-Path -LiteralPath $src)) { throw ('missing ' + $qdp + ' in payload') }
        Copy-Item -LiteralPath $src -Destination (Join-Path $pluginsDst $qdp) -Force
    }

    $snpSrc = Join-Path $payloadDir 'Starcraft\CrownLink.snp'
    if (Test-Path -LiteralPath $snpSrc) {
        $snpDst = Join-Path $target 'Starcraft\CrownLink.snp'
        $backup = Join-Path $target 'Starcraft\CrownLink.snp.stock-backup'
        if ((Test-Path -LiteralPath $snpDst) -and -not (Test-Path -LiteralPath $backup)) {
            Copy-Item -LiteralPath $snpDst -Destination $backup
            Write-Log 'backed up stock CrownLink.snp'
        }
        Copy-Item -LiteralPath $snpSrc -Destination $snpDst -Force
    }

    # Verify by size; a truncated copy is the one failure that looks like a bug.
    foreach ($c in @(
        @{ P = (Join-Path $pluginsDst 'zz-cosmo-plus.qdp'); S = (Join-Path $payloadDir 'plugins\zz-cosmo-plus.qdp') },
        @{ P = (Join-Path $pluginsDst 'zz-cosmo-rollback.qdp'); S = (Join-Path $payloadDir 'plugins\zz-cosmo-rollback.qdp') }
    )) {
        $want = (Get-Item -LiteralPath $c.S).Length
        $got = (Get-Item -LiteralPath $c.P).Length
        if ($want -ne $got) { throw ('copy failed: {0} is {1} bytes, expected {2}' -f $c.P, $got, $want) }
    }
}

# Extracts a mod zip (contains Release/...) over the target folder.
function Apply-ModZip([string]$zip, [string]$target) {
    $staging = Join-Path $script:TempDir 'mod-extract'
    if (Test-Path -LiteralPath $staging) { Remove-Item -LiteralPath $staging -Recurse -Force }
    Expand-Zip $zip $staging
    $releaseSrc = Join-Path $staging 'Release'
    if (-not (Test-Path -LiteralPath (Join-Path $releaseSrc 'Cosmonarchy BW.exe'))) {
        throw 'mod zip does not contain Release\Cosmonarchy BW.exe'
    }
    $releaseDst = Join-Path $target 'Release'
    New-Item -ItemType Directory -Path $releaseDst -Force | Out-Null
    # robocopy /MIR so files removed upstream disappear here too, like the
    # git reset --hard the Linux launcher does. Local logs/saves survive
    # (they are not part of the zip and robocopy does not delete them).
    $rc = Start-Process -FilePath 'robocopy.exe' -ArgumentList @(
        "`"$releaseSrc`"", "`"$releaseDst`"", '/MIR', '/NFL', '/NDL', '/NJH', '/NJS', '/NP', '/R:1', '/W:1'
    ) -Wait -PassThru -NoNewWindow
    if ($rc.ExitCode -ge 8) { throw ("mod copy failed (robocopy exit {0})" -f $rc.ExitCode) }
}

function Start-Game([string]$target, [string]$role) {
    $launch = Join-Path $target 'CosmonarchyPlus-launch.cmd'
    if (-not (Test-Path -LiteralPath $launch)) { Write-LaunchCmd $target }
    Start-Process -FilePath $launch -ArgumentList $role -WorkingDirectory $target
}

# ---------------- UI ----------------

$form = New-Object System.Windows.Forms.Form
$form.Text = 'Cosmonarchy Plus - installer & updater'
$form.Size = New-Object System.Drawing.Size(620, 470)
$form.StartPosition = 'CenterScreen'
$form.FormBorderStyle = 'FixedDialog'
$form.MaximizeBox = $false

$title = New-Object System.Windows.Forms.Label
$title.Text = 'Cosmonarchy Plus'
$title.Font = New-Object System.Drawing.Font('Segoe UI', 14, [System.Drawing.FontStyle]::Bold)
$title.AutoSize = $true
$title.Location = New-Object System.Drawing.Point(16, 14)
$form.Controls.Add($title)

$sub = New-Object System.Windows.Forms.Label
$sub.Text = 'Your normal Cosmonarchy is never touched. Plus gets its own folder next to it.'
$sub.AutoSize = $false
$sub.Size = New-Object System.Drawing.Size(580, 34)
$sub.Location = New-Object System.Drawing.Point(18, 46)
$form.Controls.Add($sub)

$lblDir = New-Object System.Windows.Forms.Label
$lblDir.Text = 'Existing Cosmonarchy folder (contains Release and Starcraft):'
$lblDir.AutoSize = $true
$lblDir.Location = New-Object System.Drawing.Point(18, 88)
$form.Controls.Add($lblDir)

$txtDir = New-Object System.Windows.Forms.TextBox
$txtDir.Location = New-Object System.Drawing.Point(20, 110)
$txtDir.Size = New-Object System.Drawing.Size(470, 24)
$form.Controls.Add($txtDir)

$btnBrowse = New-Object System.Windows.Forms.Button
$btnBrowse.Text = 'Browse...'
$btnBrowse.Location = New-Object System.Drawing.Point(500, 108)
$btnBrowse.Size = New-Object System.Drawing.Size(96, 28)
$form.Controls.Add($btnBrowse)

$btnBrowse.Add_Click({
    $dlg = New-Object System.Windows.Forms.FolderBrowserDialog
    $dlg.Description = 'Select the Cosmonarchy folder (the one containing Release and Starcraft)'
    if ($dlg.ShowDialog() -eq 'OK') { $txtDir.Text = $dlg.SelectedPath }
})

$lblTarget = New-Object System.Windows.Forms.Label
$lblTarget.Text = 'Plus will be installed next to it, in a folder named "Cosmonarchy Plus".'
$lblTarget.AutoSize = $true
$lblTarget.Location = New-Object System.Drawing.Point(18, 148)
$form.Controls.Add($lblTarget)

$status = New-Object System.Windows.Forms.Label
$status.Text = 'Checking for updates...'
$status.AutoSize = $false
$status.Size = New-Object System.Drawing.Size(576, 120)
$status.Location = New-Object System.Drawing.Point(22, 180)
$status.ForeColor = [System.Drawing.Color]::FromArgb(40, 40, 40)
$form.Controls.Add($status)

$btnMain = New-Object System.Windows.Forms.Button
$btnMain.Text = 'Install / Update'
$btnMain.Location = New-Object System.Drawing.Point(300, 330)
$btnMain.Size = New-Object System.Drawing.Size(140, 34)
$btnMain.Enabled = $false
$form.Controls.Add($btnMain)

$btnPlay = New-Object System.Windows.Forms.Button
$btnPlay.Text = 'Play (Solo)'
$btnPlay.Location = New-Object System.Drawing.Point(450, 330)
$btnPlay.Size = New-Object System.Drawing.Size(146, 34)
$btnPlay.Enabled = $false
$form.Controls.Add($btnPlay)

$btnUninstall = New-Object System.Windows.Forms.LinkLabel
$btnUninstall.Text = 'Remove Cosmonarchy Plus...'
$btnUninstall.Location = New-Object System.Drawing.Point(22, 404)
$btnUninstall.AutoSize = $true
$form.Controls.Add($btnUninstall)

$btnClose = New-Object System.Windows.Forms.Button
$btnClose.Text = 'Close'
$btnClose.Location = New-Object System.Drawing.Point(540, 400)
$btnClose.Size = New-Object System.Drawing.Size(60, 30)
$form.Controls.Add($btnClose)
$btnClose.Add_Click({ $form.Close() })

$script:CurrentTarget = $null
$script:CurrentManifest = $null
$script:Online = $false

function Set-Status([string]$text, [System.Drawing.Color]$color) {
    $status.Text = $text
    $status.ForeColor = $color
    $form.Refresh()
}

function Resolve-Target([string]$sourceDir) {
    return Join-Path (Split-Path -Parent $sourceDir) 'Cosmonarchy Plus'
}

# One install/update pass. Safe to re-run; it only does what is missing or stale.
function Invoke-InstallOrUpdate([string]$sourceDir, [string]$target, $manifest) {
    $state = Get-InstalledState $target
    $needBuild = $true
    if ($state) { $needBuild = ($state.build_ref -ne $manifest.build_ref) }

    New-Item -ItemType Directory -Path $target -Force | Out-Null

    # First install: copy the base game once (Starcraft folder, a few GB).
    if (-not (Test-Path -LiteralPath (Join-Path $target 'Starcraft'))) {
        Set-Status 'Copying the base game (Starcraft folder)... this takes a few minutes.' ([System.Drawing.Color]::FromArgb(150, 90, 0))
        $rc = Start-Process -FilePath 'robocopy.exe' -ArgumentList @(
            "`"$(Join-Path $sourceDir 'Starcraft')`"", "`"$(Join-Path $target 'Starcraft')`"",
            '/E', '/NFL', '/NDL', '/NJH', '/NJS', '/NP', '/R:1', '/W:1'
        ) -Wait -PassThru -NoNewWindow
        if ($rc.ExitCode -ge 8) { throw ("base copy failed (robocopy exit {0})" -f $rc.ExitCode) }
        Write-Log 'base game copied'
    }

    # Mod zip only when the pinned build changed (or on first install).
    if ($needBuild -or -not (Test-Path -LiteralPath (Join-Path $target 'Release\Cosmonarchy BW.exe'))) {
        $modName = $script:ModZip -f $manifest.version
        $modUrl = $manifest.assetUrls[$modName]
        if (-not $modUrl) { throw ('release has no asset ' + $modName) }
        $modZip = Join-Path $script:TempDir $modName
        Download-Asset $modName $modUrl $modZip { param($m) Set-Status $m ([System.Drawing.Color]::FromArgb(150, 90, 0)) }
        $hash = (Get-FileHash -LiteralPath $modZip -Algorithm SHA256).Hash.ToLower()
        if ($hash -ne $manifest.assets[$modName].sha256) { throw ('checksum mismatch for ' + $modName) }
        Set-Status 'Installing game build...' ([System.Drawing.Color]::FromArgb(150, 90, 0))
        Apply-ModZip $modZip $target
        Write-Log 'mod applied'
    }

    # Plugins zip always (it is the small, frequently changing part).
    $plugName = $script:PluginsZip -f $manifest.version
    $plugUrl = $manifest.assetUrls[$plugName]
    if (-not $plugUrl) { throw ('release has no asset ' + $plugName) }
    $plugZip = Join-Path $script:TempDir $plugName
    Download-Asset $plugName $plugUrl $plugZip { param($m) Set-Status $m ([System.Drawing.Color]::FromArgb(150, 90, 0)) }
    $hash = (Get-FileHash -LiteralPath $plugZip -Algorithm SHA256).Hash.ToLower()
    if ($hash -ne $manifest.assets[$plugName].sha256) { throw ('checksum mismatch for ' + $plugName) }
    $payload = Join-Path $script:TempDir 'plugins-extract'
    if (Test-Path -LiteralPath $payload) { Remove-Item -LiteralPath $payload -Recurse -Force }
    Set-Status 'Installing plugins...' ([System.Drawing.Color]::FromArgb(150, 90, 0))
    Expand-Zip $plugZip $payload
    Apply-PluginsPayload $payload $target

    Write-ConfTemplates (Join-Path $target 'Release')
    Write-LaunchCmd $target
    New-Shortcuts $target
    Set-InstalledState $target $manifest
    Write-Log ("installed version {0} (build {1})" -f $manifest.version, $manifest.build_ref)
}

$btnPlay.Add_Click({
    if ($script:CurrentTarget) { Start-Game $script:CurrentTarget 'solo' }
})

$btnUninstall.Add_Click({
    if (-not $script:CurrentTarget) { return }
    $yes = [System.Windows.Forms.MessageBox]::Show(
        'Remove the Cosmonarchy Plus folder and its shortcuts?' + [Environment]::NewLine +
        'Your normal Cosmonarchy is not affected.',
        'Remove Cosmonarchy Plus', 'YesNo', 'Warning')
    if ($yes -eq 'Yes') {
        foreach ($label in @('Cosmonarchy Plus', 'Cosmonarchy Plus (Host)', 'Cosmonarchy Plus (Guest)')) {
            $lnk = Join-Path ([Environment]::GetFolderPath('Desktop')) ($label + '.lnk')
            if (Test-Path -LiteralPath $lnk) { Remove-Item -LiteralPath $lnk -Force }
        }
        if (Test-Path -LiteralPath $script:CurrentTarget) {
            Remove-Item -LiteralPath $script:CurrentTarget -Recurse -Force
        }
        $script:CurrentTarget = $null
        $btnPlay.Enabled = $false
        $btnMain.Enabled = $true
        Set-Status 'Cosmonarchy Plus removed.' ([System.Drawing.Color]::FromArgb(20, 110, 40))
        Write-Log 'uninstalled'
    }
})

$btnMain.Add_Click({
    try {
        $source = $txtDir.Text.Trim()
        if (-not (Test-GameDir $source)) {
            [System.Windows.Forms.MessageBox]::Show(
                'That folder does not look like a Cosmonarchy install.' + [Environment]::NewLine +
                'Pick the folder that contains "Release" and "Starcraft".',
                'Not a Cosmonarchy folder', 'OK', 'Warning') | Out-Null
            return
        }
        $target = Resolve-Target $source
        $script:CurrentTarget = $target
        $btnMain.Enabled = $false
        Set-Status 'Working...' ([System.Drawing.Color]::FromArgb(150, 90, 0))
        Invoke-InstallOrUpdate $source $target $script:CurrentManifest
        $btnPlay.Enabled = $true
        Set-Status ('Done. Version {0} is installed.' -f $script:CurrentManifest.version) + [Environment]::NewLine +
            'Three shortcuts are on your desktop: Solo, Host, Guest.' + [Environment]::NewLine +
            'Run this installer again any time to update.' + [Environment]::NewLine +
            ('Log: ' + $script:LogPath) ([System.Drawing.Color]::FromArgb(20, 110, 40))
        $btnMain.Enabled = $true
    } catch {
        Write-Log ("ERROR: {0}" -f $_.Exception.Message)
        Set-Status ('Failed: ' + $_.Exception.Message) + [Environment]::NewLine +
            ('Log: ' + $script:LogPath) ([System.Drawing.Color]::FromArgb(170, 30, 30))
        $btnMain.Enabled = $true
    }
})

# ---------------- startup ----------------

Write-Log 'updater started'
$detected = @(Find-GameDirs)
if ($detected.Count -gt 0) { $txtDir.Text = $detected[0] }

try {
    $script:CurrentManifest = Get-LatestRelease
    $script:Online = $true
    $latest = $script:CurrentManifest.version
    $firstSource = if ($detected.Count -gt 0) { $detected[0] } else { $null }
    if ($firstSource) {
        $state = Get-InstalledState (Resolve-Target $firstSource)
        if ($state -and $state.version -eq $latest) {
            $script:CurrentTarget = Resolve-Target $firstSource
            $btnPlay.Enabled = $true
            $btnMain.Enabled = $true
            Set-Status ("You are up to date (version {0})." -f $latest) ([System.Drawing.Color]::FromArgb(20, 110, 40))
        } elseif ($state) {
            $btnMain.Text = ('Update to ' + $latest)
            $btnMain.Enabled = $true
            Set-Status ("Installed: {0}. Latest: {1}. Click to update." -f $state.version, $latest) ([System.Drawing.Color]::FromArgb(20, 110, 40))
        } else {
            $btnMain.Text = 'Install'
            $btnMain.Enabled = $true
            Set-Status ('Found your Cosmonarchy install. Click Install - nothing else to configure.' + [Environment]::NewLine +
                ('Latest version: ' + $latest)) ([System.Drawing.Color]::FromArgb(20, 110, 40))
        }
    } else {
        Set-Status ('No Cosmonarchy install was found on this computer.' + [Environment]::NewLine +
            'Install Cosmonarchy first (Fraud Launcher 2 from fraudsclub.com), then run this again.' + [Environment]::NewLine +
            'Or click Browse to point at the folder manually.') ([System.Drawing.Color]::FromArgb(170, 30, 30))
    }
} catch {
    Write-Log ("WARNING: cannot reach the update server: {0}" -f $_.Exception.Message)
    if ($detected.Count -gt 0) {
        $target = Resolve-Target $detected[0]
        if (Get-InstalledState $target) {
            $script:CurrentTarget = $target
            $btnPlay.Enabled = $true
            Set-Status 'Offline - the update server is unreachable, but your installed copy still works.' ([System.Drawing.Color]::FromArgb(150, 90, 0))
        } else {
            Set-Status 'Offline - the update server is unreachable, and Plus is not installed yet. Check your connection.' ([System.Drawing.Color]::FromArgb(170, 30, 30))
        }
    } else {
        Set-Status 'Offline - the update server is unreachable. Check your connection and run again.' ([System.Drawing.Color]::FromArgb(170, 30, 30))
    }
}

[void]$form.ShowDialog()
