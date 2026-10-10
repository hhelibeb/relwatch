---
name: commit
description: 分析已暂存与未暂存的改动，按逻辑分组，生成一个或多个结构清晰、信息充分的 commit。当用户要求提交、说 "commit this" 或 "commit my changes"、需要帮忙写 commit message，或一段工作完成后需要提交时使用。
argument-hint: "[message]"
---

# 创建 commit

你的任务是为仓库的改动创建 git commit。

## 输入

`$ARGUMENTS` —— 可选的 commit message 提示。为空或未被替换为字面量时 → 从历史与 `git diff` 推断。

## 背景：
- **会话内**：若有对话历史，用它理解本次构建/改动了什么
- **独立运行**：若无上下文，完全依赖 git 状态与文件检查

## 流程：

0. **检查 git 可用性：**
   - 运行 `git rev-parse --show-toplevel`；若失败，告知用户："当前目录不是 git 仓库。运行 `git init` 初始化一个。" 然后停止，不要继续。

1. **梳理改动了什么：**
   - **会话内**：回顾对话历史，理解本次完成了什么。
   - 自行采集快照：`git status --short`、`git diff HEAD --stat --ignore-submodules=all`、`git log --pretty=%s -n 20`（最后一条是第 3 步的风格样本；在无 HEAD 的初始仓库下为空）。
   - 快照给出文件清单与逐文件 diffstat（新增/删除行数）。对 diffstat 很小的文件（≲5 行），行数本身已足够写出 message —— 跳过 `git diff`。仅当改动较大、或从文件名加行数看不出意图时，才运行 `git diff <path>`。
   - 对 status 中出现的未跟踪目录（如 `?? path/`），除非目录内文件很多，默认其内容即为改动；不要用 `cat`/`head` 去核对显而易见的用途。
   - 判断改动应做成单个 commit 还是拆成多个逻辑 commit。

2. **运行提交前检查：**
   执行 `git commit` **之前**，必须依次运行以下检查并确保全部通过，否则不允许提交：

   本项目使用 **pnpm** 作为包管理器（见全局 AGENTS.md）。所有前端脚本用 `pnpm exec` / `pnpm run`，不要用 `npx` / `npm run`。

   1. **TypeScript 编译检查（含 Vue SFC）**: `pnpm exec vue-tsc --noEmit` — 零错误
   2. **tsc 编译检查**: `pnpm exec tsc --noEmit` — 零错误
      > 本项是**本地自愿门禁**：CI 只跑 `vue-tsc`（`pnpm run build` / `pnpm run typecheck`），
      > release skill Step 5 也把它当额外保险跑一遍。它与 `vue-tsc` 覆盖面高度重叠
      > （SFC 在 `tsc` 眼里走 `src/env.d.ts` 的 `*.vue` shim，对组件内部与模板的检查更弱），
      > 保留它只为兜 `vue-tsc` 的漏检——**遇到真实漏检案例时，把最小复现补到这条注释里**；
      > 长期拿不出案例就删掉本项，免得本地门禁比 CI 更严。
   3. **前端测试**: `pnpm exec vitest run` — 全部通过
   4. **前端 Lint**: `pnpm run lint` — 无 error
   5. **Rust 编译**: `cargo check` — 成功
      > 只做 Rust 编译即可，**不要跑 `tauri build`**：`bundle.createUpdaterArtifacts` 为 `true` 时
      > tauri CLI 强制要求签名私钥，缺 `TAURI_SIGNING_PRIVATE_KEY` 会直接构建失败（上游设计）。
      > 打包/更新产物的验证走 CI 产出的 draft release，详见 release skill Step 9.2。
   6. **Rust 测试**: `cargo test` — 全部通过
   7. **Rust Clippy**: `cargo clippy -- -D warnings` — 无 error
   8. **bindings.ts 同步检查**: `bash scripts/check-bindings.sh` — 输出 `✓ bindings.ts 与 Rust 代码同步` 且退出码 0
      > tauri-specta 生成物：Rust 侧命令/结构体/事件变更后必须重新生成 `src/bindings.ts` 并随提交一起带出（脚本会自动重新生成并 diff 比对；差异非空时按提示 `git add src/bindings.ts` 后重跑）

   > 如果某一步失败，必须先修复再提交。不得以「后续修复」为由跳过检查。
   >
   > **pnpm install 网络问题排查**：若 `pnpm add` / `pnpm install` 卡在 `GET ... error (unknown)` 反复重试，原因是 pnpm 走 socks5 代理对 registry.npmjs.org 的高并发 metadata 查询不可靠（attestations + minimumReleaseAge 策略验证批量失败）。解法：用国内镜像直连，例如 `pnpm add -D <pkg> --registry=https://registry.npmmirror.com/ --config.proxy= --config.https-proxy=`，无需跳过 minimumReleaseAge。

   若任一检查失败，**立即停止**，向用户报告具体失败的步骤和错误信息，不要继续规划 commit。

3. **规划你的 commit：**
   - 确定哪些文件属于同一组
   - 起草清晰、简洁的 commit message
   - commit message 使用祈使语气
   - **与仓库既有的 subject 风格保持一致**（第 1 步采集的 `git log --pretty=%s -n 20` 样本）——沿用相同的前缀约定（如 Conventional Commits 的 `feat:` / `fix(scope):` / `docs:`、gitmoji、无前缀的 sentence-case、ticket 编号前缀等）、相同的长度预算、相同的大小写风格。若样本为空（初始仓库）或风格混杂，默认使用无前缀的祈使句 sentence-case。
   - 侧重说明改动的**原因**，而不只是改了什么
   - 提交前检查是否含敏感信息（API key、凭据、浏览历史、订阅列表（RSS/频道/邮件订阅）、收藏与书签、播放与阅读记录等）

4. **向用户展示方案：**
   - 列出每个 commit 计划加入的文件
   - 展示将要使用的 commit message
   - 用 `ask_user_question` 工具确认提交方案。问题："{N} 个 commit，共 {M} 个文件。是否继续？"。Header："Commit"。选项："提交（推荐）"（按方案创建 commit）；"调整"（修改分组或 commit message）；"查看文件"（提交前展示完整 diff）。

5. **确认后执行：**
   - 用 `git add` 指定具体文件（绝不使用 `-A` 或 `.`）
   - 用规划好的 message 创建 commit
   - 用 `git log --oneline --stat -n X` 展示结果（X = 刚创建的 commit 数）—— `--stat` 会列出每个 commit 的增减行数（insertions/deletions），让用户直观看到每次提交的规模

## 重要：

- commit 只应由用户本人署名
- 不要添加 "Co-Authored-By" 行
- 写 commit message 时，要像用户自己写的那样

## 记住：

- 灵活适配：有对话上下文就用它，没有就从 git 状态推断
- 会话内：你掌握改动的完整语境；独立运行：从 git 分析推断
- 按目的分组相关改动（feature、fix、refactor、docs）
- 保持 commit 原子化：一个 commit 一个逻辑改动
- 出现以下情况就拆成多个 commit：不同 feature、bug 修复与 feature 混在一起、或互不相关的关注点
- 用户信任你的判断 —— 是他们让你提交的
