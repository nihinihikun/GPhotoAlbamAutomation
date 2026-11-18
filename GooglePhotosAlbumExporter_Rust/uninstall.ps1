$ErrorActionPreference = 'Stop'
$reparsePoint = [IO.FileAttributes]::ReparsePoint
$comparison = [StringComparison]::OrdinalIgnoreCase
$backupSuffix = '.gpae-uninstall-' + (Get-Date -Format 'yyyyMMdd-HHmmss') + '-' + [Guid]::NewGuid().ToString('N').Substring(0, 8)

function Get-CanonicalPath([string]$Path, [string]$BasePath) {
    if ([string]::IsNullOrWhiteSpace($Path)) { throw 'A path is missing.' }
    $path = $Path.Replace('/', '\')
    if (![IO.Path]::IsPathRooted($path)) { $path = [IO.Path]::Combine($BasePath, $path) }
    if ($path -notmatch '^[A-Za-z]:\\[^:]*$') { throw "Expected a local drive path: $path" }
    foreach ($part in $path.Substring(3).Split('\')) {
        if ($part -notin @('.', '..') -and $part -match '[. ]$') {
            throw "Ambiguous Windows path: $path"
        }
    }

    $fullPath = [IO.Path]::GetFullPath($path)
    $driveRoot = [IO.Path]::GetPathRoot($fullPath)
    $resolved = $driveRoot
    foreach ($part in $fullPath.Substring($driveRoot.Length).TrimEnd('\').Split('\')) {
        if (!$part) { continue }
        $next = [IO.Path]::Combine($resolved, $part)
        try { $item = Get-Item -LiteralPath $next -Force }
        catch [System.Management.Automation.ItemNotFoundException] { $item = $null }
        if ($item) {
            if ($item.Attributes -band $reparsePoint) { throw "Linked path: $next" }
            $next = [IO.Path]::Combine($resolved, $item.Name)
        }
        $resolved = $next
    }
    return $resolved
}

function Test-Within([string]$Path, [string]$Root) {
    return $Path.Equals($Root, $comparison) -or $Path.StartsWith($Root.TrimEnd('\') + '\', $comparison)
}

function Test-Overlap([string]$First, [string]$Second) {
    return (Test-Within $First $Second) -or (Test-Within $Second $First)
}

function Assert-SafeContents([string]$Path) {
    $pending = [Collections.Generic.Stack[string]]::new()
    $pending.Push($Path)
    while ($pending.Count -gt 0) {
        foreach ($entry in Get-ChildItem -LiteralPath $pending.Pop() -Force) {
            if ($entry.Attributes -band $reparsePoint) { throw "Linked item: $($entry.FullName)" }
            if ($entry.Name.StartsWith('albums_', $comparison)) { throw "Possible album export: $($entry.FullName)" }
            if ($entry.PSIsContainer) { $pending.Push($entry.FullName) }
        }
    }
}

function Assert-ProfileOwned([string]$Path) {
    if (Test-Within $appRoot $Path) { throw 'The profile overlaps the application folder.' }
    if (Test-Within $env:USERPROFILE $Path) { throw 'The profile overlaps the Windows user folder.' }
    foreach ($protected in $protectedProfilePaths) {
        if ($protected -and (Test-Overlap $Path $protected)) { throw "The profile overlaps a protected folder: $protected" }
    }

    $markerPath = [IO.Path]::Combine($Path, '.gpae-profile')
    $markerText = 'GPAE Chrome profile v1'
    if (Test-Path -LiteralPath $markerPath -PathType Leaf) {
        $marker = Get-Item -LiteralPath $markerPath -Force
        if (!($marker.Attributes -band $reparsePoint) -and $marker.Length -eq $markerText.Length -and
            [IO.File]::ReadAllText($markerPath) -ceq $markerText) { return }
    } elseif ($Path.Equals($defaultProfileDir, $comparison)) {
        $localState = [IO.Path]::Combine($Path, 'Local State')
        if ((Test-Path -LiteralPath $localState -PathType Leaf) -or
            !(Get-ChildItem -LiteralPath $Path -Force | Select-Object -First 1)) { return }
    }
    throw 'This profile has no valid GPAE ownership marker.'
}

function Assert-Candidate([string]$Kind, [string]$Path) {
    $resolved = Get-CanonicalPath $Path $appRoot
    if (!$resolved.Equals($Path, $comparison)) { throw "Path changed: $Path" }
    if ([IO.DriveInfo]::new([IO.Path]::GetPathRoot($resolved)).DriveType -ne [IO.DriveType]::Fixed) {
        throw 'The path must be on a local fixed drive.'
    }
    if (Test-Overlap $resolved $outputDir) { throw 'This item overlaps the configured export folder.' }

    if ($Kind -eq 'Settings') {
        if (!(Test-Path -LiteralPath $resolved -PathType Leaf)) { throw 'The settings file is missing.' }
        return
    }

    if (!(Test-Path -LiteralPath $resolved -PathType Container)) { throw 'The folder is missing.' }
    if ($Kind -eq 'Profile') {
        Assert-ProfileOwned $resolved
    } else {
        if (Test-Overlap $resolved $profileDir) { throw 'The build cache overlaps the configured profile.' }
        $rustInfo = [IO.Path]::Combine($resolved, '.rustc_info.json')
        $releaseApp = [IO.Path]::Combine($resolved, 'release', 'gpae.exe')
        $debugApp = [IO.Path]::Combine($resolved, 'debug', 'gpae.exe')
        if (!(Test-Path -LiteralPath $rustInfo -PathType Leaf) -or
            (!(Test-Path -LiteralPath $releaseApp -PathType Leaf) -and !(Test-Path -LiteralPath $debugApp -PathType Leaf))) {
            throw 'The folder does not contain a recognizable GPAE build.'
        }
    }
    Assert-SafeContents $resolved
}

function Confirm-Quarantine([string]$Kind, [string]$Path) {
    if (!(Test-Path -LiteralPath $Path)) { return $true }
    try {
        Assert-Candidate $Kind $Path
    } catch {
        Write-Warning "Skipped $Kind ($Path). $($_.Exception.Message)"
        return $true
    }

    $backup = $Path + $backupSuffix
    Write-Host "`n$Kind"
    Write-Host "  From: $Path"
    Write-Host "  To:   $backup"
    if ((Read-Host 'Move this item aside? [y/N]') -ine 'y') { return $true }
    try {
        if ([IO.File]::ReadAllText($configPath) -cne $configText) { throw 'config.json changed during uninstall.' }
        Assert-Candidate $Kind $Path
        if (!(Get-CanonicalPath $backup $appRoot).Equals($backup, $comparison) -or (Test-Path -LiteralPath $backup)) {
            throw "Backup destination is already in use: $backup"
        }
        if ($Kind -eq 'Settings') {
            [IO.File]::Move($Path, $backup)
        } else {
            [IO.Directory]::Move($Path, $backup)
        }
        Write-Host 'Moved.'
        return $true
    } catch {
        Write-Warning "Could not move $Kind ($Path). $($_.Exception.Message)"
        return $false
    }
}

try {
    $appRoot = Get-CanonicalPath $PSScriptRoot $PSScriptRoot
    $configPath = Get-CanonicalPath 'config.json' $appRoot
    if (!(Test-Path -LiteralPath $configPath -PathType Leaf)) {
        Write-Host 'No active config.json found. Check this folder for any exports or backups before removing it:'
        Write-Host $appRoot
        exit 0
    }
    $configText = [IO.File]::ReadAllText($configPath)
    $settings = $configText | ConvertFrom-Json
    $outputSetting = if ($null -ne $settings.output_dir) { $settings.output_dir } else { $settings.output_directory }
    $profileSetting = if ($null -ne $settings.profile_dir) { $settings.profile_dir } else { 'chrome-profile' }
    if ($outputSetting -isnot [string] -or $profileSetting -isnot [string]) {
        throw 'config.json must contain a valid output directory and optional profile directory.'
    }

    $outputDir = Get-CanonicalPath $outputSetting $appRoot
    $profileDir = Get-CanonicalPath $profileSetting $appRoot
    $defaultProfileDir = [IO.Path]::Combine($appRoot, 'chrome-profile')
    $buildCache = [IO.Path]::Combine($appRoot, 'target')
    $protectedProfilePaths = @($env:SystemRoot, $env:ProgramFiles, ${env:ProgramFiles(x86)}, $env:ProgramW6432, $env:ProgramData, $buildCache)
    foreach ($name in @('browser', 'docs', 'examples', 'src', 'tests', '.git', '.github', '.agents', '.codex')) {
        $protectedProfilePaths += [IO.Path]::Combine($appRoot, $name)
    }
    foreach ($name in @('Google', 'Microsoft', 'Chromium', 'BraveSoftware', 'Vivaldi')) {
        $protectedProfilePaths += [IO.Path]::Combine($env:LOCALAPPDATA, $name)
    }

    Write-Host 'Close GPAE and every window in its dedicated Chrome profile before continuing.'
    Write-Host "Export folder to keep: $outputDir"
    Write-Host 'Each confirmed item is renamed as a backup in the same folder.'

    $profileMoved = Confirm-Quarantine 'Profile' $profileDir
    $cacheMoved = Confirm-Quarantine 'Build cache' $buildCache
    if (!$profileMoved -or !$cacheMoved) {
        Write-Warning 'config.json was kept so you can retry.'
        exit 1
    }
    if (!(Confirm-Quarantine 'Settings' $configPath)) { exit 1 }
    Write-Host "`nApplication folder: $appRoot"
    Write-Host 'To remove the source folder in Explorer, first save any exports or backups you want to keep.'
} catch {
    [Console]::Error.WriteLine("Uninstall stopped. $($_.Exception.Message)")
    exit 1
}
