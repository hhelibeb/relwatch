# Commit 前检查

执行 `git commit` **之前**，必须依次运行以下检查并确保全部通过，否则不允许提交：

1. **TypeScript 编译检查**: `npx vue-tsc --noEmit` — 零错误
2. **前端测试**: `npx vitest run` — 全部通过
3. **前端 Lint**: `npm run lint` — 无 error
4. **Rust 编译**: `cargo build`（或 `cargo check`）— 成功
5. **Rust 测试**: `cargo test` — 全部通过
6. **Rust Clippy**: `cargo clippy -- -D warnings` — 无 error

> 如果某一步失败，必须先修复再提交。不得以「后续修复」为由跳过检查。

# Release 工作流

准备发布新版本时，必须先加载 `.pi/skills/release/SKILL.md` 中的 release skill，严格按照其中的 12 步流程执行，不可跳过任何步骤。
