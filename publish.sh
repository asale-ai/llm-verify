#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# Unattended release: bump version -> commit -> push -> tag -> push tag ->
# publish to crates.io. The tag triggers .github/workflows/release.yml, which
# builds all five targets and publishes the GitHub Release; the crate goes up
# from here, once that build has gone green.
#
#   ./publish.sh "commit message"
#   ./publish.sh -m minor "commit message"
#   ./publish.sh --version 1.2.3 "commit message"
#   ./publish.sh --dry-run "commit message"
#   ./publish.sh --no-crate "commit message"   # skip crates.io
#
# Re-running after a failure resumes: if the tag already exists and points at
# exactly this code, the git half is left alone and the release picks up from
# whichever step went wrong.
#
# No interactive prompts. Credentials are read from .env and never written
# into the repository. crates.io needs CARGO_API_KEY there.

set -euo pipefail

BUMP="patch"
EXPLICIT_VERSION=""
DRY_RUN=0
MESSAGE=""
SKIP_CHECKS=0
PUBLISH_CRATE=1
RESUME=0
ON_CRATES_IO=0

die() { printf '\n\033[31m错误\033[0m: %s\n\n' "$*" >&2; exit 1; }
step() { printf '\n\033[1m▸ %s\033[0m\n' "$*"; }
info() { printf '  %s\n' "$*"; }
ok() { printf '  \033[32m✓\033[0m %s\n' "$*"; }

usage() {
  sed -n '3,20p' "$0" | sed 's/^# \{0,1\}//'
  exit "${1:-0}"
}

while [ $# -gt 0 ]; do
  case "$1" in
    -m|--bump)    BUMP="${2:-}"; shift 2 ;;
    -v|--version) EXPLICIT_VERSION="${2:-}"; shift 2 ;;
    -n|--dry-run) DRY_RUN=1; shift ;;
    --skip-checks) SKIP_CHECKS=1; shift ;;
    --no-crate)   PUBLISH_CRATE=0; shift ;;
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

CRATE=$(grep -m1 '^name *= *"' Cargo.toml | sed 's/.*"\(.*\)".*/\1/')

# ── the tag may already be here ───────────────────────────────────────────
# The write order below is commit -> push -> tag -> push tag -> wait for CI ->
# crates.io, and only the last step depends on anything outside this machine
# staying up. When it fails — expired token, a blip at crates.io, a laptop
# that went to sleep during the CI wait — everything before it has already
# landed, and the only thing missing is one `cargo publish`.
#
# Refusing to run because the tag exists takes that repair away: the version
# is spent (a tag cannot honestly be moved, and `--version` next would leave a
# hole on crates.io), so all that is left is typing the publish command by
# hand — the one step that most deserves to stay scripted.
#
# So an existing tag is a question rather than a verdict: does it point at
# exactly the code we are about to publish? Tree equality plus a clean
# worktree answers it — commit metadata may differ, content may not. If it
# does, this is an interrupted run and we resume. If it does not, the tag
# means something else and the old error still stands.
if git rev-parse -q --verify "refs/tags/$TAG" >/dev/null; then
  [ "$CURRENT" = "$NEXT" ] || die "标签 $TAG 已存在，但 Cargo.toml 里是 ${CURRENT}。
     这两个对不上，说明 $TAG 不是这次要发的东西。请用 --version 指定其它版本号。"
  [ -z "$(git status --porcelain)" ] || die "标签 $TAG 已存在，但工作区还有未提交的改动。
     接着发会把这些改动一起发出去，和 $TAG 指向的内容对不上。
     先提交或清掉它们再重跑，或者用 --version 指定其它版本号。"
  [ "$(git rev-parse "$TAG^{tree}")" = "$(git rev-parse 'HEAD^{tree}')" ] \
    || die "标签 $TAG 已存在，且指向的代码和当前 HEAD 不一致。请用 --version 指定其它版本号。"
  RESUME=1
fi

