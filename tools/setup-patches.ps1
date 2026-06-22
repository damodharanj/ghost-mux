#!/usr/bin/env pwsh
Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$ProjectRoot = Split-Path -Parent $ScriptDir

$GpuiDir = Join-Path $ProjectRoot "patches" "gpui-component"
$PatchFile = Join-Path $ProjectRoot "patches" "gpui-component.patch"
$RepoUrl = "https://github.com/longbridge/gpui-component.git"
$CommitSha = "196b9259b562c26be97c92f88c798bbeefa9cb3d"

if (-not (Test-Path $GpuiDir)) {
    Write-Host "==> Cloning gpui-component from upstream..."
    git clone $RepoUrl $GpuiDir
    $origDir = Get-Location
    try {
        Set-Location $GpuiDir
        Write-Host "==> Checking out specific commit: $CommitSha..."
        git checkout $CommitSha
        Write-Host "==> Applying patch gpui-component.patch..."
        git apply $PatchFile
        Write-Host "==> gpui-component setup successfully!"
    } finally {
        Set-Location $origDir
    }
} else {
    Write-Host "==> gpui-component is already present."
}
