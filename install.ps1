# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# NodeRunner: Mainnet Protocol — Installer (Windows PowerShell)
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
#
# Usage (run from project root in PowerShell):
#   .\install.ps1              Build + install (default features)
#   .\install.ps1 -Minimal     Build without gamepad/sound
#   .\install.ps1 -Uninstall   Remove installed files
#   .\install.ps1 -Help        Show help
#
# Install location:
#   %LOCALAPPDATA%\NodeRunner\
#
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

param(
    [switch]$Minimal,
    [switch]$Uninstall,
    [switch]$Help
)

$ErrorActionPreference = "Stop"

# ── Config ──
$AppName     = "noderunner"
$AppDisplay  = "NodeRunner: Mainnet Protocol"
$Version     = "0.3.2"
$DataDir     = Join-Path $env:LOCALAPPDATA "NodeRunner"
$ScriptDir   = Split-Path -Parent $MyInvocation.MyCommand.Path

# ── Helpers ──
function Write-Info  ($msg) { Write-Host "[INFO]  $msg" -ForegroundColor Cyan }
function Write-Ok    ($msg) { Write-Host "[OK]    $msg" -ForegroundColor Green }
function Write-Warn  ($msg) { Write-Host "[WARN]  $msg" -ForegroundColor Yellow }
function Write-Err   ($msg) { Write-Host "[ERROR] $msg" -ForegroundColor Red }

function Show-Banner {
    Write-Host ""
    Write-Host "  +-------------------------------------------+" -ForegroundColor Cyan
    Write-Host "  |  NodeRunner: Mainnet Protocol  v$Version   |" -ForegroundColor Cyan
    Write-Host "  |  Terminal Action Puzzle                    |" -ForegroundColor Cyan
    Write-Host "  +-------------------------------------------+" -ForegroundColor Cyan
    Write-Host ""
}

# ── Help ──
if ($Help) {
    Show-Banner
    Write-Host "Usage: .\install.ps1 [OPTIONS]"
    Write-Host ""
    Write-Host "  (none)       Build and install with all features"
    Write-Host "  -Minimal     Build without gamepad and sound"
    Write-Host "  -Uninstall   Remove installed files"
    Write-Host "  -Help        Show this help"
    Write-Host ""
    Write-Host "Install location: $DataDir\"
    Write-Host ""
    Write-Host "Requirements:"
    Write-Host "  - Rust toolchain  https://rustup.rs"
    Write-Host ""
    exit 0
}

# ── Uninstall ──
if ($Uninstall) {
    Show-Banner
    Write-Info "Uninstalling $AppDisplay..."

    # Remove from PATH
    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
    if ($userPath -and $userPath.Contains($DataDir)) {
        $newPath = ($userPath.Split(";") | Where-Object { $_ -ne $DataDir }) -join ";"
        [Environment]::SetEnvironmentVariable("Path", $newPath, "User")
        Write-Ok "Removed from PATH"
    }

    if (Test-Path $DataDir) {
        # Check for save files
        $saves = Get-ChildItem -Path $DataDir -Filter "save*.dat" -ErrorAction SilentlyContinue
        if ($saves) {
            Write-Warn "Save files found:"
            $saves | ForEach-Object { Write-Host "        $($_.Name)" }
            $ans = Read-Host "  Delete save files too? [y/N]"
            if ($ans -match "^[Yy]") {
                Remove-Item -Recurse -Force $DataDir
                Write-Ok "Removed $DataDir (including saves)"
            } else {
                Get-ChildItem -Path $DataDir -Exclude "save*.dat" | Remove-Item -Recurse -Force
                # Remove subdirs
                @("levels", "packs") | ForEach-Object {
                    $sub = Join-Path $DataDir $_
                    if (Test-Path $sub) { Remove-Item -Recurse -Force $sub }
                }
                Write-Ok "Removed $DataDir (save files preserved)"
            }
        } else {
            Remove-Item -Recurse -Force $DataDir
            Write-Ok "Removed $DataDir"
        }
    } else {
        Write-Info "Nothing to remove - not installed."
    }

    Write-Host ""
    Write-Ok "Uninstall complete."
    exit 0
}

# ── Main Install ──
Show-Banner

