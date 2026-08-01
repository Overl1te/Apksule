# Build Apksule release binary + Inno Setup installer.
# Usage: .\scripts\build-installer.ps1 [-SkipBuild] [-Version 0.1.1]

[CmdletBinding()]
param(
    [switch]$SkipBuild,
    [string]$Version = "0.1.0"
)

$ErrorActionPreference = "Stop"

$Root = Split-Path -Parent $PSScriptRoot
Set-Location $Root

if ($Version -notmatch '^\d+\.\d+\.\d+$') {
    throw "Version must look like 0.1.0 (got: $Version)"
}

$IsccCandidates = @(
    "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
    "${env:ProgramFiles}\Inno Setup 6\ISCC.exe"
)
$Iscc = $IsccCandidates | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $Iscc) {
    throw "Inno Setup compiler (ISCC.exe) not found. Install JRSoftware.InnoSetup."
}

$Dist = Join-Path $Root "dist"
$ReleaseExe = Join-Path $Root "target\release\apksule.exe"
New-Item -ItemType Directory -Path $Dist -Force | Out-Null

if (-not $SkipBuild) {
    Write-Host "==> cargo build --release -p apksule"
    cargo build --release -p apksule
}

if (-not (Test-Path $ReleaseExe)) {
    throw "Missing release binary: $ReleaseExe"
}

Write-Host "==> compiling Inno Setup installer (version $Version)"
& $Iscc `
    "/DMyAppVersion=$Version" `
    "/DSourceDir=$Root\target\release" `
    "/DOutputDir=$Dist" `
    "/DIconFile=$Root\assets\apksule.ico" `
    "$Root\installer\apksule.iss"
if ($LASTEXITCODE -ne 0) {
    throw "ISCC failed with exit code $LASTEXITCODE"
}

$Installer = Get-ChildItem $Dist -Filter "Apksule-Setup-*.exe" | Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (-not $Installer) {
    throw "Installer was not produced in $Dist"
}

Copy-Item $ReleaseExe (Join-Path $Dist "apksule.exe") -Force
Copy-Item (Join-Path $Root "assets\apksule.ico") (Join-Path $Dist "apksule.ico") -Force
Copy-Item (Join-Path $Root "assets\apksule-avatar.png") (Join-Path $Dist "apksule-avatar.png") -Force

Write-Host "==> artifacts"
Get-ChildItem $Dist | Format-Table Name, Length, LastWriteTime -AutoSize
Write-Host "Installer: $($Installer.FullName)"
