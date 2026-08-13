#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# llm-verify installer for macOS and Linux.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/asale-ai/llm-verify/main/install.sh | sh
#
# Environment:
#   LLM_VERIFY_VERSION   install a specific tag (default: latest)
#   LLM_VERIFY_BIN_DIR   install location (default: ~/.local/bin)

set -eu

REPO="asale-ai/llm-verify"
BIN="llm-verify"
BIN_DIR="${LLM_VERIFY_BIN_DIR:-$HOME/.local/bin}"

# ── output ────────────────────────────────────────────────────────────────
if [ -t 1 ] && [ -z "${NO_COLOR:-}" ]; then
  R='\033[31m'; G='\033[32m'; Y='\033[33m'; D='\033[2m'; Z='\033[0m'
else
  R=''; G=''; Y=''; D=''; Z=''
fi
say()  { printf '%b\n' "  $*"; }
ok()   { printf '%b\n' "  ${G}✓${Z} $*"; }
warn() { printf '%b\n' "  ${Y}!${Z} $*"; }
die()  { printf '%b\n' "\n  ${R}安装失败${Z}: $*\n" >&2; exit 1; }

need() {
  command -v "$1" >/dev/null 2>&1 || die "缺少必需的命令 '$1'，请先安装后重试。"
}

# ── platform detection ────────────────────────────────────────────────────
detect_target() {
  os=$(uname -s)
  arch=$(uname -m)
  case "$os" in
    Darwin) os_part="apple-darwin" ;;
    Linux)  os_part="unknown-linux-gnu" ;;
    *) die "不支持的操作系统: $os。Windows 请改用 install.ps1。" ;;
  esac
  case "$arch" in
    x86_64|amd64)  arch_part="x86_64" ;;
    arm64|aarch64) arch_part="aarch64" ;;
    *) die "不支持的 CPU 架构: $arch。可用的架构为 x86_64 与 aarch64。" ;;
  esac
  if [ "$os_part" = "unknown-linux-gnu" ] && [ "$arch_part" = "aarch64" ]; then
    : # built and published
  fi
  echo "${arch_part}-${os_part}"
}

# ── main ──────────────────────────────────────────────────────────────────
printf '\n  %bllm-verify%b 安装程序\n\n' "$D" "$Z"

need uname
need mkdir
need tar
if command -v curl >/dev/null 2>&1; then
  DL="curl -fsSL"
  DL_OUT="curl -fsSL -o"
elif command -v wget >/dev/null 2>&1; then
  DL="wget -qO-"
  DL_OUT="wget -qO"
else
  die "需要 curl 或 wget 之一来下载文件。"
fi

TARGET=$(detect_target)
say "平台   : $TARGET"

