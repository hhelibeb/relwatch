# Release 工作流

准备发布新版本时，必须先加载 `.pi/skills/release/SKILL.md` 中的 release skill，严格按照其中的流程执行。

# Commit 工作流

未经用户允许，不要自行commit、push。准备Commit时，必须先加载 `.pi/skills/commit/SKILL.md` 中的 commit skill

# 术语约定

本项目是 Tauri 桌面应用 **relwatch**。用户提到的以下术语**默认指 relwatch 应用内置功能**，与任何外部 Agent 工具（如 pi 自身）无关：

- **"Agent 工作区"**：relwatch 右侧聊天面板（`src/components/AgentWorkspace.vue`），可拖入监控源/版本、@ 选 Skill、与本地 pi Agent（RPC 常驻进程）对话。
  - 后端：`src-tauri/src/agent.rs`、`src-tauri/src/agent_rpc.rs`、`src-tauri/src/db/agent.rs`
  - 数据：SQLite `agent_runs` 表；会话文件 `%APPDATA%\RelWatch\agent-sessions\ws-<session_key>.jsonl`

若指外部 Agent 工具功能（如 pi 的 subagent / Agent 面板 / worktree / watchdog），会明确说明。排查问题优先查 relwatch 代码与 `%APPDATA%\RelWatch\` 数据。

# 时区规则（重要，比对之前先统一时区）

排查时经常要比较两类时间，**二者基准不同，比对前必须先换算成同一时区，否则方向/先后会判反**：

- **UTC**：relwatch 的会话数据文件 `%APPDATA%\RelWatch\agent-sessions\ws-<session_key>.jsonl` 里每个事件的 `timestamp` 字段是 UTC，且带 `Z`（如 `2026-08-20T11:43:02.565Z`）。网页/API 返回的发布时间（如 YouTube 的 `2026-08-20T11:00:17Z`）也多为 UTC。
- **本地时间（本机 +0800）**：`stat`/`ls -l`/`dir` 列出的文件修改时间、SQLite `app_settings` 等写入时间、`date` 输出，均为本地时间（本机为 UTC+8）。

# 注释规则

几乎不要写注释，只在复杂场景下解释为什么，而不需要解释做什么，让代码自解释做了什么。不要把注释当做笔记使用。
