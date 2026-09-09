<#
.SYNOPSIS
    One-time setup for WinTerminalP.

.DESCRIPTION
    Builds the release binaries, installs the `winter` command into the Cargo
    bin directory (which is already on PATH), and installs the Windows Terminal
    integration. Run this once after cloning. Afterwards `winter` works from any
    shell and keeps working across reboots.

.EXAMPLE
    PS> .\install.ps1
#>
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw 'Rust/Cargo was not found. Install it from https://rustup.rs and run this script again.'
}

$repoRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
Push-Location $repoRoot
try {
    Write-Host 'Installing winter, winterminalp, and winterd (release build)...' -ForegroundColor Cyan
    cargo install --path . --bins --locked --force
    if ($LASTEXITCODE -ne 0) {
        throw "cargo install failed with exit code $LASTEXITCODE"
    }
}
finally {
    Pop-Location
}

$cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
$winterExe = Join-Path $cargoBin 'winter.exe'
if (-not (Test-Path -LiteralPath $winterExe)) {
    throw "winter.exe was not found in $cargoBin"
}

if (($env:Path -split ';') -notcontains $cargoBin) {
    Write-Warning "$cargoBin is not on your PATH. Add it and restart your shell before running 'winter'."
}

Write-Host 'Installing the Windows Terminal integration...' -ForegroundColor Cyan
& $winterExe install
if ($LASTEXITCODE -ne 0) {
    throw "winter install failed with exit code $LASTEXITCODE"
}

Write-Host ''
Write-Host "Done. Run 'winter' to start the controller (Windows will prompt for UAC)." -ForegroundColor Green
