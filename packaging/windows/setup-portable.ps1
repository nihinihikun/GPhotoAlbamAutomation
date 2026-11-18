$ErrorActionPreference = 'Stop'
Set-Location -LiteralPath $PSScriptRoot

function Find-Browser {
    foreach ($binary in @('Google\Chrome\Application\chrome.exe', 'Microsoft\Edge\Application\msedge.exe')) {
        foreach ($root in @($env:ProgramW6432, $env:ProgramFiles, ${env:ProgramFiles(x86)}, $env:LOCALAPPDATA)) {
            if (!$root) { continue }
            $path = Join-Path $root $binary
            if (Test-Path -LiteralPath $path -PathType Leaf) { return $path }
        }
    }
}

function Install-Browser {
    $command = Get-Command winget.exe -CommandType Application -ErrorAction SilentlyContinue
    $winget = if ($command) { $command.Source }
    if (!$winget -and $env:LOCALAPPDATA) {
        $candidate = Join-Path $env:LOCALAPPDATA 'Microsoft\WindowsApps\winget.exe'
        if (Test-Path -LiteralPath $candidate -PathType Leaf) { $winget = $candidate }
    }
    if (!$winget) {
        throw 'Chrome or Edge is required. Install a browser, or install App Installer to enable automatic Chrome installation, then reopen SETUP.bat.'
    }
    Write-Host 'Starting Chrome installation. Follow the installer prompts.'
    & $winget install --id Google.Chrome --exact --source winget --no-upgrade
    if ($LASTEXITCODE -ne 0) {
        throw "Chrome installation returned $LASTEXITCODE. Check the installer output and reopen SETUP.bat."
    }
    if (!(Find-Browser)) {
        throw 'Chrome could not be located after installation. Set browser_path in config.json to its executable.'
    }
}

try {
    $executable = Join-Path $PSScriptRoot 'gpae.exe'
    if (!(Test-Path -LiteralPath $executable -PathType Leaf)) {
        throw 'gpae.exe was not found. Extract the complete ZIP before running SETUP.bat.'
    }
    $configPath = Join-Path $PSScriptRoot 'config.json'
    if (!(Test-Path -LiteralPath $configPath -PathType Leaf)) {
        Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'config.example.json') -Destination $configPath -ErrorAction Stop
    }
    Write-Host 'Set account and output_dir in config.json. Save and close Notepad to continue.'
    # The user edits the configuration in this foreground interactive editor.
    Start-Process -FilePath 'notepad.exe' -ArgumentList ('"' + $configPath + '"') -Wait

    $settings = Get-Content -LiteralPath $configPath -Raw -Encoding UTF8 | ConvertFrom-Json
    $browserPath = if ($settings.browser_path) { $settings.browser_path } else { $settings.chrome_path }
    if ($browserPath) {
        if (!(Test-Path -LiteralPath $browserPath -PathType Leaf)) {
            throw 'The configured browser_path does not exist. Correct it or set it to null for automatic detection.'
        }
    } elseif (!(Find-Browser)) {
        Install-Browser
    }

    & $executable setup --config $configPath
    exit $LASTEXITCODE
} catch {
    [Console]::Error.WriteLine("SETUP failed. $($_.Exception.Message)")
    exit 1
}
