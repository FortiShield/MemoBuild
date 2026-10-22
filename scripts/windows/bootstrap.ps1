#Requires -Version 5.1
<#
.SYNOPSIS
    Windows bootstrap for MemoBuild.

.DESCRIPTION
    Installs the toolchain required to build MemoBuild on Windows:
    Git, the Rust toolchain, protoc, Git LFS and (optionally) cargo-audit.

.PARAMETER Yes
    Do not ask for confirmation before installing anything.

.PARAMETER DryRun
    Print what would be done, change nothing.

.PARAMETER Check
    Only verify the environment, install nothing.

.PARAMETER NoLfs
    Skip the Git LFS setup step.

.EXAMPLE
    pwsh -File scripts/windows/bootstrap.ps1 -Yes
#>
[CmdletBinding()]
param(
    [switch]$Yes,
    [switch]$DryRun,
    [switch]$Check,
    [switch]$NoLfs
)

Set-StrictMode -Version 2.0
$ErrorActionPreference = 'Stop'

# Map GNU-style flags to PowerShell switches for direct script invocation.
# When invoked via scripts/bootstrap the args are already mapped, but when
# running the platform script directly we accept both styles.
$Argv = $MyInvocation.BoundParameters.Keys
foreach ($key in $Argv) {
    $val = $MyInvocation.BoundParameters[$key]
    # Normalize bound parameter keys (strip leading dashes) so we can match
    # both GNU-style flags (--dry-run) and PowerShell's internal names (DryRun).
    $normalizedKey = $key -replace '^-+'
    if ($normalizedKey -match 'dry-run' -or $normalizedKey -eq 'DryRun') { $DryRun = $true }
    if ($normalizedKey -match 'check' -or $normalizedKey -eq 'Check') { $Check = $true }
    if ($normalizedKey -match 'no-lfs' -or $normalizedKey -eq 'NoLfs') { $NoLfs = $true }
    if ($normalizedKey -match 'y' -or $normalizedKey -eq 'Yes') { $Yes = $true }
}
}

$RepoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$env:MEMOBUILD_REPO_ROOT = $RepoRoot

# ---------------------------------------------------------------------------
# Output helpers
# ---------------------------------------------------------------------------

function Write-Step { param([string]$Message) Write-Host "==> $Message" -ForegroundColor Cyan }
function Write-Ok { param([string]$Message) Write-Host "  ok $Message" -ForegroundColor Green }
function Write-Info { param([string]$Message) Write-Host "==> $Message" -ForegroundColor Blue }
function Write-Skip { param([string]$Message) Write-Host "skipped $Message" -ForegroundColor Yellow }
function Write-Warn { param([string]$Message) Write-Host "warning: $Message" -ForegroundColor Yellow }
function Write-Err { param([string]$Message) Write-Host "error: $Message" -ForegroundColor Red }

function Test-Command {
    param([string]$Name)
    $null -ne (Get-Command $Name -ErrorAction SilentlyContinue)
}

function Confirm-Action {
    param([string]$Question)

    if ($Yes -or $env:MEMOBUILD_SKIP_SUDO_PROMPT -eq '1') { return $true }

    # There is nobody around to answer the question, so continue instead of
    # blocking the run.
    if ([Console]::IsInputRedirected) { return $true }

    $answer = Read-Host "$Question [y/N]"
    return $answer -match '^(y|yes)$'
}

# ---------------------------------------------------------------------------
# PATH handling
# ---------------------------------------------------------------------------

function Add-ToUserPath {
    param([string]$Directory)

    if ([string]::IsNullOrWhiteSpace($Directory)) { return }
    if (-not (Test-Path $Directory)) { return }

    $current = [Environment]::GetEnvironmentVariable('Path', 'User')
    if ($null -eq $current) { $current = '' }

    $entries = $current -split ';' | Where-Object { $_ -ne '' }
    if ($entries -notcontains $Directory) {
        [Environment]::SetEnvironmentVariable('Path', (($entries + $Directory) -join ';'), 'User')
        Write-Ok "added $Directory to the user PATH"
    }

    if (($env:Path -split ';') -notcontains $Directory) {
        $env:Path = "$env:Path;$Directory"
    }
}

# Pick up the PATH entries that an installer just persisted. winget and
# chocolatey update the machine or user PATH, but the current process keeps
# the environment it was started with, so tools installed during this run
# would stay invisible without this refresh.
function Update-SessionPath {
    $persisted = @(
        [Environment]::GetEnvironmentVariable('Path', 'Machine'),
        [Environment]::GetEnvironmentVariable('Path', 'User')
    ) | Where-Object { $_ -ne '' }

    $entries = @($env:Path -split ';' | Where-Object { $_ -ne '' })
    foreach ($path in ($persisted -split ';')) {
        if ($path -ne '' -and ($entries -notcontains $path)) {
            $entries += $path
        }
    }
    $env:Path = $entries -join ';'
}

