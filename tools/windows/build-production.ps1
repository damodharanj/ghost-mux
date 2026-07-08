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

$cargoTomlPath = Join-Path $ProjectRoot "Cargo.toml"
$cargoToml = Get-Content -Raw -Path $cargoTomlPath
$nameMatch = [regex]::Match($cargoToml, '(?m)^\s*name\s*=\s*"([^"]+)"')
if (-not $nameMatch.Success) {
    throw "unable to read binary name from Cargo.toml"
}
$BinName = $nameMatch.Groups[1].Value

$requiresZig = [regex]::IsMatch($cargoToml, '(?m)^\s*libghostty-vt-sys\s*=.*vendored')
if ($requiresZig) {
    $ensureZigScript = Join-Path $ScriptDir "ensure-zig.ps1"
    if (-not (Test-Path $ensureZigScript)) {
        throw "missing helper script at $ensureZigScript"
    }
    $zigBin = & $ensureZigScript
    if (-not (Test-Path $zigBin)) {
        throw "failed to install/use repo-local Zig at $zigBin"
    }
    $env:ZIG = $zigBin
    $env:PATH = "$(Split-Path -Parent $zigBin)$([System.IO.Path]::PathSeparator)$env:PATH"
}

$DistDir = Join-Path $ProjectRoot "dist"
$AppDir = Join-Path $DistDir $BinName
$LibDir = Join-Path $AppDir "lib"
$BinPath = Join-Path $AppDir "$BinName.exe"

if (Test-Path $AppDir) {
    Remove-Item -Recurse -Force $AppDir
}
New-Item -ItemType Directory -Path $LibDir | Out-Null

& (Join-Path $ProjectRoot "tools" "setup-patches.ps1")

Write-Host "==> Building release binary"
& $CargoBin build --release
& $CargoBin build --release --manifest-path (Join-Path $ProjectRoot "ghost-mux-server" "Cargo.toml") --target-dir (Join-Path $ProjectRoot "target")

$SourceBin = Join-Path $ProjectRoot "target/release/$BinName.exe"
if (-not (Test-Path $SourceBin)) {
    throw "release binary not found at $SourceBin"
}
Copy-Item -Path $SourceBin -Destination $BinPath -Force

$SourceServerBin = Join-Path $ProjectRoot "target/release/ghost-mux-server.exe"
if (-not (Test-Path $SourceServerBin)) {
    throw "release server binary not found at $SourceServerBin"
}
Copy-Item -Path $SourceServerBin -Destination (Join-Path $AppDir "ghost-mux-server.exe") -Force

$settingsPath = Join-Path $ProjectRoot "settings.yaml"
if (Test-Path $settingsPath) {
    Copy-Item -Path $settingsPath -Destination (Join-Path $AppDir "settings.yaml") -Force
}

$assetsPath = Join-Path $ProjectRoot "assets"
if (Test-Path $assetsPath) {
    Copy-Item -Path $assetsPath -Destination (Join-Path $AppDir "assets") -Recurse -Force
    $designPath = Join-Path $AppDir "assets/design"
    if (Test-Path $designPath) {
        Remove-Item -Recurse -Force $designPath
    }
}

$webPath = Join-Path $ProjectRoot "web"
if (Test-Path $webPath) {
    Copy-Item -Path $webPath -Destination (Join-Path $AppDir "web") -Recurse -Force
}

Write-Host "==> Done"
Write-Host "Bundle: $AppDir"
Write-Host "Binary: $BinPath"
