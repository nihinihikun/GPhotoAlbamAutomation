$ErrorActionPreference = 'Stop'
Set-Location -LiteralPath $PSScriptRoot

function Find-Cargo {
    $command = Get-Command cargo.exe -CommandType Application -ErrorAction SilentlyContinue
    if ($command) { return $command.Source }

    $cargoHomes = @($env:CARGO_HOME, (Join-Path $env:USERPROFILE '.cargo'))
    foreach ($cargoHome in $cargoHomes) {
        if (!$cargoHome) { continue }
        $bin = Join-Path $cargoHome 'bin'
        $cargo = Join-Path $bin 'cargo.exe'
        if (Test-Path -LiteralPath $cargo -PathType Leaf) {
            $env:Path = "$bin;$env:Path"
            return $cargo
        }
    }
}

function Find-Browser {
    foreach ($binary in @('Google\Chrome\Application\chrome.exe', 'Microsoft\Edge\Application\msedge.exe')) {
        foreach ($root in @($env:ProgramW6432, $env:ProgramFiles, ${env:ProgramFiles(x86)}, $env:LOCALAPPDATA)) {
            if (!$root) { continue }
            $path = Join-Path $root $binary
            if (Test-Path -LiteralPath $path -PathType Leaf) { return $path }
        }
    }
}

function Start-PackageInstall([string]$PackageId, [string[]]$Options) {
    $winget = (Get-Command winget.exe -CommandType Application -ErrorAction SilentlyContinue).Source
    if (!$winget -and $env:LOCALAPPDATA) {
        $aliasPath = Join-Path $env:LOCALAPPDATA 'Microsoft\WindowsApps\winget.exe'
        if (Test-Path -LiteralPath $aliasPath -PathType Leaf) { $winget = $aliasPath }
    }
    if (!$winget) {
        Write-Host 'Install or update App Installer, then run SETUP.bat again.'
        Start-Process 'https://apps.microsoft.com/detail/9nblggh4nns1'
        throw "WinGet is required to install $PackageId."
    }

    Write-Host "Starting installation of $PackageId. Follow the installer prompts."
    & $winget install --id $PackageId --exact --source winget --no-upgrade @Options
    if ($LASTEXITCODE -ne 0) {
        throw "WinGet returned $LASTEXITCODE for $PackageId. Check its output and run SETUP.bat again."
    }
}

function Test-RustLinker([string]$Rustc) {
    $scratch = Join-Path ([IO.Path]::GetTempPath()) ('gpae-link-' + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $scratch | Out-Null
    try {
        $source = Join-Path $scratch 'gpae_setup_probe.rs'
        [IO.File]::WriteAllText($source, 'fn main() {}')
        $ErrorActionPreference = 'Continue'
        $output = & $Rustc $source --out-dir $scratch 2>&1
        return @{ ExitCode = $LASTEXITCODE; Output = ($output -join [Environment]::NewLine) }
    } finally {
        Get-ChildItem -LiteralPath $scratch -File | Remove-Item -Force
        Remove-Item -LiteralPath $scratch
    }
}

try {
    $settings = Get-Content -LiteralPath 'config.json' -Raw -Encoding UTF8 | ConvertFrom-Json
    $browserPath = if ($settings.browser_path) { $settings.browser_path } else { $settings.chrome_path }
    if ($browserPath) {
        if (!(Test-Path -LiteralPath $browserPath -PathType Leaf)) {
            throw 'The browser_path in config.json does not exist. Correct it or set it to null for automatic detection.'
        }
    } elseif (!(Find-Browser)) {
        Start-PackageInstall 'Google.Chrome'
        if (!(Find-Browser)) { throw 'Chrome or Edge could not be located. Set browser_path in config.json.' }
    }

    $cargo = Find-Cargo
    if (!$cargo) {
        Start-PackageInstall 'Rustlang.Rustup' @('--interactive')
        $cargo = Find-Cargo
        if (!$cargo) { throw 'Cargo could not be located after installation. Reopen SETUP.bat.' }
    }

    $rustc = Join-Path (Split-Path -Parent $cargo) 'rustc.exe'
    if (!(Test-Path -LiteralPath $rustc -PathType Leaf)) {
        $rustc = (Get-Command rustc.exe -CommandType Application -ErrorAction Stop).Source
    }
    $compilerInfo = (& $rustc -vV) -join "`n"
    if ($LASTEXITCODE -ne 0) { throw 'Rust could not start. Complete rustup setup and run SETUP.bat again.' }

    $probe = Test-RustLinker $rustc
    if ($probe.ExitCode -ne 0 -and $compilerInfo -match '(?m)^host: .*windows-msvc$' -and
        $probe.Output -match 'linker .+ not found|LNK(?:1104|1181).*?(?:kernel32|advapi32|msvcrt|ucrt|vcruntime|legacy_stdio_definitions)\.lib') {
        Start-PackageInstall 'Microsoft.VisualStudio.2022.BuildTools' @(
            '--override', '--passive --wait --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended'
        )
        $probe = Test-RustLinker $rustc
    }
    if ($probe.ExitCode -ne 0) {
        Write-Host $probe.Output
        throw 'Rust could not link a test program. For MSVC, verify the C++ Build Tools workload and Windows SDK in Visual Studio Installer.'
    }

    Write-Host 'Building release executable and opening browser setup...'
    & $cargo run --release --locked -- setup --config config.json
    exit $LASTEXITCODE
} catch {
    [Console]::Error.WriteLine("SETUP failed. $($_.Exception.Message)")
    exit 1
}
