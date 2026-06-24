#!/usr/bin/env pwsh
Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$ProjectRoot = Split-Path -Parent (Split-Path -Parent $ScriptDir)

if ($env:CARGO -and (Test-Path $env:CARGO)) {
    $CargoBin = $env:CARGO
} else {
    $cargoCmd = Get-Command cargo -ErrorAction SilentlyContinue
    if (-not $cargoCmd) {
        throw "cargo not found. Set CARGO or install Rust toolchain."
    }
    $CargoBin = $cargoCmd.Source
}

Set-Location $ProjectRoot
& (Join-Path $ProjectRoot "tools" "setup-patches.ps1")

Write-Host "==> Starting ghost-mux-server in background..."
# Start server process in background
$ServerJob = Start-Process -FilePath $CargoBin -ArgumentList "run", "--bin", "ghost-mux-server", "--", "--port", "3030" -PassThru -NoNewWindow

Start-Sleep -Seconds 2

try {
    Write-Host "==> Starting ghost-mux GUI..."
    & $CargoBin run @args
} finally {
    Write-Host "==> Stopping ghost-mux-server..."
    Stop-Process -Id $ServerJob.Id -Force -ErrorAction SilentlyContinue
}
