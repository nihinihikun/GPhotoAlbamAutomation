param([string]$OutputDirectory)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) {
    throw 'Run this packaging script on Windows with Rust and the MSVC build tools installed.'
}

$repository = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$source = Join-Path $repository 'GooglePhotosAlbumExporter_Rust'
$templates = Join-Path $repository 'packaging/windows'
$buildRoot = Join-Path $repository 'target/windows-build'
$workRoot = Join-Path $repository 'target/package-work'
if (!$OutputDirectory) { $OutputDirectory = Join-Path $repository 'target/releases' }
$output = [IO.Path]::GetFullPath($OutputDirectory)

function Invoke-Cargo([string[]]$Arguments) {
    & cargo.exe @Arguments
    if ($LASTEXITCODE -ne 0) { throw "Cargo exited with code $LASTEXITCODE." }
}

function Write-Utf8([string]$Path, [string]$Text) {
    [IO.File]::WriteAllText($Path, $Text, [Text.UTF8Encoding]::new($false))
}

function Copy-PackageFile([string]$Path, [string]$RelativePath) {
    $destination = Join-Path $package $RelativePath
    $null = New-Item -ItemType Directory -Path ([IO.Path]::GetDirectoryName($destination)) -Force
    Copy-Item -LiteralPath $Path -Destination $destination
}

