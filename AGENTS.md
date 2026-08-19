# Release 工作流

准备发布新版本时，必须先加载 `.pi/skills/release/SKILL.md` 中的 release skill，严格按照其中的 13 步流程执行，不可跳过任何步骤。

# Commit 工作流

准备Commit时，必须先加载 `.pi/skills/commit/SKILL.md` 中的 commit skill

# 术语约定（重要，防误会）

本项目是 Tauri 桌面应用 **relwatch**（版本发布监控工具）。用户提到的以下术语**默认指 relwatch 应用内置功能**，不是 pi coding agent 环境自身功能：

- **"Agent 工作区" / "工作区"**：relwatch 右侧聊天面板（`src/components/AgentWorkspace.vue`），可把监控源/版本拖入、@ 选 Skill、与本地 pi Agent（RPC 常驻进程）对话。
  - 后端：`src-tauri/src/agent.rs`（执行器 + 工作区会话模型）、`src-tauri/src/agent_rpc.rs`（pi RPC 客户端）、`src-tauri/src/db/agent.rs`（配置与 run 记录）
  - 数据：SQLite `agent_runs` 表（每次提交的 run：pending/running/success/failed/timeout/cancelled）；会话文件 `%APPDATA%\RelWatch\agent-sessions\ws-<session_key>.jsonl`
  - 超时：设置 `app_settings.agent_timeout_seconds`（默认 300 秒），超时 kill 并记 run 为 timeout（error=`err.agent.timeout|<秒>`）
- **"Agent 会话"**：同一 session_key 的多次提交共享一个 pi 会话文件（多轮对话）。

若用户指 pi 自身功能（subagent / Agent 面板 / worktree / watchdog 等），会明确说"pi 的 Agent"。排查用户问题优先查 relwatch 代码与 `%APPDATA%\RelWatch\` 下的数据，不要先去查 pi 的扩展或文档。

# Git 命令约定（重要，防卡住）

执行 `git log`、`git diff`、`git status` 等分页命令时统一加 `--no-pager`（如 `git --no-pager log`），防止默认分页器 less 在非交互终端等待输入而卡住不返回。

