# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# NodeRunner: Mainnet Protocol — MSI Package Builder (Windows)
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
#
# Usage (PowerShell, run from project root):
#   .\build-msi.ps1              Build MSI with all features
#   .\build-msi.ps1 -Minimal     Build without gamepad/sound
#   .\build-msi.ps1 -NoBuild     Skip cargo build (use existing binary)
#   .\build-msi.ps1 -Help        Show help
#
# Requirements:
#   - Rust toolchain (cargo)
#   - WiX Toolset v3 or v4
#     v3: https://wixtoolset.org/docs/wix3/
#     v4: dotnet tool install --global wix
#
# Output:
#   dist\noderunner-0.3.2.msi
#
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

param(
    [switch]$Minimal,
    [switch]$NoBuild,
    [switch]$Help
)

$ErrorActionPreference = "Stop"

# ── Package Metadata ──
$AppName    = "NodeRunner"
$ExeName    = "noderunner"
$Version    = "0.3.2"
$Publisher  = "NodeRunner Team"
$Comment    = "Terminal-based action puzzle game"
$HelpUrl    = "https://github.com/example/noderunner"

# These GUIDs must remain stable across versions for proper upgrades.
# Generated once; do NOT change them.
$UpgradeCode   = "7A3F8B2E-1C4D-4E5F-A6B7-8C9D0E1F2A3B"
$ProductCode    = "*"  # Auto-generate per build (standard practice)

$ScriptDir  = Split-Path -Parent $MyInvocation.MyCommand.Path
$DistDir    = Join-Path $ScriptDir "dist"
$BuildDir   = Join-Path $ScriptDir "build\msi"
$WxsFile    = Join-Path $BuildDir "noderunner.wxs"

# ── Helpers ──
function Write-Info  ($msg) { Write-Host "[INFO]  $msg" -ForegroundColor Cyan }
function Write-Ok    ($msg) { Write-Host "[OK]    $msg" -ForegroundColor Green }
function Write-Warn  ($msg) { Write-Host "[WARN]  $msg" -ForegroundColor Yellow }
function Write-Err   ($msg) { Write-Host "[ERROR] $msg" -ForegroundColor Red }

function Show-Banner {
    Write-Host ""
    Write-Host "  +-------------------------------------------+" -ForegroundColor Cyan
    Write-Host "  |  NodeRunner MSI Builder  v$Version          |" -ForegroundColor Cyan
    Write-Host "  |  Windows Installer Package                |" -ForegroundColor Cyan
    Write-Host "  +-------------------------------------------+" -ForegroundColor Cyan
    Write-Host ""
}

# ── Help ──
if ($Help) {
    Show-Banner
    Write-Host "Usage: .\build-msi.ps1 [OPTIONS]"
    Write-Host ""
    Write-Host "  (none)       Build MSI with all features (gamepad + sound)"
    Write-Host "  -Minimal     Build without gamepad and sound"
    Write-Host "  -NoBuild     Skip cargo build, use existing binary"
    Write-Host "  -Help        Show this help"
    Write-Host ""
    Write-Host "Requirements:"
    Write-Host "  - Rust toolchain  https://rustup.rs"
    Write-Host "  - WiX Toolset v3  https://wixtoolset.org/docs/wix3/"
    Write-Host "    or WiX v4:      dotnet tool install --global wix"
    Write-Host ""
    Write-Host "Output: dist\noderunner-$Version.msi"
    Write-Host ""
    exit 0
}

Show-Banner

# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# Find WiX Toolset
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

$WixVersion = 0
$CandleExe = $null
$LightExe  = $null
$WixExe    = $null

