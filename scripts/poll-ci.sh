#!/usr/bin/env bash
# CI 轮询脚本 — 前台阻塞，等待 main 分支全部 workflow（Release 除外）和可选 tag Release 全部通过
# Usage: ./scripts/poll-ci.sh [tag]
set -euo pipefail

TAG="${1:-}"
POLL=30
MAX=30 # 轮次上限（配合 POLL 约 15 分钟）

# 按 commit 过滤：只按 branch 取「最新一次 run」时，本次新 run 还没进 API 就会读到上一个
# commit 的绿灯，等于拿上一次的结果给本次发布放行。SHA 可用 CI_SHA 覆盖（如发布脚本已
# 记录被推送的 commit，或本地 HEAD 不是待验证的那个）。
SHA="${CI_SHA:-$(git rev-parse HEAD)}"

# workflow 名单从仓库读，不再手工维护副本：漏加一个 workflow = 它红了也不报错，
# 发布就会带着红的 CI 照发。release.yml 只在 tag 上跑，由 check_tag_release 单独负责。
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
NAMES=()
while IFS= read -r name; do
  [ -n "$name" ] && NAMES+=("$name")
done < <(
  grep -h --exclude=release.yml '^name:' "$ROOT"/.github/workflows/*.yml |
    sed 's/^name:[[:space:]]*//' | sort
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
  if $all_done && ! $any_failed; then return 0; else return 1; fi
}

check_tag_release() {
  local tag="$1"
  local row status conclusion
  row=$(gh run list --limit 15 --json name,status,conclusion,headBranch \
    --jq "[.[] | select(.name==\"Release\" and .headBranch==\"$tag\")][0] | \"\(.status)//\(.conclusion)\"" 2>/dev/null || true)
  status="${row%%//*}"
  conclusion="${row##*//}"
  if [ -z "$status" ]; then
    printf "  \xe2\x8f\xb3 %-15s  %s\n" "Release" "N/A"
    return 1
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
      return 1
      ;;
  esac
}

for i in $(seq 1 "$MAX"); do
  echo "[$i/$MAX]  $(date '+%H:%M:%S')"

  main_ok=false
  check_workflows main "$SHA" "${NAMES[@]}" && main_ok=true

  tag_ok=true
  if [ -n "$TAG" ]; then
    check_tag_release "$TAG" || tag_ok=false
  fi

  echo ""
  if $main_ok && $tag_ok; then
    echo "===== 🎉 全部 CI 通过！======"
    exit 0
  fi
  if [ "$i" -lt "$MAX" ]; then
    sleep "$POLL"
  fi
done

echo "===== ❌ 超时：CI 未在 $((MAX * POLL / 60)) 分钟内完成 ====="
exit 1
