# Release 工作流

准备发布新版本时，必须先加载 `.pi/skills/release/SKILL.md` 中的 release skill，严格按照其中的 13 步流程执行，不可跳过任何步骤。

# Commit 工作流

准备Commit时，必须先加载 `.pi/skills/commit/SKILL.md` 中的 commit skill

# 术语约定（重要，防误会）

本项目是 Tauri 桌面应用 **relwatch**。用户提到的以下术语**默认指 relwatch 应用内置功能**，与任何外部 Agent 工具（含 pi 自身）无关：

- **"Agent 工作区" / "工作区"**：relwatch 右侧聊天面板（`src/components/AgentWorkspace.vue`），可拖入监控源/版本、@ 选 Skill、与本地 pi Agent（RPC 常驻进程）对话。
  - 后端：`src-tauri/src/agent.rs`、`src-tauri/src/agent_rpc.rs`、`src-tauri/src/db/agent.rs`
  - 数据：SQLite `agent_runs` 表；会话文件 `%APPDATA%\RelWatch\agent-sessions\ws-<session_key>.jsonl`
  - 超时：`app_settings.agent_timeout_seconds`（默认 300 秒），超时记 run 为 timeout（error=`err.agent.timeout|<秒>`）
- **"Agent 会话"**：同一 session_key 的多次提交共享一个会话文件（多轮对话）。

若指外部 Agent 工具功能（如 pi 的 subagent / Agent 面板 / worktree / watchdog），会明确说明。排查问题优先查 relwatch 代码与 `%APPDATA%\RelWatch\` 数据。

# Git 命令约定（重要，防卡住）

执行 `git log`、`git diff`、`git status` 等分页命令时统一加 `--no-pager`（如 `git --no-pager log`），防止默认分页器 less 在非交互终端等待输入而卡住不返回。

