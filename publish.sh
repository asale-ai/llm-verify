#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# Unattended release: bump version -> commit -> push -> tag -> push tag.
# The tag triggers .github/workflows/release.yml, which builds all five
# targets and publishes the GitHub Release.
#
#   ./publish.sh "commit message"
#   ./publish.sh -m minor "commit message"
#   ./publish.sh --version 1.2.3 "commit message"
#   ./publish.sh --dry-run "commit message"
#
# No interactive prompts. Credentials are read from .env and never written
# into the repository.

set -euo pipefail

BUMP="patch"
EXPLICIT_VERSION=""
DRY_RUN=0
MESSAGE=""
SKIP_CHECKS=0

die() { printf '\n\033[31m错误\033[0m: %s\n\n' "$*" >&2; exit 1; }
step() { printf '\n\033[1m▸ %s\033[0m\n' "$*"; }
info() { printf '  %s\n' "$*"; }
ok() { printf '  \033[32m✓\033[0m %s\n' "$*"; }

usage() {
  sed -n '3,16p' "$0" | sed 's/^# \{0,1\}//'
  exit "${1:-0}"
}

while [ $# -gt 0 ]; do
  case "$1" in
    -m|--bump)    BUMP="${2:-}"; shift 2 ;;
    -v|--version) EXPLICIT_VERSION="${2:-}"; shift 2 ;;
    -n|--dry-run) DRY_RUN=1; shift ;;
    --skip-checks) SKIP_CHECKS=1; shift ;;
    -h|--help)    usage 0 ;;
    -*)           die "未知参数 $1（用 --help 查看用法）" ;;
    *)            MESSAGE="${MESSAGE:+$MESSAGE }$1"; shift ;;
  esac
done

[ -n "$MESSAGE" ] || die "缺少 commit message。用法： ./publish.sh \"你的提交说明\""
case "$BUMP" in major|minor|patch) ;; *) die "--bump 只能是 major / minor / patch" ;; esac

cd "$(dirname "$0")"

command -v git >/dev/null || die "缺少 git"
command -v cargo >/dev/null || die "缺少 cargo"
git rev-parse --git-dir >/dev/null 2>&1 || die "当前目录不是 git 仓库"

# .env carries publishing credentials. Source it without exporting the values
# into anything that gets logged.
if [ -f .env ]; then
  set -a
  # shellcheck disable=SC1091
  . ./.env
  set +a
fi

# ── current version ───────────────────────────────────────────────────────
CURRENT=$(grep -m1 '^version *= *"' Cargo.toml | sed 's/.*"\(.*\)".*/\1/')
[ -n "$CURRENT" ] || die "无法从 Cargo.toml 读取当前版本号"

if [ -n "$EXPLICIT_VERSION" ]; then
  NEXT="${EXPLICIT_VERSION#v}"
else
  IFS=. read -r MA MI PA <<<"$CURRENT"
  case "$BUMP" in
    major) NEXT="$((MA + 1)).0.0" ;;
    minor) NEXT="${MA}.$((MI + 1)).0" ;;
    patch) NEXT="${MA}.${MI}.$((PA + 1))" ;;
  esac
fi
echo "$NEXT" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$' || die "版本号格式不合法: $NEXT"

TAG="v$NEXT"
BRANCH=$(git rev-parse --abbrev-ref HEAD)

step "发布 $CURRENT → $NEXT ($BUMP)"
info "分支    : $BRANCH"
info "标签    : $TAG"
info "提交说明: $MESSAGE"
[ "$DRY_RUN" = 1 ] && info "模式    : 预演（不做任何写操作）"

git rev-parse "$TAG" >/dev/null 2>&1 && die "标签 $TAG 已存在。请用 --version 指定其它版本号。"

# ── checks ────────────────────────────────────────────────────────────────
if [ "$SKIP_CHECKS" = 0 ]; then
  step "检查"
  cargo fmt --check >/dev/null 2>&1 && ok "cargo fmt" || {
    info "cargo fmt 有差异，正在自动修复…"
    cargo fmt
    ok "cargo fmt（已自动格式化）"
  }
  cargo clippy --all-targets -- -D warnings >/dev/null 2>&1 \
    && ok "cargo clippy" \
    || die "cargo clippy 有告警。修复后重试，或用 --skip-checks 跳过。"
  cargo test --quiet >/dev/null 2>&1 && ok "cargo test" || die "测试未通过。修复后重试。"
  cargo build --release --quiet && ok "release 构建通过"
  SIZE=$(ls -lh target/release/llm-verify 2>/dev/null | awk '{print $5}')
  [ -n "$SIZE" ] && info "二进制体积: $SIZE"
fi

# ── version bump ──────────────────────────────────────────────────────────
step "更新版本号"
if [ "$DRY_RUN" = 1 ]; then
  info "将把 Cargo.toml 的 version 改为 $NEXT"
else
  # Only the [package] version — the first `version =` in the file. Anchoring
  # to the exact current value avoids touching a dependency of the same name.
  if [ "$(uname -s)" = "Darwin" ]; then
    sed -i '' "1,/^version *= *\"$CURRENT\"/s/^version *= *\"$CURRENT\"/version = \"$NEXT\"/" Cargo.toml
  else
    sed -i "1,/^version *= *\"$CURRENT\"/s/^version *= *\"$CURRENT\"/version = \"$NEXT\"/" Cargo.toml
  fi
  grep -q "^version = \"$NEXT\"" Cargo.toml || die "Cargo.toml 版本号更新失败"
  # Keep Cargo.lock in step so the commit is self-consistent.
  cargo metadata --format-version 1 >/dev/null 2>&1 || true
  ok "Cargo.toml → $NEXT"
fi

# ── commit and push ───────────────────────────────────────────────────────
step "提交并推送"
if [ "$DRY_RUN" = 1 ]; then
  git status --short | sed 's/^/  /'
  info "将提交上述改动并推送到 origin/$BRANCH"
  info "将创建标签 $TAG 并推送"
  step "预演结束，未做任何改动"
  exit 0
fi

git add -A
if git diff --cached --quiet; then
  info "没有需要提交的改动"
else
  git commit -q -m "$MESSAGE

Release $TAG"
  ok "已提交"
fi

git push -q origin "$BRANCH" || die "推送到 origin/$BRANCH 失败"
ok "已推送到 origin/$BRANCH"

git tag -a "$TAG" -m "Release $TAG

$MESSAGE"
git push -q origin "$TAG" || die "推送标签 $TAG 失败"
ok "已推送标签 $TAG"

# ── watch the release ─────────────────────────────────────────────────────
step "发布工作流"
if command -v gh >/dev/null 2>&1; then
  info "已触发 release 工作流，正在等待…"
  sleep 8
  RUN_ID=$(gh run list --workflow=release.yml --limit 1 --json databaseId \
             --jq '.[0].databaseId' 2>/dev/null || true)
  if [ -n "$RUN_ID" ]; then
    info "运行 ID: $RUN_ID"
    if gh run watch "$RUN_ID" --exit-status 2>/dev/null; then
      ok "构建成功"
      gh release view "$TAG" --json assets --jq '.assets[].name' 2>/dev/null | sed 's/^/    /'
      ok "发布完成: $(gh repo view --json url --jq .url)/releases/tag/$TAG"
    else
      die "发布工作流失败。查看日志： gh run view $RUN_ID --log-failed"
    fi
  else
    info "未能定位工作流运行，请手动查看： gh run list --workflow=release.yml"
  fi
else
  info "未安装 gh CLI，无法自动跟踪。"
  info "查看进度: https://github.com/asale-ai/llm-verify/actions"
fi

printf '\n'
