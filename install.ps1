# SPDX-License-Identifier: Apache-2.0
<#
.SYNOPSIS
    llm-verify installer for Windows.

.DESCRIPTION
    Downloads the release archive matching this machine's architecture,
    verifies its SHA-256 against the published SHA256SUMS, and installs the
    binary into a per-user directory.

.EXAMPLE
    irm https://raw.githubusercontent.com/asale-ai/llm-verify/main/install.ps1 | iex

.NOTES
    Environment overrides:
      LLM_VERIFY_VERSION   install a specific tag (default: latest)
      LLM_VERIFY_BIN_DIR   install location (default: %LOCALAPPDATA%\Programs\llm-verify)
#>

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$Repo = 'asale-ai/llm-verify'
$Bin  = 'llm-verify.exe'
$BinDir = if ($env:LLM_VERIFY_BIN_DIR) {
    $env:LLM_VERIFY_BIN_DIR
} else {
    Join-Path $env:LOCALAPPDATA 'Programs\llm-verify'
}

function Say  { param($m) Write-Host "  $m" }
function Ok   { param($m) Write-Host "  " -NoNewline; Write-Host "✓" -ForegroundColor Green -NoNewline; Write-Host " $m" }
function Warn { param($m) Write-Host "  " -NoNewline; Write-Host "!" -ForegroundColor Yellow -NoNewline; Write-Host " $m" }
function Die  {
    param($m)
    Write-Host ""
    Write-Host "  Install failed: " -ForegroundColor Red -NoNewline
    Write-Host $m
    Write-Host ""
    exit 1
}

Write-Host ""
Write-Host "  llm-verify installer" -ForegroundColor DarkGray
Write-Host ""

# ── platform ──────────────────────────────────────────────────────────────
$arch = $env:PROCESSOR_ARCHITECTURE
switch ($arch) {
    'AMD64' { $target = 'x86_64-pc-windows-msvc' }
    'ARM64' {
        Die "No prebuilt Windows ARM64 artefact yet.
       Run the x64 build under emulation, or build from source:
       cargo install --git https://github.com/$Repo"
    }
    default { Die "Unsupported CPU architecture: $arch. Only x86_64 is published." }
}
Say "Platform : $target"

# TLS 1.2 is not the default on older Windows PowerShell hosts.
try {
    [Net.ServicePointManager]::SecurityProtocol =
        [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
} catch { }

# ── version ───────────────────────────────────────────────────────────────
$version = $env:LLM_VERIFY_VERSION
if (-not $version) {
    Say "Resolving the latest version…"
    try {
        $rel = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest" `
                                 -Headers @{ 'User-Agent' = 'llm-verify-installer' }
        $version = $rel.tag_name
    } catch {
        Die "Could not resolve the latest version: $($_.Exception.Message)
       The network may be down or the GitHub API rate-limited.
       Pin one instead: `$env:LLM_VERIFY_VERSION='v0.2.0'; irm ... | iex"
    }
}
if (-not $version) { Die "GitHub returned no usable version tag." }
$num = $version.TrimStart('v')
Say "Version  : $version"

# ── download ──────────────────────────────────────────────────────────────
$name  = "llm-verify-$num-$target"
$asset = "$name.zip"
$url   = "https://github.com/$Repo/releases/download/$version/$asset"

$tmp = Join-Path ([IO.Path]::GetTempPath()) ("llm-verify-" + [Guid]::NewGuid().ToString('N').Substring(0, 8))
New-Item -ItemType Directory -Path $tmp -Force | Out-Null

try {
    Say "Download : $asset"
    $zip = Join-Path $tmp $asset
    try {
        Invoke-WebRequest -Uri $url -OutFile $zip -UseBasicParsing `
                          -Headers @{ 'User-Agent' = 'llm-verify-installer' }
    } catch {
        Die "Download failed: $url
       $($_.Exception.Message)
       That release may have no $target artefact.
       See https://github.com/$Repo/releases/tag/$version for what is available."
    }
    if (-not (Test-Path $zip) -or (Get-Item $zip).Length -eq 0) {
        Die "The downloaded file was empty: $url"
    }

    # ── checksum ──────────────────────────────────────────────────────────
    $sumsUrl = "https://github.com/$Repo/releases/download/$version/SHA256SUMS"
    $sums = Join-Path $tmp 'SHA256SUMS'
    $verified = $false
    try {
        Invoke-WebRequest -Uri $sumsUrl -OutFile $sums -UseBasicParsing `
                          -Headers @{ 'User-Agent' = 'llm-verify-installer' }
        $line = Get-Content $sums | Where-Object { $_ -match "\s\*?$([regex]::Escape($asset))$" } | Select-Object -First 1
        if ($line) {
            $expected = ($line -split '\s+')[0].ToLower()
            $actual = (Get-FileHash -Path $zip -Algorithm SHA256).Hash.ToLower()
            if ($expected -ne $actual) {
                Die "Checksum mismatch.
       expected: $expected
       actual:   $actual
       The file was corrupted in transit, or came from somewhere untrusted.
       Install aborted."
            }
            Ok "Checksum verified (sha256)"
            $verified = $true
        }
    } catch { }
    if (-not $verified) { Warn "Cannot verify the checksum (file unavailable, or no entry for this asset)" }

    # ── unpack ────────────────────────────────────────────────────────────
    try {
        Expand-Archive -Path $zip -DestinationPath $tmp -Force
    } catch {
        Die "Extraction failed; the archive may be corrupt: $($_.Exception.Message)"
    }
    $src = Join-Path $tmp "$name\$Bin"
    if (-not (Test-Path $src)) { Die "Unexpected archive layout: $name\$Bin not found." }

    # ── install ───────────────────────────────────────────────────────────
    try {
        New-Item -ItemType Directory -Path $BinDir -Force | Out-Null
        Copy-Item -Path $src -Destination (Join-Path $BinDir $Bin) -Force
    } catch {
        Die "Could not write to $BinDir: $($_.Exception.Message)
       Permissions, or the file is currently running.
       Set `$env:LLM_VERIFY_BIN_DIR to another location."
    }

    $exe = Join-Path $BinDir $Bin
    try {
        $ver = & $exe --version 2>$null
    } catch {
        Die "Installed to $exe, but it will not run: $($_.Exception.Message)"
    }
    Ok "Installed $ver → $exe"
}
finally {
    Remove-Item -Path $tmp -Recurse -Force -ErrorAction SilentlyContinue
}

# ── PATH ──────────────────────────────────────────────────────────────────
$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if ($userPath -and ($userPath -split ';' | Where-Object { $_.TrimEnd('\') -ieq $BinDir.TrimEnd('\') })) {
    Write-Host ""
    Ok "$BinDir is already on your PATH — open a new terminal and run llm-verify"
} else {
    try {
        $newPath = if ($userPath) { "$userPath;$BinDir" } else { $BinDir }
        [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
        $env:Path = "$env:Path;$BinDir"
        Write-Host ""
        Ok "Added $BinDir to your user PATH (takes effect in new terminals)"
    } catch {
        Write-Host ""
        Warn "Could not update PATH automatically: $($_.Exception.Message)"
        Write-Host "      Add this directory to your PATH manually:"
        Write-Host "      $BinDir" -ForegroundColor Green
    }
}

Write-Host ""
Write-Host "  Get started:"
Write-Host "      llm-verify --base-url <URL> --api-key <KEY> --model <MODEL>" -ForegroundColor DarkGray
Write-Host "      npx skills add asale-ai/llm-verify   # the skill, for your AI coding tool" -ForegroundColor DarkGray
Write-Host ""