VERSION="${LLM_VERIFY_VERSION:-}"
if [ -z "$VERSION" ]; then
  say "查询最新版本…"
  VERSION=$($DL "https://api.github.com/repos/$REPO/releases/latest" 2>/dev/null \
    | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' \
    | head -n 1) || true
  [ -n "$VERSION" ] || die "无法获取最新版本号。可能是网络问题或 GitHub API 限流；也可以用 LLM_VERIFY_VERSION=v0.1.0 指定版本。"
fi
NUM="${VERSION#v}"
say "版本   : $VERSION"

NAME="llm-verify-${NUM}-${TARGET}"
ASSET="${NAME}.tar.gz"
URL="https://github.com/$REPO/releases/download/$VERSION/$ASSET"

TMP=$(mktemp -d 2>/dev/null || mktemp -d -t llm-verify)
# shellcheck disable=SC2064
trap "rm -rf '$TMP'" EXIT INT TERM

say "下载   : $ASSET"
if ! $DL_OUT "$TMP/$ASSET" "$URL" 2>/dev/null; then
  die "下载失败: $URL
       该版本可能没有 $TARGET 的产物，或网络不可达。
       可用产物见 https://github.com/$REPO/releases/tag/$VERSION"
fi
[ -s "$TMP/$ASSET" ] || die "下载到的文件是空的: $URL"

# ── checksum ──────────────────────────────────────────────────────────────
if command -v sha256sum >/dev/null 2>&1; then
  SHA_CMD="sha256sum"
elif command -v shasum >/dev/null 2>&1; then
  SHA_CMD="shasum -a 256"
else
  SHA_CMD=""
fi

if [ -n "$SHA_CMD" ] && $DL_OUT "$TMP/SHA256SUMS" \
     "https://github.com/$REPO/releases/download/$VERSION/SHA256SUMS" 2>/dev/null; then
  expected=$(grep " $ASSET\$" "$TMP/SHA256SUMS" 2>/dev/null | awk '{print $1}' | head -n 1)
  if [ -n "$expected" ]; then
    actual=$(cd "$TMP" && $SHA_CMD "$ASSET" | awk '{print $1}')
    if [ "$expected" = "$actual" ]; then
      ok "校验通过 (sha256)"
    else
      die "校验和不匹配。
       期望: $expected
       实际: $actual
       文件可能在传输中损坏，或来源不可信。已中止安装。"
    fi
  else
    warn "SHA256SUMS 中没有 $ASSET 的条目，跳过校验"
  fi
else
  warn "无法校验和（缺少 sha256 工具或校验文件不可用）"
fi

# ── unpack and install ────────────────────────────────────────────────────
tar xzf "$TMP/$ASSET" -C "$TMP" || die "解压失败，压缩包可能已损坏。"
SRC="$TMP/$NAME/$BIN"
[ -f "$SRC" ] || die "压缩包结构不符合预期，未找到 $NAME/$BIN。"

mkdir -p "$BIN_DIR" || die "无法创建目录 $BIN_DIR（权限不足？可用 LLM_VERIFY_BIN_DIR 换一个位置）"
if ! install -m 755 "$SRC" "$BIN_DIR/$BIN" 2>/dev/null; then
  cp "$SRC" "$BIN_DIR/$BIN" || die "无法写入 $BIN_DIR/$BIN（权限不足？可用 LLM_VERIFY_BIN_DIR 换一个位置）"
  chmod 755 "$BIN_DIR/$BIN"
fi

# Verify it actually runs on this machine before declaring success.
if ! "$BIN_DIR/$BIN" --version >/dev/null 2>&1; then
  die "已安装到 $BIN_DIR/$BIN，但无法执行。
       可能是架构不匹配，或系统缺少所需的 libc 版本。"
fi
ok "已安装 $("$BIN_DIR/$BIN" --version) → $BIN_DIR/$BIN"

# ── PATH ──────────────────────────────────────────────────────────────────
case ":${PATH}:" in
  *":$BIN_DIR:"*)
    printf '\n'
    ok "$BIN_DIR 已在 PATH 中，直接运行： ${G}llm-verify${Z}"
    ;;
  *)
    case "${SHELL:-}" in
      */zsh)  RC="~/.zshrc" ;;
      */bash) RC="~/.bashrc" ;;
      */fish) RC="~/.config/fish/config.fish" ;;
      *)      RC="你的 shell 配置文件" ;;
    esac
    printf '\n'
    warn "$BIN_DIR 不在 PATH 中。把下面这行加进 $RC："
    printf '\n      %bexport PATH="%s:$PATH"%b\n' "$G" "$BIN_DIR" "$Z"
    printf '\n  然后重开终端，或先用完整路径运行：\n'
    printf '      %b%s/%s --help%b\n' "$D" "$BIN_DIR" "$BIN" "$Z"
    ;;
esac

printf '\n  开始使用：\n'
printf '      %bllm-verify --base-url <URL> --api-key <KEY> --model <MODEL>%b\n' "$D" "$Z"
printf '      %bllm-verify install-skill%b   # 装进 Claude Code / Codex / OpenCode / Gemini CLI\n\n' "$D" "$Z"