# ---------------------------------------------------------------------------
# Package managers
# ---------------------------------------------------------------------------

function Install-Winget {
    param([string]$Id, [string]$Name)

    if (-not (Test-Command 'winget')) { return $false }

    Write-Step "installing $Name with winget"
    if ($DryRun) { Write-Skip "would run: winget install --id $Id"; return $true }

    & winget install --id $Id -e --source winget --accept-source-agreements --accept-package-agreements --silent | Out-Host
    if ($LASTEXITCODE -ne 0) {
        Write-Warn "winget could not install $Name"
        return $false
    }
    Update-SessionPath
    return $true
}

function Install-Choco {
    param([string]$Package, [string]$Name)

    if (-not (Test-Command 'choco')) { return $false }

    Write-Step "installing $Name with chocolatey"
    if ($DryRun) { Write-Skip "would run: choco install $Package -y"; return $true }

    & choco install $Package -y --no-progress | Out-Host
    if ($LASTEXITCODE -ne 0) {
        Write-Warn "chocolatey could not install $Name"
        return $false
    }
    Update-SessionPath
    return $true
}

# ---------------------------------------------------------------------------
# Toolchain steps
# ---------------------------------------------------------------------------

function Ensure-Git {
    if (Test-Command 'git') {
        Write-Ok "git is already installed"
        return
    }

    if (-not (Install-Winget -Id 'Git.Git' -Name 'Git')) {
        Install-Choco -Package 'git' -Name 'Git' | Out-Null
    }

    if (-not (Test-Command 'git')) {
        if (-not $DryRun) {
            Write-Warn "Git is still missing, install it from https://git-scm.com/download/win"
        }
    }
}

function Ensure-GitLfs {
    if (Test-Command 'git-lfs') {
        Write-Ok "git-lfs is already installed"
        return
    }

    if (-not (Test-Command 'git')) {
        # git lfs is a git extension, it cannot be configured without git.
        if (-not $DryRun) {
            Write-Warn "git is missing, skipping the git-lfs setup"
        }
        return
    }

    if (-not (Install-Winget -Id 'GitHub.GitLFS' -Name 'Git LFS')) {
        Install-Choco -Package 'git-lfs' -Name 'Git LFS' | Out-Null
    }

    if (-not (Test-Command 'git-lfs')) {
        if (-not $DryRun) {
            Write-Warn "git-lfs is still missing, install it from https://git-lfs.com"
        }
        return
    }

    if ($DryRun) { return }
    & git lfs install --local | Out-Null
    Write-Ok "git-lfs installed"
}

function Ensure-Rust {
    function $compare-versions($a, $b) {
        # Compare two dotted numeric versions. Returns $true if $a >= $b.
        $partsA = $a -split '\.'
        $partsB = $b -split '\.'
        for ($i = 0; $i -max 3; $i++) {
            $va = $partsA[$i] -replace '[^0-9]', '0'
            $vb = $partsB[$i] -replace '[^0-9]', '0'
            if ($va -gt $vb) { return $true }
            if ($va -lt $vb) { return $false }
        }
        return $true
    }

    if (Test-Command 'cargo') {
        $rustcOutput = & rustc --version 2>$null
        if ($rustcOutput) {
            $rustcVersion = $rustcOutput -replace 'rustc\s+', ''
            # Minimum acceptable rustc version (1.75.0)
            $minVersion = '1.75.0'
            if (-not $compare-versions $rustcVersion $minVersion) {
                Write-Warn "rustc $rustcVersion is older than the required $minVersion, upgrading..."
                if (-not (Install-Winget -Id 'Rustlang.Rustup' -Name 'Rustup' -ErrorAction SilentlyContinue)) {
                    $installer = Join-Path $env:TEMP 'rustup-init.exe'
                    if ($DryRun) {
                        Write-Skip 'would download https://win.rustup.rs/x86_64 and run rustup-init.exe'
                    } else {
                        Invoke-WebRequest -Uri 'https://win.rustup.rs/x86_64' -OutFile $installer -UseBasicParsing
                        & $installer -y --profile default --default-toolchain stable | Out-Host
                    }
                }
                $rustupInit = Get-Command 'rustup-init.exe' -ErrorAction SilentlyContinue
                if ($rustupInit -and -not $DryRun) {
                    & $rustupInit.Source -y --profile default --default-toolchain stable | Out-Host
                }
            }
        }
        Add-ToUserPath "$env:USERPROFILE\.cargo\bin"

        if (-not (Test-Command 'cargo')) {
            if (-not $DryRun) {
                Write-Warn "cargo is still missing, install Rust from https://rustup.rs and re-run the bootstrap"
            }
            return
        }
        Write-Ok "rustc $(& rustc --version)"
        return
    }

    if (-not (Install-Winget -Id 'Rustlang.Rustup' -Name 'Rustup')) {
        Write-Step 'downloading rustup-init'
        $installer = Join-Path $env:TEMP 'rustup-init.exe'
        if ($DryRun) {
            Write-Skip 'would download https://win.rustup.rs/x86_64 and run rustup-init.exe'
        } else {
            Invoke-WebRequest -Uri 'https://win.rustup.rs/x86_64' -OutFile $installer -UseBasicParsing
            & $installer -y --profile default --default-toolchain stable | Out-Host
        }
    }

    $rustupInit = Get-Command 'rustup-init.exe' -ErrorAction SilentlyContinue
    if ($rustupInit -and -not $DryRun) {
        & $rustupInit.Source -y --profile default --default-toolchain stable | Out-Host
    }

    Add-ToUserPath "$env:USERPROFILE\.cargo\bin"

    if (-not (Test-Command 'cargo')) {
        if (-not $DryRun) {
            Write-Warn "cargo is still missing, install Rust from https://rustup.rs and re-run the bootstrap"
        }
        return
    }
    Write-Ok "rustc $(& rustc --version)"
}

