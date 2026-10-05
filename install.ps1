# mlo installer for Windows.
#
# Downloads the prebuilt x86_64-pc-windows-msvc release archive, verifies it
# against the release's SHA256SUMS, extracts it to
# %LOCALAPPDATA%\Programs\mlo, and puts that directory on the user PATH.
#
# Usage (PowerShell):
#   irm https://raw.githubusercontent.com/dillydalli3r/mlo/main/install.ps1 | iex
#   .\install.ps1 -Version 0.1.0
#   .\install.ps1 -InstallDir "$env:USERPROFILE\bin"
#
# Environment overrides: MLO_REPO (owner/name), MLO_VERSION.

[CmdletBinding()]
param(
    # Release version to install; the latest release when omitted.
    [string]$Version = $env:MLO_VERSION,
    # Install directory; %LOCALAPPDATA%\Programs\mlo when omitted.
    [string]$InstallDir = $env:MLO_INSTALL_DIR,
    # Repository owner/name to download from.
    [string]$Repo = $(if ($env:MLO_REPO) { $env:MLO_REPO } else { 'dillydalli3r/mlo' })
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'   # Invoke-WebRequest is much faster without it

function Fail([string]$Reason) {
    Write-Error "mlo-install: error: $Reason"
    exit 1
}

# --- platform -------------------------------------------------------------
# The prebuilt Windows archive is x86_64 only. Running on an ARM64 Windows
# machine is possible in emulation, but the binary and its PATH entry are the
# x64 ones, so this refuses rather than installing something that may misbehave.
$arch = $env:PROCESSOR_ARCHITECTURE
if ($arch -ne 'AMD64') {
    Fail "UNSUPPORTED_PLATFORM: processor architecture '$arch' (mlo publishes a prebuilt Windows build for x86_64 only; build the ARM64 target from source with 'cargo build --release --target aarch64-pc-windows-msvc')"
}

$target = 'x86_64-pc-windows-msvc'
$asset = "mlo-$Version-$target.zip"

# --- version --------------------------------------------------------------
if ([string]::IsNullOrWhiteSpace($Version)) {
    try {
        $release = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest" -UseBasicParsing
        $Version = $release.tag_name.TrimStart('v')
    } catch {
        Fail "NETWORK: could not read the latest release from https://api.github.com/repos/$Repo/releases/latest ($($_.Exception.Message))"
    }
    if ([string]::IsNullOrWhiteSpace($Version)) { Fail "NO_RELEASE: the latest release carries no tag name" }
    $asset = "mlo-$Version-$target.zip"
}

$base = "https://github.com/$Repo/releases/download/v$Version"

if ([string]::IsNullOrWhiteSpace($InstallDir)) {
    $InstallDir = Join-Path $env:LOCALAPPDATA 'Programs\mlo'
}

$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("mlo-install-" + [System.Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $tmp | Out-Null

try {
    # --- download ---------------------------------------------------------
    $assetPath = Join-Path $tmp $asset
    Write-Host "mlo-install: downloading $base/$asset"
    try {
        Invoke-WebRequest -Uri "$base/$asset" -OutFile $assetPath -UseBasicParsing
        Invoke-WebRequest -Uri "$base/SHA256SUMS" -OutFile (Join-Path $tmp 'SHA256SUMS') -UseBasicParsing
    } catch {
        Fail "DOWNLOAD_FAILED: $base/$asset (is v$Version released for $target?) — $($_.Exception.Message)"
    }

    # --- verify -----------------------------------------------------------
    $expected = (Get-Content (Join-Path $tmp 'SHA256SUMS') |
                 Where-Object { ($_ -split '\s+')[1] -eq $asset } |
                 ForEach-Object { ($_ -split '\s+')[0] } |
                 Select-Object -First 1)
    if ([string]::IsNullOrWhiteSpace($expected)) { Fail "CHECKSUM_MISSING: no SHA256SUMS entry for $asset" }

    $actual = (Get-FileHash -Algorithm SHA256 -Path $assetPath).Hash.ToLowerInvariant()
    if ($actual -ne $expected.ToLowerInvariant()) {
        Fail "CHECKSUM_MISMATCH: $asset is $actual, expected $expected"
    }

    # --- extract ----------------------------------------------------------
    $extract = Join-Path $tmp 'extract'
    New-Item -ItemType Directory -Force -Path $extract | Out-Null
    try {
        Expand-Archive -Path $assetPath -DestinationPath $extract -Force
    } catch {
        Fail "EXTRACT_FAILED: $asset — $($_.Exception.Message)"
    }
    $binary = (Get-ChildItem -Path $extract -Recurse -Filter 'mlo.exe' | Select-Object -First 1).FullName
    if ([string]::IsNullOrWhiteSpace($binary)) { Fail "ARCHIVE_LAYOUT: $asset does not contain mlo.exe" }

    # --- install ----------------------------------------------------------
    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    Copy-Item -Path $binary -Destination (Join-Path $InstallDir 'mlo.exe') -Force

    # --- PATH -------------------------------------------------------------
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if ([string]::IsNullOrWhiteSpace($userPath)) { $userPath = '' }
    $segments = $userPath -split ';' | Where-Object { $_ -ne '' }
    if ($segments -notcontains $InstallDir) {
        $newPath = (@($InstallDir) + $segments) -join ';'
        [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
        Write-Host "mlo-install: added $InstallDir to your user PATH"
    }

    Write-Host "mlo-install: installed mlo $Version -> $(Join-Path $InstallDir 'mlo.exe')"
    Write-Host "mlo-install: open a NEW terminal so the PATH change takes effect, then run:"
    Write-Host "  mlo doctor"
    Write-Host "  mlo shell install   # add the right-click menu"
}
finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}