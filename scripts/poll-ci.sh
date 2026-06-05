#!/usr/bin/env bash
# CI 轮询脚本 — 前台阻塞，等待 main 分支 CI/Lint/Secret Scan 和可选 tag Release 全部通过
# Usage: ./scripts/poll-ci.sh [tag]
set -euo pipefail

TAG="${1:-}"
POLL=30
NAMES=("CI" "Lint" "Secret Scan")

echo "===== CI 轮询开始 ====="
echo "Branch: main${TAG:+ | Tag: $TAG}"
echo "Poll: ${POLL}s | 上限 15 分钟"
echo "======================"

check_workflows() {
  local branch="$1"
  shift
  local all_done=true any_failed=false name row status conclusion

  for name in "$@"; do
    row=$(gh run list --branch "$branch" --limit 15 \
      --json name,status,conclusion \
      --jq "[.[] | select(.name==\"$name\")][0] | \"\(.status)//\(.conclusion)\"" 2>/dev/null || true)
    status="${row%%//*}"
    conclusion="${row##*//}"
    case "$status" in
      completed)
        if [ "$conclusion" = "success" ] || [ "$conclusion" = "skipped" ]; then
          printf "  \xe2\x9c\x85 %-15s  %s\n" "$name" "$conclusion"
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

for i in $(seq 1 30); do
  echo "[$i/60]  $(date '+%H:%M:%S')"

  main_ok=false
  check_workflows main "${NAMES[@]}" && main_ok=true

  tag_ok=true
  if [ -n "$TAG" ]; then
    check_tag_release "$TAG" || tag_ok=false
  fi

  echo ""
  if $main_ok && $tag_ok; then
    echo "===== 🎉 全部 CI 通过！======"
    exit 0
  fi
  [ "$i" -lt 60 ] && sleep "$POLL"
done

echo "===== ❌ 超时：CI 未在 15 分钟内完成 ====="
exit 1