function Find-WiX {
    # Try WiX v4+/v5+/v6 (wix.exe in PATH or dotnet tool)
    $wix4 = Get-Command "wix" -ErrorAction SilentlyContinue
    if ($wix4) {
        $script:WixVersion = 4
        $script:WixExe = $wix4.Source
        Write-Ok "WiX v4+ found: $($wix4.Source)"
        return
    }

    # Search common WiX v4+/v6 install locations
    $wixModernPaths = @(
        "${env:ProgramFiles}\WiX Toolset v6.0\bin",
        "${env:ProgramFiles}\WiX Toolset v5.0\bin",
        "${env:ProgramFiles}\WiX Toolset v4.0\bin",
        "${env:ProgramFiles(x86)}\WiX Toolset v6.0\bin",
        "${env:ProgramFiles(x86)}\WiX Toolset v5.0\bin",
        "${env:ProgramFiles(x86)}\WiX Toolset v4.0\bin"
    )
    foreach ($dir in $wixModernPaths) {
        $w = Join-Path $dir "wix.exe"
        if (Test-Path $w) {
            $script:WixVersion = 4
            $script:WixExe = $w
            Write-Ok "WiX v4+ found: $dir"
            return
        }
    }

    # Try WiX v3 in PATH
    $candle = Get-Command "candle.exe" -ErrorAction SilentlyContinue
    $light  = Get-Command "light.exe"  -ErrorAction SilentlyContinue
    if ($candle -and $light) {
        $script:WixVersion = 3
        $script:CandleExe = $candle.Source
        $script:LightExe  = $light.Source
        Write-Ok "WiX v3 found in PATH"
        return
    }

    # Search common WiX v3 install locations
    $searchPaths = @(
        "${env:ProgramFiles(x86)}\WiX Toolset v3.14\bin",
        "${env:ProgramFiles(x86)}\WiX Toolset v3.11\bin",
        "${env:ProgramFiles}\WiX Toolset v3.14\bin",
        "${env:ProgramFiles}\WiX Toolset v3.11\bin"
    )
    if ($env:WIX) {
        $searchPaths = @("$env:WIX\bin") + $searchPaths
    }

    foreach ($dir in $searchPaths) {
        $c = Join-Path $dir "candle.exe"
        $l = Join-Path $dir "light.exe"
        if ((Test-Path $c) -and (Test-Path $l)) {
            $script:WixVersion = 3
            $script:CandleExe = $c
            $script:LightExe  = $l
            Write-Ok "WiX v3 found: $dir"
            return
        }
    }

    Write-Err "WiX Toolset not found."
    Write-Host ""
    Write-Host "  Install WiX v3:"
    Write-Host "    https://wixtoolset.org/docs/wix3/"
    Write-Host ""
    Write-Host "  Or install WiX v4+:"
    Write-Host "    dotnet tool install --global wix"
    Write-Host ""
    exit 1
}

Find-WiX

# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# Build Binary
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

