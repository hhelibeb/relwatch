#!/usr/bin/env bash
# 检查 tauri-specta 生成物 src/bindings.ts 与 Rust 代码同步
# Usage: ./scripts/check-bindings.sh
# 退出码 0 = 同步；非 0 = 过期（Rust 侧命令/类型/事件变更后未重新生成）
set -euo pipefail

cd "$(dirname "$0")/.."

WORKING_BINDINGS="src/bindings.ts"
BACKUP_BINDINGS="$(mktemp)"
TMP_BINDINGS="$(mktemp)"
# 无论成功失败都还原工作区文件并清理临时文件（幂等：源不存在时 mv 静默跳过）
cleanup() {
  mv -f "$BACKUP_BINDINGS" "$WORKING_BINDINGS" 2>/dev/null || true
  rm -f "$TMP_BINDINGS"
}
trap cleanup EXIT

# 1. 备份当前工作区文件；export_typescript_bindings 测试会直接覆盖 src/bindings.ts
cp "$WORKING_BINDINGS" "$BACKUP_BINDINGS"

# 2. 重新生成 bindings.ts（debug 构建的导出测试；Rust 侧改过命令/类型/事件时产物会变化）
(cd src-tauri && cargo test export_typescript_bindings --quiet) >/dev/null
cp "$WORKING_BINDINGS" "$TMP_BINDINGS"

# 3. 还原工作区文件（脚本自身不修改工作区）；cleanup 的 mv 因源已不存在而静默跳过
mv -f "$BACKUP_BINDINGS" "$WORKING_BINDINGS"

# 4. 与工作区当前文件比对：有差异说明 Rust 侧变更未重新生成
if cmp -s "$TMP_BINDINGS" "$WORKING_BINDINGS"; then
  echo "✓ bindings.ts 与 Rust 代码同步"
else
  echo "✗ bindings.ts 已过期：请重新运行 (cd src-tauri && cargo test export_typescript_bindings) 并提交生成物" >&2
  exit 1
fi
