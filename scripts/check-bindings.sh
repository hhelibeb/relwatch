#!/usr/bin/env bash
# 检查 tauri-specta 生成物 src/bindings.ts 与 Rust 代码同步
# Usage: ./scripts/check-bindings.sh
# 退出码 0 = 同步；非 0 = 过期（Rust 侧命令/类型/事件变更后未重新生成）
set -euo pipefail

cd "$(dirname "$0")/.."

# 1. 重新生成 bindings.ts（debug 构建的导出测试；Rust 侧改过命令/类型/事件时产物会变化）
(cd src-tauri && cargo test export_typescript_bindings --quiet)

# 2. 与已提交版本比对：有差异说明上次提交未带最新生成物
if git diff --exit-code -- src/bindings.ts >/dev/null; then
  echo "✓ bindings.ts 与 Rust 代码同步"
else
  echo "✗ bindings.ts 已过期：已自动重新生成，请 git add src/bindings.ts 后重新检查" >&2
  exit 1
fi