if (-not $NoBuild) {
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        Write-Err "cargo not found. Install Rust: https://rustup.rs"
        exit 1
    }

    Write-Info "Building $ExeName (release)..."
    Push-Location $ScriptDir
    try {
        if ($Minimal) {
            Write-Info "Features: minimal"
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
    Write-Ok "Build complete"
} else {
    Write-Info "Skipping build (-NoBuild)"
}

$Binary = Join-Path $ScriptDir "target\release\$ExeName.exe"
if (-not (Test-Path $Binary)) {
    Write-Err "Binary not found: $Binary"
    exit 1
}
$binarySize = (Get-Item $Binary).Length / 1MB
Write-Ok ("Binary: {0:N1} MB" -f $binarySize)

# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# Generate deterministic GUIDs from strings
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

function New-DeterministicGuid {
    param([string]$Name)
    # UUID v5 (SHA-1 based) in the "noderunner" namespace
    $ns = [guid]"7A3F8B2E-1C4D-4E5F-A6B7-8C9D0E1F2A3B"
    $enc = [System.Text.Encoding]::UTF8
    $sha = [System.Security.Cryptography.SHA1]::Create()
    $bytes = $sha.ComputeHash($enc.GetBytes($ns.ToString() + $Name))
    $bytes[6] = ($bytes[6] -band 0x0F) -bor 0x50  # version 5
    $bytes[8] = ($bytes[8] -band 0x3F) -bor 0x80  # variant
    $hex = [BitConverter]::ToString($bytes[0..15]).Replace("-","")
    $g = "$($hex.Substring(0,8))-$($hex.Substring(8,4))-$($hex.Substring(12,4))-$($hex.Substring(16,4))-$($hex.Substring(20,12))"
    return $g.ToUpper()
}

# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# Generate WiX Source (.wxs)
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Write-Info "Generating WiX source..."

New-Item -ItemType Directory -Force -Path $BuildDir | Out-Null

# Collect level files
$levelsDir = Join-Path $ScriptDir "levels"
$levelFiles = @()
if (Test-Path $levelsDir) {
    $levelFiles = Get-ChildItem -Path $levelsDir -Filter "*.txt" | Sort-Object Name
}

# Collect pack files
$packsDir = Join-Path $ScriptDir "packs"
$packFiles = @()
if (Test-Path $packsDir) {
    $packFiles = Get-ChildItem -Path $packsDir -Filter "*.nlp" | Sort-Object Name
}

# Build component XML for level files
$levelComponents = ""
$levelComponentRefs = ""
foreach ($f in $levelFiles) {
    $id = "Level_" + ($f.BaseName -replace '[^A-Za-z0-9_]', '_')
    $guid = New-DeterministicGuid "level/$($f.Name)"
    $levelComponents += @"

            <Component Id="$id" Guid="$guid">
              <File Id="$id`_file" Source="$($f.FullName)" KeyPath="yes" />
            </Component>
"@
    $levelComponentRefs += @"

        <ComponentRef Id="$id" />
"@
}

# Build component XML for pack files
$packComponents = ""
$packComponentRefs = ""
foreach ($f in $packFiles) {
    $id = "Pack_" + ($f.BaseName -replace '[^A-Za-z0-9_]', '_')
    $guid = New-DeterministicGuid "pack/$($f.Name)"
    $packComponents += @"

            <Component Id="$id" Guid="$guid">
              <File Id="$id`_file" Source="$($f.FullName)" KeyPath="yes" />
            </Component>
"@
    $packComponentRefs += @"

        <ComponentRef Id="$id" />
"@
}

# Config file path
$configFile = Join-Path $ScriptDir "config.toml"
$readmeFile = Join-Path $ScriptDir "README.md"

# ── Generate version-specific WXS ──

if ($WixVersion -eq 3) {

# ═══════════════════════════════════════
# WiX v3 XML
# ═══════════════════════════════════════
$wxsContent = @"
<?xml version="1.0" encoding="UTF-8"?>
<Wix xmlns="http://schemas.microsoft.com/wix/2006/wi">

  <Product Id="$ProductCode"
           Name="$AppName"
           Language="1033"
           Version="$Version.0"
           Manufacturer="$Publisher"
           UpgradeCode="$UpgradeCode">

    <Package InstallerVersion="200"
             Compressed="yes"
             InstallScope="perMachine"
             Description="$Comment"
             Comments="$AppName v$Version" />

    <MajorUpgrade DowngradeErrorMessage=
      "A newer version of $AppName is already installed." />

    <MediaTemplate EmbedCab="yes" />

    <Icon Id="NodeRunnerIcon" SourceFile="$Binary" />
    <Property Id="ARPPRODUCTICON" Value="NodeRunnerIcon" />
    <Property Id="ARPHELPLINK" Value="$HelpUrl" />

    <Directory Id="TARGETDIR" Name="SourceDir">
      <Directory Id="ProgramFilesFolder">
        <Directory Id="INSTALLFOLDER" Name="$AppName">
          <Directory Id="LevelsFolder" Name="levels" />
          <Directory Id="PacksFolder"  Name="packs" />
        </Directory>
      </Directory>
      <Directory Id="ProgramMenuFolder">
        <Directory Id="AppMenuFolder" Name="$AppName" />
      </Directory>
    </Directory>

    <DirectoryRef Id="INSTALLFOLDER">
      <Component Id="MainExecutable" Guid="$(New-DeterministicGuid 'exe/noderunner')">
        <File Id="noderunner_exe" Source="$Binary" KeyPath="yes" />
        <Environment Id="PATH" Name="PATH" Value="[INSTALLFOLDER]"
                     Permanent="no" Part="last" Action="set" System="yes" />
      </Component>
      <Component Id="ConfigFile" Guid="$(New-DeterministicGuid 'config/config.toml')"
                 NeverOverwrite="yes">
        <File Id="config_toml" Source="$configFile" KeyPath="yes" />
      </Component>
      <Component Id="ReadmeFile" Guid="$(New-DeterministicGuid 'doc/readme')">
        <File Id="readme_md" Source="$readmeFile" KeyPath="yes" />
      </Component>
    </DirectoryRef>

    <DirectoryRef Id="LevelsFolder">$levelComponents
    </DirectoryRef>

    <DirectoryRef Id="PacksFolder">$packComponents
    </DirectoryRef>

    <DirectoryRef Id="AppMenuFolder">
      <Component Id="StartMenuShortcut" Guid="$(New-DeterministicGuid 'shortcut/startmenu')">
        <Shortcut Id="AppShortcut" Name="$AppName" Description="$Comment"
                  Target="[INSTALLFOLDER]$ExeName.exe"
                  WorkingDirectory="INSTALLFOLDER" Icon="NodeRunnerIcon" />
        <RemoveFolder Id="RemoveAppMenuFolder" On="uninstall" />
        <RegistryValue Root="HKCU" Key="Software\$AppName" Name="installed"
                       Type="integer" Value="1" KeyPath="yes" />
      </Component>
    </DirectoryRef>

    <Feature Id="Complete" Title="$AppName" Level="1">
      <ComponentRef Id="MainExecutable" />
      <ComponentRef Id="ConfigFile" />
      <ComponentRef Id="ReadmeFile" />
      <ComponentRef Id="StartMenuShortcut" />
$levelComponentRefs
$packComponentRefs
    </Feature>

    <UIRef Id="WixUI_InstallDir" />
    <Property Id="WIXUI_INSTALLDIR" Value="INSTALLFOLDER" />
    <WixVariable Id="WixUILicenseRtf" Value="$BuildDir\license.rtf" />

  </Product>
</Wix>
"@

} else {

# ═══════════════════════════════════════
# WiX v4+ / v5+ / v6 XML
# ═══════════════════════════════════════
$wxsContent = @"
<?xml version="1.0" encoding="UTF-8"?>
<Wix xmlns="http://wixtoolset.org/schemas/v4/wxs"
     xmlns:ui="http://wixtoolset.org/schemas/v4/wxs/ui">

  <Package Name="$AppName"
           Language="1033"
           Version="$Version.0"
           Manufacturer="$Publisher"
           UpgradeCode="$UpgradeCode"
           Scope="perMachine">

    <SummaryInformation Description="$Comment" />

    <MajorUpgrade DowngradeErrorMessage=
      "A newer version of $AppName is already installed." />

    <MediaTemplate EmbedCab="yes" />

    <Icon Id="NodeRunnerIcon" SourceFile="$Binary" />
    <Property Id="ARPPRODUCTICON" Value="NodeRunnerIcon" />
    <Property Id="ARPHELPLINK" Value="$HelpUrl" />

    <!-- ── Directory Structure ── -->
    <StandardDirectory Id="ProgramFilesFolder">
      <Directory Id="INSTALLFOLDER" Name="$AppName">
        <Directory Id="LevelsFolder" Name="levels" />
        <Directory Id="PacksFolder"  Name="packs" />
      </Directory>
    </StandardDirectory>

    <StandardDirectory Id="ProgramMenuFolder">
      <Directory Id="AppMenuFolder" Name="$AppName" />
    </StandardDirectory>

    <!-- ── Components ── -->
    <DirectoryRef Id="INSTALLFOLDER">
      <Component Id="MainExecutable" Guid="$(New-DeterministicGuid 'exe/noderunner')">
        <File Id="noderunner_exe" Source="$Binary" KeyPath="yes" />
        <Environment Id="PATH" Name="PATH" Value="[INSTALLFOLDER]"
                     Permanent="no" Part="last" Action="set" System="yes" />
      </Component>
      <Component Id="ConfigFile" Guid="$(New-DeterministicGuid 'config/config.toml')"
                 NeverOverwrite="yes">
        <File Id="config_toml" Source="$configFile" KeyPath="yes" />
      </Component>
      <Component Id="ReadmeFile" Guid="$(New-DeterministicGuid 'doc/readme')">
        <File Id="readme_md" Source="$readmeFile" KeyPath="yes" />
      </Component>
    </DirectoryRef>

    <DirectoryRef Id="LevelsFolder">$levelComponents
    </DirectoryRef>

    <DirectoryRef Id="PacksFolder">$packComponents
    </DirectoryRef>

    <DirectoryRef Id="AppMenuFolder">
      <Component Id="StartMenuShortcut" Guid="$(New-DeterministicGuid 'shortcut/startmenu')">
        <Shortcut Id="AppShortcut" Name="$AppName" Description="$Comment"
                  Target="[INSTALLFOLDER]$ExeName.exe"
                  WorkingDirectory="INSTALLFOLDER" Icon="NodeRunnerIcon" />
        <RemoveFolder Id="RemoveAppMenuFolder" On="uninstall" />
        <RegistryValue Root="HKCU" Key="Software\$AppName" Name="installed"
                       Type="integer" Value="1" KeyPath="yes" />
      </Component>
    </DirectoryRef>

    <!-- ── Feature ── -->
    <Feature Id="Complete" Title="$AppName" Level="1">
      <ComponentRef Id="MainExecutable" />
      <ComponentRef Id="ConfigFile" />
      <ComponentRef Id="ReadmeFile" />
      <ComponentRef Id="StartMenuShortcut" />
$levelComponentRefs
$packComponentRefs
    </Feature>

    <!-- ── UI ── -->
    <Property Id="WIXUI_INSTALLDIR" Value="INSTALLFOLDER" />
    <ui:WixUI Id="WixUI_InstallDir" />
    <WixVariable Id="WixUILicenseRtf" Value="$BuildDir\license.rtf" />

  </Package>
</Wix>
"@

}  # end WiX version branch

# Write WXS
Set-Content -Path $WxsFile -Value $wxsContent -Encoding UTF8
Write-Ok "Generated: $WxsFile"
Write-Ok "  Levels: $($levelFiles.Count) files"
Write-Ok "  Packs:  $($packFiles.Count) files"

# Generate minimal license RTF (required by WixUI_InstallDir)
$licenseRtf = Join-Path $BuildDir "license.rtf"
$rtfContent = @"
{\rtf1\ansi\deff0
{\fonttbl{\f0 Consolas;}}
\f0\fs20
$AppName v$Version\par
\par
MIT License\par
\par
Copyright (c) 2025 $Publisher\par
\par
Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files, to deal in the Software
without restriction, including without limitation the rights to use, copy,
modify, merge, publish, distribute, sublicense, and/or sell copies of the
Software, and to permit persons to whom the Software is furnished to do so,
subject to the following conditions:\par
\par
The above copyright notice and this permission notice shall be included in
all copies or substantial portions of the Software.\par
\par
THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND.\par
}
"@
Set-Content -Path $licenseRtf -Value $rtfContent -Encoding ASCII
Write-Ok "Generated license.rtf"

# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# Compile MSI
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

New-Item -ItemType Directory -Force -Path $DistDir | Out-Null
$MsiFile = Join-Path $DistDir "$ExeName-$Version.msi"

Write-Host ""
Write-Info "Compiling MSI..."

if ($WixVersion -eq 3) {
    # WiX v3: candle.exe → .wixobj, then light.exe → .msi
    $wixobj = Join-Path $BuildDir "noderunner.wixobj"

    Write-Info "  candle.exe (compile)..."
    & $CandleExe -nologo `
        -out $wixobj `
        -ext WixUIExtension `
        $WxsFile

    if ($LASTEXITCODE -ne 0) {
        Write-Err "candle.exe failed (exit code $LASTEXITCODE)"
        exit 1
    }
    Write-Ok "  Compiled to .wixobj"

    Write-Info "  light.exe (link)..."
    & $LightExe -nologo `
        -out $MsiFile `
        -ext WixUIExtension `
        -spdb `
        $wixobj

    if ($LASTEXITCODE -ne 0) {
        Write-Err "light.exe failed (exit code $LASTEXITCODE)"
        exit 1
    }

} elseif ($WixVersion -eq 4) {
    # WiX v4: single `wix build` command
    Write-Info "  wix build..."
    & $WixExe build `
        -o $MsiFile `
        -ext WixToolset.UI.wixext `
        $WxsFile

    if ($LASTEXITCODE -ne 0) {
        Write-Err "wix build failed (exit code $LASTEXITCODE)"
        exit 1
    }
}

# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# Done
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

if (Test-Path $MsiFile) {
    $msiSize = (Get-Item $MsiFile).Length / 1MB
    Write-Host ""
    Write-Host "  +=========================================+" -ForegroundColor Green
    Write-Host "  |       MSI Package Complete!             |" -ForegroundColor Green
    Write-Host "  +=========================================+" -ForegroundColor Green
    Write-Host ""
    Write-Host ("  Output:  {0}  ({1:N1} MB)" -f $MsiFile, $msiSize)
    Write-Host ""
    Write-Host "  Install:"
    Write-Host "    msiexec /i `"$MsiFile`""
    Write-Host ""
    Write-Host "  Silent install:"
    Write-Host "    msiexec /i `"$MsiFile`" /qn"
    Write-Host ""
    Write-Host "  Uninstall:"
    Write-Host "    msiexec /x `"$MsiFile`""
    Write-Host ""
    Write-Host "  Install location: C:\Program Files\$AppName\"
    Write-Host ""
} else {
    Write-Err "MSI file not created."
    exit 1
}

# Cleanup intermediate files
Remove-Item -Path $BuildDir -Recurse -Force -ErrorAction SilentlyContinue
Write-Ok "Cleaned up build files"
