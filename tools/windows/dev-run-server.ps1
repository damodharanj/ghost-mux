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
& $CargoBin run --bin ghost-mux-server @args
exit $LASTEXITCODE