# The protoc release archive that Ensure-Protoc installs. The release metadata
# and the asset URL are mutable, so both the version and the archive digest are
# pinned here, in the repo, where they are reviewed: the digest is what
# authenticates the download, and nothing is extracted - let alone executed -
# until the bytes match it. A mismatch aborts the bootstrap.
#
# Regenerate when bumping $ProtocVersion:
#
#   curl -sSf https://api.github.com/repos/protocolbuffers/protobuf/releases/tags/v<version> \
#     | jq -r '.assets[] | select(.name == "protoc-<version>-win64.zip") | .digest'
#
# and cross-check the value against an independent channel that re-hashes the
# asset, eg. the InstallerSha256 of Google.Protobuf in microsoft/winget-pkgs.
$ProtocVersion = '36.2'
$ProtocArchive = "protoc-$ProtocVersion-win64.zip"
$ProtocSha256 = 'f0c128dc0d8492eceece83bb459a4c0e316764b929ffbf1aa416357fd644edd3'

function Ensure-Protoc {
    if (Test-Command 'protoc') {
        Write-Ok "protoc is already installed"
        return
    }

    if (Install-Choco -Package 'protobuf' -Name 'protoc') {
        if (-not [string]::IsNullOrWhiteSpace($env:ChocolateyInstall)) {
            Add-ToUserPath "$env:ChocolateyInstall\bin"
        }
        return
    }

    # Fall back to the pinned official release archive.
    $toolsDir = Join-Path $env:LOCALAPPDATA 'MemoBuild\tools'
    $protocDir = Join-Path $toolsDir 'protoc'

    if ($DryRun) {
        Write-Skip "would download protoc v$ProtocVersion into $protocDir"
        return
    }

    Write-Step "downloading protoc v$ProtocVersion"
    New-Item -ItemType Directory -Force -Path $protocDir | Out-Null

    $archive = Join-Path $env:TEMP $ProtocArchive
    $url = "https://github.com/protocolbuffers/protobuf/releases/download/v$ProtocVersion/$ProtocArchive"
    Invoke-WebRequest -Uri $url -OutFile $archive -UseBasicParsing

    $actualSha256 = (Get-FileHash -Path $archive -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualSha256 -ne $ProtocSha256) {
        Remove-Item -Path $archive -Force -ErrorAction SilentlyContinue
        Write-Err "$ProtocArchive failed verification, refusing to extract or run it"
        Write-Err "  expected SHA-256 $ProtocSha256"
        Write-Err "  actual SHA-256   $actualSha256"
        throw "$ProtocArchive failed SHA-256 verification"
    }

    Expand-Archive -Path $archive -DestinationPath $protocDir -Force
    Remove-Item -Path $archive -Force -ErrorAction SilentlyContinue

    Add-ToUserPath $protocDir
    Write-Ok "protoc v$ProtocVersion installed in $protocDir"
}