# Check Rust
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Write-Err "cargo not found. Install Rust: https://rustup.rs"
    Write-Host ""
    Write-Host "  winget install Rustlang.Rustup"
    Write-Host ""
    exit 1
}
$cargoVer = (cargo --version) -replace "cargo ", ""
Write-Ok "cargo $cargoVer"

# Build
$buildMode = if ($Minimal) { "minimal" } else { "default" }
Write-Info "Building $AppDisplay ($buildMode)..."
Write-Host ""

Push-Location $ScriptDir
try {
    if ($Minimal) {
        Write-Info "Features: none (minimal)"
        cargo build --release --no-default-features
    } else {
        Write-Info "Features: gamepad + sound"
        cargo build --release
    }
} catch {
    Write-Err "Build failed."
    Pop-Location
    exit 1
}
Pop-Location

Write-Host ""
Write-Ok "Build successful!"

# Install
Write-Info "Installing to $DataDir\..."

New-Item -ItemType Directory -Force -Path $DataDir | Out-Null

# Binary
$binary = Join-Path $ScriptDir "target\release\$AppName.exe"
if (-not (Test-Path $binary)) {
    Write-Err "Binary not found: $binary"
    exit 1
}
Copy-Item $binary -Destination (Join-Path $DataDir "$AppName.exe") -Force
Write-Ok "Binary installed"

# Config
$configSrc = Join-Path $ScriptDir "config.toml"
$configDst = Join-Path $DataDir "config.toml"
if (Test-Path $configSrc) {
    if (Test-Path $configDst) {
        Write-Warn "config.toml already exists - keeping existing"
        Copy-Item $configSrc -Destination "$configDst.new" -Force
        Write-Info "New default saved as config.toml.new"
    } else {
        Copy-Item $configSrc -Destination $configDst -Force
        Write-Ok "config.toml installed"
    }
}

# Levels
$levelsSrc = Join-Path $ScriptDir "levels"
if (Test-Path $levelsSrc) {
    $levelsDst = Join-Path $DataDir "levels"
    New-Item -ItemType Directory -Force -Path $levelsDst | Out-Null
    Copy-Item "$levelsSrc\*.txt" -Destination $levelsDst -Force -ErrorAction SilentlyContinue
    $count = (Get-ChildItem "$levelsDst\*.txt" -ErrorAction SilentlyContinue).Count
    Write-Ok "Levels installed ($count files)"
}

# Packs
$packsSrc = Join-Path $ScriptDir "packs"
if (Test-Path $packsSrc) {
    $packsDst = Join-Path $DataDir "packs"
    New-Item -ItemType Directory -Force -Path $packsDst | Out-Null
    Copy-Item "$packsSrc\*.nlp" -Destination $packsDst -Force -ErrorAction SilentlyContinue
    $pcount = (Get-ChildItem "$packsDst\*.nlp" -ErrorAction SilentlyContinue).Count
    Write-Ok "Level packs installed ($pcount files)"
}

# Add to PATH
$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
if (-not $userPath) { $userPath = "" }
if (-not $userPath.Contains($DataDir)) {
    [Environment]::SetEnvironmentVariable("Path", "$DataDir;$userPath", "User")
    $env:Path = "$DataDir;$env:Path"
    Write-Ok "Added to user PATH"
} else {
    Write-Ok "Already in PATH"
}

# Summary
Write-Host ""
Write-Host "  +=========================================+" -ForegroundColor Green
Write-Host "  |       Installation Complete!            |" -ForegroundColor Green
Write-Host "  +=========================================+" -ForegroundColor Green
Write-Host ""
Write-Host "  Run the game (open a new terminal):"
Write-Host ""
Write-Host "    $AppName" -ForegroundColor White
Write-Host ""
Write-Host "  Or directly:"
Write-Host "    $(Join-Path $DataDir "$AppName.exe")"
Write-Host ""
Write-Host "  Install location:  $DataDir\"
Write-Host "  Config:            $DataDir\config.toml"
Write-Host "  Levels:            $DataDir\levels\"
Write-Host "  Level packs:       $DataDir\packs\"
Write-Host "  Save data:         $DataDir\save_*.dat"
Write-Host ""
Write-Host "  Uninstall:"
Write-Host "    .\install.ps1 -Uninstall"
Write-Host ""