# ── crates.io preflight ───────────────────────────────────────────────────
# A crates.io release cannot be taken back, only yanked, and by the time we
# would reach it the commit and the tag are already pushed. So everything that
# can be known in advance — missing token, version already taken — is checked
# here, before the first write.
if [ "$PUBLISH_CRATE" = 1 ]; then
  if [ -z "${CARGO_API_KEY:-}" ]; then
    die "缺少 CARGO_API_KEY（放进 .env 即可，token 在 https://crates.io/settings/tokens 生成）。
     只发 GitHub Release 不发 crate 请加 --no-crate。"
  fi
  CODE=$(curl -sS -o /dev/null -w '%{http_code}' \
           -H 'User-Agent: llm-verify publish.sh' \
           "https://crates.io/api/v1/crates/$CRATE/$NEXT" 2>/dev/null || echo 000)
  case "$CODE" in
    # Braces are load-bearing wherever full-width punctuation follows a
    # variable: bash reads the multibyte comma as part of the name and dies
    # under `set -u` with an "unbound variable" for a variable that is set.
    200) info "crates.io 已有 $CRATE ${NEXT}，本次跳过 crate 发布"; PUBLISH_CRATE=0; ON_CRATES_IO=1 ;;
    404) info "crates.io  : 将发布 $CRATE $NEXT" ;;
    # A network failure here is not a reason to abort the whole release; the
    # publish step below will surface the real error if there is one.
    *)   info "crates.io  : 无法查询（HTTP ${CODE}），仍会尝试发布" ;;
  esac
fi

if [ "$RESUME" = 1 ]; then
  # Tag and crate both present with nothing uncommitted: this version is fully
  # out. Say so and stop, rather than spending twenty minutes re-watching a CI
  # run whose result cannot change anything.
  if [ "$ON_CRATES_IO" = 1 ]; then
    step "$TAG 已经发布完成"
    info "标签和 crates.io 上的 $CRATE $NEXT 都在，且就是当前这份代码 —— 没有要做的。"
    info "要发新的一版就改代码后重跑，或用 --version 指定版本号。"
    printf '\n'
    exit 0
  fi
  step "续跑 $TAG"
  info "$TAG 已存在且指向当前这份代码 —— 跳过改版本号和提交，从上次断掉的地方接着发。"
fi

# ── checks ────────────────────────────────────────────────────────────────
if [ "$SKIP_CHECKS" = 0 ]; then
  step "检查"
  if cargo fmt --check >/dev/null 2>&1; then
    ok "cargo fmt"
  elif [ "$RESUME" = 1 ]; then
    # Reformatting on the resume path would change files that $TAG already
    # pins, and there is no commit step left to absorb them.
    die "cargo fmt 有差异，但这次是续跑 ${TAG}，改动没有地方提交。
     先自己 cargo fmt 并处理掉这些改动，或者用 --version 发一个新版本。"
  else
    info "cargo fmt 有差异，正在自动修复…"
    cargo fmt
    ok "cargo fmt（已自动格式化）"
  fi
  cargo clippy --all-targets -- -D warnings >/dev/null 2>&1 \
    && ok "cargo clippy" \
    || die "cargo clippy 有告警。修复后重试，或用 --skip-checks 跳过。"
  cargo test --quiet >/dev/null 2>&1 && ok "cargo test" || die "测试未通过。修复后重试。"
  cargo build --release --quiet && ok "release 构建通过"
  SIZE=$(ls -lh target/release/llm-verify 2>/dev/null | awk '{print $5}')
  [ -n "$SIZE" ] && info "二进制体积: $SIZE"

  # Anything the checks wrote (a re-resolved Cargo.lock, say) would ship in the
  # crate without ever reaching $TAG. Cheap to notice here, invisible later.
  if [ "$RESUME" = 1 ] && [ -n "$(git status --porcelain)" ]; then
    die "检查这一步动了工作区里的文件，但续跑 $TAG 没有提交环节，发出去就会和标签对不上。
     看一眼 git status，处理完再重跑。"
  fi
fi

# ── version bump ──────────────────────────────────────────────────────────
step "更新版本号"
if [ "$RESUME" = 1 ]; then
  info "Cargo.toml 已经是 ${NEXT}，跳过"
elif [ "$DRY_RUN" = 1 ]; then
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
  if [ "$RESUME" = 1 ]; then
    info "$TAG 已存在且指向当前这份代码，不会再提交或改动标签"
    info "会确认 origin/$BRANCH 和 $TAG 都在远端（上次可能正断在推标签那一步）"
  else
    git status --short | sed 's/^/  /'
    info "将提交上述改动并推送到 origin/$BRANCH"
    info "将创建标签 $TAG 并推送"
  fi
  if [ "$PUBLISH_CRATE" = 1 ]; then
    info "CI 构建通过后，将发布 $CRATE $NEXT 到 crates.io"
  else
    # The preflight above already said why — flag, or version already taken.
    info "不发布 crate"
  fi
  step "预演结束，未做任何改动"
  exit 0
fi

if [ "$RESUME" = 0 ]; then
  git add -A
  if git diff --cached --quiet; then
    info "没有需要提交的改动"
  else
    git commit -q -m "$MESSAGE

