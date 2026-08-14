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
die()  { printf '%b\n' "\n  ${R}Install failed${Z}: $*\n" >&2; exit 1; }

need() {
  command -v "$1" >/dev/null 2>&1 || die "required command '$1' not found; install it and try again."
}

# ── platform detection ────────────────────────────────────────────────────
detect_target() {
  os=$(uname -s)
  arch=$(uname -m)
  case "$os" in
    Darwin) os_part="apple-darwin" ;;
    Linux)  os_part="unknown-linux-gnu" ;;
    *) die "unsupported operating system: ${os}. On Windows use install.ps1 instead." ;;
  esac
  case "$arch" in
    x86_64|amd64)  arch_part="x86_64" ;;
    arm64|aarch64) arch_part="aarch64" ;;
    *) die "unsupported CPU architecture: ${arch}. Builds exist for x86_64 and aarch64." ;;
  esac
  if [ "$os_part" = "unknown-linux-gnu" ] && [ "$arch_part" = "aarch64" ]; then
    : # built and published
  fi
  echo "${arch_part}-${os_part}"
}

# ── main ──────────────────────────────────────────────────────────────────
printf '\n  %bllm-verify%b installer\n\n' "$D" "$Z"

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
  die "needs either curl or wget to download."
fi

TARGET=$(detect_target)
say "Platform : $TARGET"

VERSION="${LLM_VERIFY_VERSION:-}"
if [ -z "$VERSION" ]; then
  say "Resolving the latest version…"
  VERSION=$($DL "https://api.github.com/repos/$REPO/releases/latest" 2>/dev/null \
    | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' \
    | head -n 1) || true
  [ -n "$VERSION" ] || die "could not resolve the latest version. The network may be down or the GitHub API rate-limited; you can pin one with LLM_VERIFY_VERSION=v0.2.0."
fi
NUM="${VERSION#v}"
say "Version  : $VERSION"

NAME="llm-verify-${NUM}-${TARGET}"
ASSET="${NAME}.tar.gz"
URL="https://github.com/$REPO/releases/download/$VERSION/$ASSET"

TMP=$(mktemp -d 2>/dev/null || mktemp -d -t llm-verify)
# shellcheck disable=SC2064
trap "rm -rf '$TMP'" EXIT INT TERM

say "Download : $ASSET"
if ! $DL_OUT "$TMP/$ASSET" "$URL" 2>/dev/null; then
  die "download failed: $URL
       That release may have no $TARGET artefact, or the network is unreachable.
       See https://github.com/$REPO/releases/tag/$VERSION for what is available."
fi
[ -s "$TMP/$ASSET" ] || die "the downloaded file was empty: $URL"

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
      ok "checksum verified (sha256)"
    else
      die "checksum mismatch.
       expected: $expected
       actual:   $actual
       The file was corrupted in transit, or came from somewhere untrusted.
       Install aborted."
    fi
  else
    warn "no entry for $ASSET in SHA256SUMS; skipping verification"
  fi
else
  warn "cannot verify the checksum (no sha256 tool, or the checksum file is unavailable)"
fi

# ── unpack and install ────────────────────────────────────────────────────
tar xzf "$TMP/$ASSET" -C "$TMP" || die "extraction failed; the archive may be corrupt."
SRC="$TMP/$NAME/$BIN"
[ -f "$SRC" ] || die "unexpected archive layout: $NAME/${BIN} not found."

mkdir -p "$BIN_DIR" || die "could not create ${BIN_DIR} (permissions? set LLM_VERIFY_BIN_DIR to another location)"
if ! install -m 755 "$SRC" "$BIN_DIR/$BIN" 2>/dev/null; then
  cp "$SRC" "$BIN_DIR/$BIN" || die "could not write $BIN_DIR/${BIN} (permissions? set LLM_VERIFY_BIN_DIR to another location)"
  chmod 755 "$BIN_DIR/$BIN"
fi

# Verify it actually runs on this machine before declaring success.
if ! "$BIN_DIR/$BIN" --version >/dev/null 2>&1; then
  die "installed to $BIN_DIR/${BIN}, but it will not run.
       The architecture may not match, or the system libc may be too old."
fi
ok "installed $("$BIN_DIR/$BIN" --version) → $BIN_DIR/$BIN"

# ── PATH ──────────────────────────────────────────────────────────────────
case ":${PATH}:" in
  *":$BIN_DIR:"*)
    printf '\n'
    ok "$BIN_DIR is on your PATH — just run ${G}llm-verify${Z}"
    ;;
  *)
    case "${SHELL:-}" in
      */zsh)  RC="~/.zshrc" ;;
      */bash) RC="~/.bashrc" ;;
      */fish) RC="~/.config/fish/config.fish" ;;
      *)      RC="your shell profile" ;;
    esac
    printf '\n'
    warn "$BIN_DIR is not on your PATH. Add this line to ${RC}:"
    printf '\n      %bexport PATH="%s:$PATH"%b\n' "$G" "$BIN_DIR" "$Z"
    printf '\n  Then open a new terminal, or use the full path for now:\n'
    printf '      %b%s/%s --help%b\n' "$D" "$BIN_DIR" "$BIN" "$Z"
    ;;
esac

printf '\n  Get started:\n'
printf '      %bllm-verify --base-url <URL> --api-key <KEY> --model <MODEL>%b\n' "$D" "$Z"
printf '      %bnpx skills add asale-ai/llm-verify%b   # the skill, for your AI coding tool\n\n' "$D" "$Z"
