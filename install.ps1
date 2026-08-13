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
    Write-Host "  安装失败: " -ForegroundColor Red -NoNewline
    Write-Host $m
    Write-Host ""
    exit 1
}

Write-Host ""
Write-Host "  llm-verify 安装程序" -ForegroundColor DarkGray
Write-Host ""

# ── platform ──────────────────────────────────────────────────────────────
$arch = $env:PROCESSOR_ARCHITECTURE
switch ($arch) {
    'AMD64' { $target = 'x86_64-pc-windows-msvc' }
    'ARM64' {
        Die "暂无 Windows ARM64 的预编译产物。
       可以用 x64 版本经模拟运行，或从源码构建：
       cargo install --git https://github.com/$Repo"
    }
    default { Die "不支持的 CPU 架构: $arch。目前仅提供 x86_64。" }
}
Say "平台   : $target"

# TLS 1.2 is not the default on older Windows PowerShell hosts.
try {
    [Net.ServicePointManager]::SecurityProtocol =
        [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
} catch { }

# ── version ───────────────────────────────────────────────────────────────
$version = $env:LLM_VERIFY_VERSION
if (-not $version) {
    Say "查询最新版本…"
    try {
        $rel = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest" `
                                 -Headers @{ 'User-Agent' = 'llm-verify-installer' }
        $version = $rel.tag_name
    } catch {
        Die "无法获取最新版本号：$($_.Exception.Message)
       可能是网络问题或 GitHub API 限流。
       也可以指定版本： `$env:LLM_VERIFY_VERSION='v0.1.0'; irm ... | iex"
    }
}
if (-not $version) { Die "GitHub 未返回可用的版本号。" }
$num = $version.TrimStart('v')
Say "版本   : $version"

# ── download ──────────────────────────────────────────────────────────────
$name  = "llm-verify-$num-$target"
$asset = "$name.zip"
$url   = "https://github.com/$Repo/releases/download/$version/$asset"

$tmp = Join-Path ([IO.Path]::GetTempPath()) ("llm-verify-" + [Guid]::NewGuid().ToString('N').Substring(0, 8))
New-Item -ItemType Directory -Path $tmp -Force | Out-Null

try {
    Say "下载   : $asset"
    $zip = Join-Path $tmp $asset
    try {
        Invoke-WebRequest -Uri $url -OutFile $zip -UseBasicParsing `
                          -Headers @{ 'User-Agent' = 'llm-verify-installer' }
    } catch {
        Die "下载失败: $url
       $($_.Exception.Message)
       该版本可能没有 $target 的产物。
       可用产物见 https://github.com/$Repo/releases/tag/$version"
    }
    if (-not (Test-Path $zip) -or (Get-Item $zip).Length -eq 0) {
        Die "下载到的文件是空的: $url"
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
                Die "校验和不匹配。
       期望: $expected
       实际: $actual
       文件可能在传输中损坏，或来源不可信。已中止安装。"
            }
            Ok "校验通过 (sha256)"
            $verified = $true
        }
    } catch { }
    if (-not $verified) { Warn "无法校验和（校验文件不可用或缺少条目）" }

    # ── unpack ────────────────────────────────────────────────────────────
    try {
        Expand-Archive -Path $zip -DestinationPath $tmp -Force
    } catch {
        Die "解压失败，压缩包可能已损坏：$($_.Exception.Message)"
    }
    $src = Join-Path $tmp "$name\$Bin"
    if (-not (Test-Path $src)) { Die "压缩包结构不符合预期，未找到 $name\$Bin。" }

    # ── install ───────────────────────────────────────────────────────────
    try {
        New-Item -ItemType Directory -Path $BinDir -Force | Out-Null
        Copy-Item -Path $src -Destination (Join-Path $BinDir $Bin) -Force
    } catch {
        Die "无法写入 $BinDir：$($_.Exception.Message)
       可能是权限不足或该文件正在运行。
       可用 `$env:LLM_VERIFY_BIN_DIR 指定其它位置。"
    }

    $exe = Join-Path $BinDir $Bin
    try {
        $ver = & $exe --version 2>$null
    } catch {
        Die "已安装到 $exe，但无法执行：$($_.Exception.Message)"
    }
    Ok "已安装 $ver → $exe"
}
finally {
    Remove-Item -Path $tmp -Recurse -Force -ErrorAction SilentlyContinue
}

# ── PATH ──────────────────────────────────────────────────────────────────
$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if ($userPath -and ($userPath -split ';' | Where-Object { $_.TrimEnd('\') -ieq $BinDir.TrimEnd('\') })) {
    Write-Host ""
    Ok "$BinDir 已在 PATH 中，重开终端后直接运行： llm-verify"
} else {
    try {
        $newPath = if ($userPath) { "$userPath;$BinDir" } else { $BinDir }
        [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
        $env:Path = "$env:Path;$BinDir"
        Write-Host ""
        Ok "已把 $BinDir 加入用户 PATH（新开的终端生效）"
    } catch {
        Write-Host ""
        Warn "无法自动修改 PATH：$($_.Exception.Message)"
        Write-Host "      请手动把下面的目录加入 PATH："
        Write-Host "      $BinDir" -ForegroundColor Green
    }
}

Write-Host ""
Write-Host "  开始使用："
Write-Host "      llm-verify --base-url <URL> --api-key <KEY> --model <MODEL>" -ForegroundColor DarkGray
Write-Host "      llm-verify install-skill   # 装进 Claude Code / Codex / OpenCode / Gemini CLI" -ForegroundColor DarkGray
Write-Host ""
