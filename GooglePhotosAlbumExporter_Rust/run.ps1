param([string]$Config = 'config.json')

$ErrorActionPreference = 'Stop'
Set-Location -LiteralPath $PSScriptRoot

try {
    $artifacts = @(
        & cargo.exe build --release --locked --message-format=json-render-diagnostics |
            ForEach-Object {
                $message = ConvertFrom-Json -InputObject $_
                if ($message.reason -eq 'compiler-artifact' -and
                    $message.target.name -eq 'gpae' -and
                    $message.target.kind -contains 'bin' -and $message.executable) {
                    $message.executable
                }
            }
    )
    if ($LASTEXITCODE -ne 0) { exit 1 }
    if ($artifacts.Count -ne 1 -or !(Test-Path -LiteralPath $artifacts[0] -PathType Leaf)) {
        throw 'Cargo did not report the GPAE executable.'
    }

    & $artifacts[0] run --config $Config
    exit $LASTEXITCODE
} catch {
    [Console]::Error.WriteLine("RUN failed. $($_.Exception.Message)")
    exit 1
}
