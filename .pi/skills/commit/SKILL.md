---
name: commit
description: Create structured git commits by analyzing staged and unstaged changes and grouping them logically into one or more commits with clear, descriptive messages. Use when the user asks to commit, says "commit this" or "commit my changes", wants help writing a commit message, or has finished a chunk of work that needs committing.
argument-hint: [message]
allowed-tools: Bash(git *), Read, Glob, Grep
shell-timeout: 10
---

# Commit Changes

You are tasked with creating git commits for repository changes.

## Input

`$ARGUMENTS` — optional commit message hint. Empty/literal → infer from history and `git diff`.

## Metadata

```!
node "C:/Users/hheli/.pi/agent/npm/node_modules/@juicesharp/rpiv-pi/skills/_shared/git-changes.mjs"
echo "---recent-subjects---"
git log --pretty=%s -n 20 2>/dev/null || true
```

`---recent-subjects---` — up to 20 most recent commit subject lines, used in Step 2 to match the repository's existing commit-message style. Empty on a no-HEAD initial repo.

## Context:
- **In-session**: If there's conversation history, use it to understand what was built/changed
- **Standalone**: If no context available, rely entirely on git state and file inspection

## Process:

0. **Check git availability:**
   - If `in_repo:` in the Metadata block is `no`, tell the user: "This directory is not a git repository. Run `git init` to initialize one." Stop — do not proceed.

1. **Think about what changed:**
   - **If in-session**: Review the conversation history to understand what was accomplished.
   - The Metadata block gives you the file list and per-file diffstat (insertions/deletions). For files with a small diffstat (≲5 lines), the line counts alone are enough to write the message — skip `git diff`. Run `git diff <path>` only for files where the change is large or the intent isn't obvious from filename + line counts.
   - For untracked directories shown in status (e.g. `?? path/`), assume their contents are the change unless the directory has many files; do NOT `cat`/`head` files to verify obvious purpose.
   - Consider whether changes should be one commit or multiple logical commits.

2. **Run pre-commit checks:**
   执行 `git commit` **之前**，必须依次运行以下检查并确保全部通过，否则不允许提交：

   本项目使用 **pnpm** 作为包管理器（见全局 AGENTS.md）。所有前端脚本用 `pnpm exec` / `pnpm run`，不要用 `npx` / `npm run`。

   1. **TypeScript 编译检查（含 Vue SFC）**: `pnpm exec vue-tsc --noEmit` — 零错误
   2. **tsc 编译检查**（构建级 stricter 检查）: `pnpm exec tsc --noEmit` — 零错误
      > `vue-tsc` 对 Vue SFC 类型推断较宽松，`tsc` 能捕获 setProps 等额外的类型错误。
   3. **前端测试**: `pnpm exec vitest run` — 全部通过
   4. **前端 Lint**: `pnpm run lint` — 无 error
   5. **Rust 编译**: `cargo build`（或 `cargo check`）— 成功
   6. **Rust 测试**: `cargo test` — 全部通过
   7. **Rust Clippy**: `cargo clippy -- -D warnings` — 无 error
   8. **bindings.ts 同步检查**: `bash scripts/check-bindings.sh` — 输出 `✓ bindings.ts 与 Rust 代码同步` 且退出码 0
      > tauri-specta 生成物：Rust 侧命令/结构体/事件变更后必须重新生成 `src/bindings.ts` 并随提交一起带出（脚本会自动重新生成并 diff 比对；差异非空时按提示 `git add src/bindings.ts` 后重跑）

   > 如果某一步失败，必须先修复再提交。不得以「后续修复」为由跳过检查。
   >
   > **pnpm install 网络问题排查**：若 `pnpm add` / `pnpm install` 卡在 `GET ... error (unknown)` 反复重试，原因是 pnpm 走 socks5 代理对 registry.npmjs.org 的高并发 metadata 查询不可靠（attestations + minimumReleaseAge 策略验证批量失败）。解法：用国内镜像直连，例如 `pnpm add -D <pkg> --registry=https://registry.npmmirror.com/ --config.proxy= --config.https-proxy=`，既快又让供应链策略正常生效，无需跳过 minimumReleaseAge。

   若任一检查失败，**立即停止**，向用户报告具体失败的步骤和错误信息，不要继续规划 commit。

3. **Plan your commit(s):**
   - Identify which files belong together
   - Draft clear, descriptive commit messages
   - Use imperative mood in commit messages
   - **Match the subject style observed in `---recent-subjects---`** — same prefix convention (e.g. `feat:` / `fix(scope):` / `docs:` for Conventional Commits, gitmoji, bare sentence-case, ticket-prefixed, etc.), same length budget, same casing. If the sample is empty (initial repo) or mixed, default to imperative sentence-case with no prefix.
   - Focus on why the changes were made, not just what
   - Check for sensitive information (API keys, credentials) before committing

4. **Present your plan to the user:**
   - List the files you plan to add for each commit
   - Show the commit message(s) you'll use
   - Use the `ask_user_question` tool to confirm the commit plan. Question: "{N} commit(s) with {M} files. Proceed?". Header: "Commit". Options: "Commit (Recommended)" (Create the commit(s) as planned); "Adjust" (Change the grouping or commit messages); "Review files" (Show me the full diff before committing).

5. **Execute upon confirmation:**
   - Use `git add` with specific files (never use `-A` or `.`)
   - Create commits with your planned messages
   - Show the result with `git log --oneline --stat -n X` (where X = number of commits you just created) — `--stat` 会列出每个 commit 的增减行数（insertions/deletions），让用户直观看到每次提交的规模

## Important:

- **NEVER add co-author information or Claude attribution**
- Commits should be authored solely by the user
- Do not include any "Generated with Claude" messages
- Do not add "Co-Authored-By" lines
- Write commit messages as if the user wrote them

## Remember:

- Adapt your approach: use conversation context if available, otherwise infer from git state
- In-session: you have full context of what was done; Standalone: infer from git analysis
- Group related changes by purpose (feature, fix, refactor, docs)
- Keep commits atomic: one logical change per commit
- Split into multiple commits if: different features, mixing bugs with features, or unrelated concerns
- The user trusts your judgment - they asked you to commit