function Get-Sha256([string]$Path) {
    (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

$previousTarget = [Environment]::GetEnvironmentVariable('CARGO_TARGET_DIR', 'Process')
$previousFlags = [Environment]::GetEnvironmentVariable('CARGO_ENCODED_RUSTFLAGS', 'Process')
$scratch = $null
$archivePath = $null
$archiveCreated = $false
$complete = $false
try {
    $null = Get-Command cargo.exe -CommandType Application -ErrorAction Stop
    $null = Get-Command rustc.exe -CommandType Application -ErrorAction Stop
    $manifest = Join-Path $source 'Cargo.toml'
    $metadataText = Invoke-Cargo -Arguments @('metadata', '--manifest-path', $manifest, '--locked', '--format-version', '1')
    $metadata = ($metadataText -join [Environment]::NewLine) | ConvertFrom-Json
    $app = @($metadata.packages | Where-Object { $_.name -eq 'gpae' -and !$_.source })
    if ($app.Count -ne 1 -or $app[0].version -notmatch '^\d+\.\d+\.\d+(?:[-+][A-Za-z0-9.-]+)?$') {
        throw 'Could not identify the exporter package version.'
    }
    $version = $app[0].version
    $name = "GooglePhotosAlbumExporter-v$version-windows-x64"
    $archivePath = Join-Path $output ($name + '.zip')
    if ((Test-Path -LiteralPath $archivePath) -or (Test-Path -LiteralPath ($archivePath + '.sha256'))) {
        throw "A release with this name already exists in $output. Choose another output directory."
    }
    $sysroot = (& rustc.exe --print sysroot).Trim()
    if ($LASTEXITCODE -ne 0) { throw 'Rust could not report its installation directory.' }
    $rustDocs = Join-Path $sysroot 'share/doc/rust'
    $bundledRustNotices = Join-Path $templates 'licenses/rust'
    foreach ($notice in @('COPYRIGHT', 'LICENSE-APACHE', 'LICENSE-MIT')) {
        if (!(Test-Path -LiteralPath (Join-Path $rustDocs $notice) -PathType Leaf) -and
            !(Test-Path -LiteralPath (Join-Path $bundledRustNotices $notice) -PathType Leaf)) {
            throw "A bundled Rust license notice is missing: $notice"
        }
    }

    # Target-specific compilation keeps the static runtime flag off host build tools.
    $env:CARGO_TARGET_DIR = $buildRoot
    $env:CARGO_ENCODED_RUSTFLAGS = '-C' + [char]31 + 'target-feature=+crt-static'
    Invoke-Cargo -Arguments @('build', '--manifest-path', $manifest, '--release', '--locked', '--target', 'x86_64-pc-windows-msvc')
    $executable = Join-Path $buildRoot 'x86_64-pc-windows-msvc/release/gpae.exe'
    & $executable --help
    if ($LASTEXITCODE -ne 0) { throw 'The built executable did not start.' }

    $null = New-Item -ItemType Directory -Path $workRoot -Force
    $scratch = Join-Path $workRoot ([Guid]::NewGuid().ToString('N'))
    $package = Join-Path $scratch $name
    $null = New-Item -ItemType Directory -Path $package
    foreach ($file in @('SETUP.bat', 'RUN.bat', 'setup-portable.ps1', 'README.md')) {
        Copy-PackageFile (Join-Path $templates $file) $file
    }
    $readme = Join-Path $package 'README.md'
    Write-Utf8 $readme ([IO.File]::ReadAllText($readme).Replace('v0.2.0', "v$version"))
    Copy-PackageFile $executable 'gpae.exe'
    foreach ($file in @('LICENSE', 'UNINSTALL.bat', 'docs/TOOLKIT_LICENSE.txt', 'docs/03_SCHEMA_AND_COMPATIBILITY.md')) {
        Copy-PackageFile (Join-Path $source $file) $file
    }
    $uninstall = [IO.File]::ReadAllText((Join-Path $source 'uninstall.ps1'))
    Write-Utf8 (Join-Path $package 'uninstall.ps1') ($uninstall.Replace('To remove the source folder in Explorer', 'To remove the application folder in Explorer'))
    $settings = [ordered]@{
        account = 'YOUR_OLD_ACCOUNT@gmail.com'
        output_dir = 'exports'
        browser_path = $null
        timeout_seconds = 0
    }
    Write-Utf8 (Join-Path $package 'config.example.json') (($settings | ConvertTo-Json) + [char]10)

    $notices = [Collections.Generic.List[string]]::new()
    $notices.Add("Google Photos Album Exporter v$version")
    $notices.Add('Rust dependency license notices')
    $notices.Add('')
    foreach ($dependency in @($metadata.packages | Where-Object { $_.source -like 'registry+*' } | Sort-Object name, version)) {
        $dependencyRoot = [IO.Path]::GetDirectoryName($dependency.manifest_path)
        $notices.Add(('{0} {1} - {2}' -f $dependency.name, $dependency.version, $dependency.license))
        $notices.Add(('https://crates.io/crates/{0}/{1}' -f $dependency.name, $dependency.version))
        $licenses = @(Get-ChildItem -LiteralPath $dependencyRoot -File | Where-Object {
            $_.Name -match '^(?i:licen[cs]e|copying|notice)(?:$|[._-])'
        })
        foreach ($file in $licenses) {
            $relative = 'licenses/' + $dependency.name + '-' + $dependency.version + '/' + $file.Name
            Copy-PackageFile $file.FullName $relative
            $notices.Add('  ' + $relative)
        }
        $notices.Add('')
    }
    $rustVersion = (& rustc.exe --version) -join ' '
    if ($LASTEXITCODE -ne 0) { throw 'Rust could not report its version.' }
    $notices.Add($rustVersion)
    $fallbackUsed = $false
    foreach ($notice in @('COPYRIGHT', 'LICENSE-APACHE', 'LICENSE-MIT')) {
        $relative = 'licenses/rust-standard-library/' + $notice
        $noticeSource = Join-Path $rustDocs $notice
        if (!(Test-Path -LiteralPath $noticeSource -PathType Leaf)) {
            $noticeSource = Join-Path $bundledRustNotices $notice
            $fallbackUsed = $true
        }
        Copy-PackageFile $noticeSource $relative
        $notices.Add('  ' + $relative)
    }
    if ($fallbackUsed) {
        $notices.Add('Bundled fallback license notices were prepared from the Rust 1.72.0 distribution.')
    }
    Write-Utf8 (Join-Path $package 'THIRD_PARTY_NOTICES.txt') (($notices -join [char]10) + [char]10)

    $relativePaths = [string[]]@(Get-ChildItem -LiteralPath $package -Recurse -File -Force | ForEach-Object {
        $_.FullName.Substring($package.Length + 1).Replace('\', '/')
    })
    [Array]::Sort($relativePaths, [StringComparer]::Ordinal)
    $hashes = foreach ($relative in $relativePaths) {
        '{0}  {1}' -f (Get-Sha256 (Join-Path $package $relative)), $relative
    }
    Write-Utf8 (Join-Path $package 'SHA256SUMS.txt') (($hashes -join [char]10) + [char]10)
    $entries = @($relativePaths) + @('SHA256SUMS.txt')

    Add-Type -AssemblyName System.IO.Compression
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $null = New-Item -ItemType Directory -Path $output -Force
    $zipStream = [IO.File]::Open($archivePath, [IO.FileMode]::CreateNew)
    $archiveCreated = $true
    try {
        $zip = [IO.Compression.ZipArchive]::new($zipStream, [IO.Compression.ZipArchiveMode]::Create, $true)
        try {
            foreach ($relative in $entries) {
                $null = [IO.Compression.ZipFileExtensions]::CreateEntryFromFile(
                    $zip, (Join-Path $package $relative), ($name + '/' + $relative),
                    [IO.Compression.CompressionLevel]::Optimal
                )
            }
        } finally { $zip.Dispose() }
    } finally { $zipStream.Dispose() }
    $archive = [IO.Compression.ZipFile]::OpenRead($archivePath)
    try {
        if ($archive.Entries.Count -ne $entries.Count) { throw 'The ZIP entry count differs from the package.' }
        foreach ($relative in $entries) {
            $entry = $archive.GetEntry($name + '/' + $relative)
            if (!$entry) { throw "A ZIP entry is missing: $relative" }
            $stream = $entry.Open()
            $sha = [Security.Cryptography.SHA256]::Create()
            try { $actual = [BitConverter]::ToString($sha.ComputeHash($stream)).Replace('-', '').ToLowerInvariant() }
            finally { $stream.Dispose(); $sha.Dispose() }
            if ($actual -cne (Get-Sha256 (Join-Path $package $relative))) {
                throw "ZIP content differs from the package: $relative"
            }
        }
    } finally { $archive.Dispose() }
    Write-Utf8 ($archivePath + '.sha256') ((Get-Sha256 $archivePath) + '  ' + [IO.Path]::GetFileName($archivePath) + [char]10)
    $complete = $true
    Write-Host "Created and verified $archivePath"
} finally {
    [Environment]::SetEnvironmentVariable('CARGO_TARGET_DIR', $previousTarget, 'Process')
    [Environment]::SetEnvironmentVariable('CARGO_ENCODED_RUSTFLAGS', $previousFlags, 'Process')
    if (!$complete -and $archiveCreated -and $archivePath) {
        Remove-Item -LiteralPath $archivePath -Force -ErrorAction SilentlyContinue
    }
    if ($scratch -and (Test-Path -LiteralPath $scratch)) {
        $temporaryDirectory = Get-Item -LiteralPath $scratch
        $resolved = $temporaryDirectory.FullName
        if ([IO.Path]::GetDirectoryName($resolved) -ne $workRoot -or
            ($temporaryDirectory.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw 'The temporary package path is outside the expected work folder.'
        }
        Remove-Item -LiteralPath $resolved -Recurse -Force
    }
}