Release $TAG"
    ok "已提交"
  fi
fi

git push -q origin "$BRANCH" || die "推送到 origin/$BRANCH 失败"
ok "已推送到 origin/$BRANCH"

# On the resume path the tag is already here; re-pushing an identical tag is a
# no-op, and it is the one thing worth redoing blindly — the previous run may
# well have died on exactly this push, leaving the tag local-only.
if [ "$RESUME" = 0 ]; then
  git tag -a "$TAG" -m "Release $TAG

$MESSAGE"
fi
git push -q origin "$TAG" || die "推送标签 $TAG 失败"
ok "已推送标签 $TAG"

# ── watch the release ─────────────────────────────────────────────────────
# The outcome gates crates.io below, so every path has to leave BUILD_OK set.
# A build that fails on one target means `cargo install` fails there too, and
# that is not something a publish can be walked back from.
step "发布工作流"
BUILD_OK=0
BUILD_NOTE=""
if ! command -v gh >/dev/null 2>&1; then
  BUILD_NOTE="未安装 gh CLI，无法确认 CI 结果"
  info "${BUILD_NOTE}。"
  info "查看进度: https://github.com/asale-ai/llm-verify/actions"
else
  # Find the run for *this* tag rather than taking the newest run blindly: a
  # concurrent push would otherwise have us watching someone else's build. The
  # run does not appear instantly, so poll for it.
  info "等待 release 工作流出现…"
  RUN_ID=""
  for _ in $(seq 1 30); do
    RUN_ID=$(gh run list --workflow=release.yml --limit 20 \
               --json databaseId,headBranch,event \
               --jq "[.[] | select(.headBranch == \"$TAG\")] | .[0].databaseId" 2>/dev/null || true)
    [ -n "$RUN_ID" ] && [ "$RUN_ID" != "null" ] && break
    RUN_ID=""
    sleep 4
  done

  if [ -z "$RUN_ID" ]; then
    BUILD_NOTE="未能定位 $TAG 的工作流运行"
    info "${BUILD_NOTE}。"
    info "手动查看： gh run list --workflow=release.yml"
  else
    info "运行 ID: $RUN_ID"

    # Poll the conclusion directly. `gh run watch --exit-status` was flaky here:
    # invoked in the seconds after a run is created it can return non-zero for a
    # run that goes on to succeed, which turned a good release into a false alarm.
    CONCLUSION=""
    for _ in $(seq 1 240); do
      read -r STATUS CONCLUSION <<<"$(gh run view "$RUN_ID" --json status,conclusion \
          --jq '"\(.status) \(.conclusion // "")"' 2>/dev/null || echo "unknown ")"
      [ "$STATUS" = "completed" ] && break
      sleep 5
    done

    case "$CONCLUSION" in
      success)
        BUILD_OK=1
        ok "构建成功"
        gh release view "$TAG" --json assets --jq '.assets[].name' 2>/dev/null | sed 's/^/    /'
        ok "发布完成: $(gh repo view --json url --jq .url)/releases/tag/$TAG"
        ;;
      "")
        BUILD_NOTE="工作流仍在运行（已等待 20 分钟）"
        info "${BUILD_NOTE}。查看： gh run view $RUN_ID"
        ;;
      *)
        die "发布工作流 ${CONCLUSION}。查看日志： gh run view $RUN_ID --log-failed"
        ;;
    esac
  fi
fi

# ── crates.io ─────────────────────────────────────────────────────────────
MANUAL="CARGO_REGISTRY_TOKEN=\$CARGO_API_KEY cargo publish --locked --registry crates-io"
if [ "$PUBLISH_CRATE" = 1 ]; then
  step "发布到 crates.io"
  if [ "$BUILD_OK" = 0 ]; then
    info "跳过：${BUILD_NOTE}。"
    info "CI 通过后手动发布： $MANUAL"
  else
    # --registry crates-io is not redundant: a source-replacement mirror in
    # ~/.cargo/config.toml (rsproxy and friends) makes cargo refuse to publish
    # at all without it. The token travels through the environment rather than
    # --token so it never appears in the process list.
    CARGO_REGISTRY_TOKEN="$CARGO_API_KEY" \
      cargo publish --locked --registry crates-io \
      || die "cargo publish 失败。修好后重试： $MANUAL"
    ok "已发布 $CRATE $NEXT 到 crates.io"
    info "https://crates.io/crates/$CRATE/$NEXT"
  fi
fi

printf '\n'
