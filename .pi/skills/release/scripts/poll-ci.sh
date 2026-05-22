#!/usr/bin/env bash
# CI 轮询脚本 — 等待 tag push 触发的所有 workflow 全部 success
# 用法: ./scripts/poll-ci.sh
set -euo pipefail

TOKEN=$(git credential fill <<<"protocol=https\nhost=github.com\n" 2>/dev/null | grep password | cut -d= -f2 | head -1)
OWNER="hhelibeb"
REPO="relwatch"

echo "⏳ 等待 CI 全部通过..."

while true; do
  RESP=$(curl -sf -H "Authorization: token $TOKEN" \
    "https://api.github.com/repos/$OWNER/$REPO/actions/runs?branch=main&event=push&per_page=3&status=completed")

  # 取最近 3 个 completed runs 的状态
  CONCLUSIONS=$(echo "$RESP" | grep -o '"conclusion":"[^"]*"' | cut -d'"' -f4)

  ALL_SUCCESS=true
  COUNT=0
  for c in $CONCLUSIONS; do
    COUNT=$((COUNT + 1))
    if [ "$c" != "success" ]; then
      ALL_SUCCESS=false
    fi
  done

  # 至少 2 个 run 全部 success 就算通过
  if [ "$ALL_SUCCESS" = true ] && [ "$COUNT" -ge 2 ]; then
    echo "✅ 所有 workflow 已通过"
    exit 0
  fi

  echo "⏳ 尚有 workflow 运行中或未就绪..."
  sleep 30
done