function Ensure-CargoAudit {
    if (Test-Command 'cargo-audit') {
        Write-Ok "cargo-audit is already installed"
        return
    }

    if ($env:MEMOBUILD_SKIP_CARGO_AUDIT -eq '1') {
        Write-Skip 'cargo-audit (MEMOBUILD_SKIP_CARGO_AUDIT=1)'
        return
    }

    if ($DryRun) {
        Write-Skip 'would install cargo-audit'
        return
    }

    Write-Step 'installing cargo-audit (this compiles from source and takes a few minutes)'
    & cargo install cargo-audit --locked | Out-Host
    if ($LASTEXITCODE -ne 0) {
        Write-Warn "cargo install cargo-audit --locked failed, retrying without --locked"
        & cargo install cargo-audit | Out-Host
    }
    Write-Ok 'cargo-audit installed'
}

function Check-Docker {
    if (Test-Command 'docker') {
        Write-Ok "docker is already installed"
        return
    }
    Write-Skip 'docker (install Docker Desktop from https://docs.docker.com/desktop/install/windows-install/)'
}

# ---------------------------------------------------------------------------
# Verification
# ---------------------------------------------------------------------------

function Test-Environment {
    $failures = 0

    Write-Host ''
    Write-Host 'Verifying the MemoBuild toolchain'
    Write-Host '-----------------------------------'

    if (Test-Command 'cargo') {
        Write-Ok "cargo           $(& cargo --version)"
    } else {
        Write-Err 'cargo           missing (required)'
        $failures++
    }

    if (Test-Command 'rustc') {
        Write-Ok "rustc           $(& rustc --version)"
    } else {
        Write-Err 'rustc           missing (required)'
        $failures++
    }

    $cCompiler = @('cl.exe', 'gcc.exe', 'clang.exe') | Where-Object { Test-Command $_ } | Select-Object -First 1
    if ($cCompiler) {
        Write-Ok 'c compiler      available'
    } else {
        Write-Warn 'c compiler      missing (install the Visual Studio Build Tools)'
        $failures++
    }

    if (Test-Command 'protoc') {
        Write-Ok "protoc          $(& protoc --version)"
    } else {
        Write-Warn 'protoc          missing (only needed for --all-features builds)'
    }

    if (Test-Command 'cargo-audit') {
        Write-Ok 'cargo-audit     installed'
    } else {
        Write-Warn 'cargo-audit     missing (optional, used by scripts/security-audit.sh)'
    }

    if (Test-Command 'git-lfs') {
        Write-Ok 'git-lfs         installed'
    } else {
        Write-Warn 'git-lfs         missing'
    }

    if (Test-Command 'docker') {
        Write-Ok "docker          $(& docker --version)"
    } else {
        Write-Warn 'docker          missing (optional, needed to build the extension)'
    }

    if (Test-Path (Join-Path $RepoRoot 'Cargo.toml')) {
        Push-Location $RepoRoot
        try {
            # A failing native command does not raise a terminating error in
            # Windows PowerShell 5.1, so the exit code has to be inspected.
            & cargo metadata --offline --no-deps --format-version 1 | Out-Null
            if ($LASTEXITCODE -eq 0) {
                Write-Ok 'manifest        Cargo.toml resolves'
            } else {
                Write-Err 'manifest        Cargo.toml does not resolve'
                $failures++
            }
        } finally {
            Pop-Location
        }
    }

    if ($failures -eq 0) {
        Write-Ok 'environment is ready'
    } else {
        Write-Err "$failures required item(s) are missing"
    }

    return $failures
}

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

function Invoke-Main {
    if (-not (Test-Command 'winget') -and -not (Test-Command 'choco')) {
        Write-Err 'neither winget nor chocolatey was found, install one of them first'
        return 1
    }

    if ($Check) {
        Write-Info 'check-only mode, nothing will be installed'
        $failures = Test-Environment
        if ($failures -gt 0) { return $failures }
        return 0
    }

    $install = $true
    if ($DryRun) {
        Write-Info 'dry-run mode, nothing will be installed'
    } elseif (-not (Confirm-Action 'Install the missing MemoBuild toolchain components?')) {
        Write-Warn 'installation was declined, nothing was installed'
        $install = $false
    }

    if ($install) {
        # Git comes first, git-lfs is a git extension and needs it.
        Ensure-Git
        if (-not $NoLfs) {
            Ensure-GitLfs
        }
        Ensure-Rust
        Ensure-Protoc
        Ensure-CargoAudit
        Check-Docker
    }

    $failures = Test-Environment
    if ($failures -gt 0) { return $failures }
    return 0
}

$exitCode = Invoke-Main
exit $exitCode
