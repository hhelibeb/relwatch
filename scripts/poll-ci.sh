#!/usr/bin/env bash
# CI 轮询脚本 — 前台阻塞，等待 main 分支全部 PR 门禁 workflow 和可选 tag Release 通过。
# 已完成且失败的 workflow 是终态，立即退出（等待不会改变结果）。
# Usage: ./scripts/poll-ci.sh [tag]
set -euo pipefail

TAG="${1:-}"
POLL=30
MAX=30 # 轮次上限（配合 POLL 约 15 分钟）

# 按 commit 过滤：只按 branch 取「最新一次 run」时，本次新 run 还没进 API 就会读到上一个
# commit 的绿灯，等于拿上一次的结果给本次发布放行。SHA 可用 CI_SHA 覆盖。
SHA="${CI_SHA:-$(git rev-parse HEAD)}"

# workflow 名单从仓库读，不手工维护副本：漏加一个 workflow = 它红了也不报错，发布就会带着
# 红的 CI 照发。只取带 pull_request 触发的（必然为推到 main 的每个 commit 产出 run）；
# 只在 tag/cron 上跑的 workflow 永远等不到 run，会把轮询一路拖到超时。
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
NAMES=()
while IFS= read -r name; do
  [ -n "$name" ] || continue
  NAMES+=("$name")
done < <(
  for f in "$ROOT"/.github/workflows/*.yml; do
    if [ "$(basename "$f")" != "release.yml" ] && grep -q '^[[:space:]]*pull_request:' "$f"; then
      sed -n 's/^name:[[:space:]]*//p' "$f"
    fi
  done | sort
)
if [ "${#NAMES[@]}" -eq 0 ]; then
  echo "❌ 未能从 .github/workflows/ 解析出 workflow 名单" >&2
  exit 1
fi

echo "===== CI 轮询开始 ====="
echo "Branch: main${TAG:+ | Tag: $TAG}"
echo "Commit: $SHA"
echo "Workflows: ${NAMES[*]}"
echo "Poll: ${POLL}s | 上限 $((MAX * POLL / 60)) 分钟"
echo "======================"

# 返回码：0 = 全部成功；1 = 已有终态的失败（等待无意义）；2 = 仍有未结束的
check_workflows() {
  local branch="$1" sha="$2"
  shift 2
  local all_done=true any_failed=false name row status conclusion

  for name in "$@"; do
    row=$(gh run list --branch "$branch" --commit "$sha" --limit 15 \
      --json name,status,conclusion \
      --jq "[.[] | select(.name==\"$name\")][0] | \"\(.status)//\(.conclusion)\"" 2>/dev/null || true)
    status="${row%%//*}"
    conclusion="${row##*//}"
    case "$status" in
      completed)
        if [ "$conclusion" = "success" ]; then
          printf "  \xe2\x9c\x85 %-15s  %s\n" "$name" "$conclusion"
        elif [ "$conclusion" = "skipped" ]; then
          # skipped = 本次根本没跑（被条件/过滤器挡掉）。门禁没执行不等于通过：
          # 当绿灯放行会把「没跑过的 CI」带进发布流程，必须人工确认。
          printf "  \xe2\x9a\xa0\xef\xb8\x8f %-15s  %s（未执行，视为未通过）\n" "$name" "$conclusion"
          any_failed=true
        else
          printf "  \xe2\x9d\x8c %-15s  %s\n" "$name" "$conclusion"
          any_failed=true
        fi
        ;;
      *)
        printf "  \xe2\x8f\xb3 %-15s  %s\n" "$name" "${status:-N/A}"
        all_done=false
        ;;
    esac
  done
  if $any_failed; then return 1; fi
  if $all_done; then return 0; else return 2; fi
}

# 返回码同 check_workflows；tag run 尚未创建也算 2（它只会在推送后才出现）
check_tag_release() {
  local tag="$1"
  local row status conclusion
  row=$(gh run list --limit 15 --json name,status,conclusion,headBranch \
    --jq "[.[] | select(.name==\"Release\" and .headBranch==\"$tag\")][0] | \"\(.status)//\(.conclusion)\"" 2>/dev/null || true)
  status="${row%%//*}"
  conclusion="${row##*//}"
  if [ -z "$status" ]; then
    printf "  \xe2\x8f\xb3 %-15s  %s\n" "Release" "N/A"
    return 2
  fi
  case "$status" in
    completed)
      if [ "$conclusion" = "success" ]; then
        printf "  \xe2\x9c\x85 %-15s  %s\n" "Release" "$conclusion"
        return 0
      else
        printf "  \xe2\x9d\x8c %-15s  %s\n" "Release" "$conclusion"
        return 1
      fi
      ;;
    *)
      printf "  \xe2\x8f\xb3 %-15s  %s\n" "Release" "$status"
      return 2
      ;;
  esac
}

for i in $(seq 1 "$MAX"); do
  echo "[$i/$MAX]  $(date '+%H:%M:%S')"

  main_rc=2
  if check_workflows main "$SHA" "${NAMES[@]}"; then main_rc=0; else main_rc=$?; fi

  tag_rc=0
  if [ -n "$TAG" ]; then
    if check_tag_release "$TAG"; then tag_rc=0; else tag_rc=$?; fi
  fi

  echo ""
  if [ "$main_rc" = "0" ] && [ "$tag_rc" = "0" ]; then
    echo "===== 🎉 全部 CI 通过！======"
    exit 0
  fi
  if [ "$main_rc" = "1" ] || [ "$tag_rc" = "1" ]; then
    echo "===== ❌ CI 失败：上面标 ❌ 的 workflow 未通过，继续等待不会改变结果 ====="
    exit 1
  fi
  if [ "$i" -lt "$MAX" ]; then
    sleep "$POLL"
  fi
done

echo "===== ⏱ 超时：仍有 workflow 未结束（上限 $((MAX * POLL / 60)) 分钟）====="
exit 1
