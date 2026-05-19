# Design Review: RelWatch 代码审查发现

> 日期: 2026-05-19
>
> 基于 6 路并行 codebase-analyzer agent 对项目的全面审查生成。
> 项目被划分为 6 个独立部分分别分析：① 数据库持久层 ② 核心业务引擎 ③ Tauri 命令层
> ④ 桌面集成层 ⑤ 前端核心层 ⑥ 前端组件层

---

## 状态图例

| 标记 | 含义 |
|------|------|
| ` ` | 待处理 |
| `~` | 已修复（未提交） |
| `x` | 已修复（已提交） |
| `-` | 非问题（误报/搁置） |

---

## 一览表

### 🔴 严重 Bug

| 状态 | 编号 | 简述 | 优先级 |
|------|------|------|--------|
| x | B1 | COM 生命周期错配 | 最高 |
| - | B2 | Tray Badge 监听器失效（误报） | — |
| x | B3 | 前端 Tauri listen() 未清理 | 最高 |

### 🟡 高优先级设计问题

| 状态 | 编号 | 简述 | 优先级 |
|------|------|------|--------|
| | H1 | `notification_state` 独立成表 | 大版本 |
| | H2 | 迁移机制无版本号 | 低 |
| | H3 | poll 三入口代码大量重复 | 中 |
| | H4 | 日志记录不一致 | 中 |
| | H5 | 错误处理风格不统一 | 高 |
| - | H6 | DTO 边界模糊（建议搁置） | — |
| ~ | H7 | `updateSettings` 13 个位置参数 | 已修 |
| | H8 | ReleaseTab.vue 860 行模板膨胀 | 中 |

### 🟢 中低优先级改进建议

| 状态 | 编号 | 简述 | 优先级 |
|------|------|------|--------|
| | M1 | 错误处理用 `String` 类型擦除 | 低 |
| | M2 | `db/settings.rs` KV 层过度抽象 | 低 |
| | M3 | SQL 查询文本重复 4 次 | 低 |
| | M4 | `logs.message` 与 `message_key` 语义重叠 | 低 |
| | M5 | 重试逻辑（指数退避）两处重复 | 低 |
| | M6 | `update_settings` Rust 端样板代码 | 中 |
| | M7 | `commands/mod.rs` 引用风格不统一 | 低 |
| | M8 | lib.rs 窗口事件硬编码字符串 | 低 |
| | M9 | 三平台通知按钮回调 ~70 行重复 | 中 |
| ~ | M10 | `provide/inject` 使用字符串 key | 已修 |
| ~ | M11 | `invokeI18n` 死代码 + 丢失堆栈 | 已修 |
| | M12 | 右键菜单三处独立实现 | 低 |

---

## 详情

## 🔴 严重 Bug

### B1 — COM 生命周期错配 `④桌面集成`

- **状态**: x 已修复（`3514d02`）
- **文件**: `src-tauri/src/notify.rs`
- **问题**: `OnceLock` 不可逆 + `CoInitializeEx` 是线程级但 `OnceLock` 是进程级。退出 `uninit_com()` 后 COM 无法重新初始化。
- **修复**: 替换为 `thread_local! { static COM_CTX: ComGuard }`，RAII Guard 在线程退出时自动 `CoUninitialize()`。`uninit_com()` 变为空操作。

### B2 — Tray Badge 监听器失效 `④桌面集成`

- **状态**: - 误报
- **说明**: Rust 端 `EventId = u32`（Copy），drop 无副作用。需显式 `app.unlisten(event_id)` 才移除。当前代码未调用 `unlisten`，监听器正常工作。

### B3 — 前端 Tauri listen() 未清理 `⑤前端核心`

- **状态**: x 已修复（`3514d02`）
- **文件**: `src/App.vue`
- **问题**: 前端 `listen()` 返回 `UnlistenFn`（`() => void`），未存储和调用。HMR 热替换后旧回调累积，事件多次触发。
- **修复**: 存入 `unlisteners` 数组，`onUnmounted` 中统一遍历调用。

---

## 🟡 高优先级设计问题

### H1 — `notification_state` 独立成表 `①数据库`

- **状态**: ❌ 未修复
- **文件**: `src-tauri/src/db/releases.rs`, `src-tauri/src/db/init.rs`
- **问题**: 与 releases 是 1:1 关系（`UNIQUE(release_id)` 强制），却引入 LEFT JOIN + COALESCE + 两步插入。约 40% 的查询代码是多余的。
- **方案**: 合并到 releases 表：`status DEFAULT 'pending'` + `snooze_until` 列。
- **风险**: 🔴 需要 Schema 迁移脚本，波及面大。建议随大版本发布。

### H2 — 迁移机制无版本号 `①数据库`

- **状态**: ❌ 未修复
- **文件**: `src-tauri/src/db/init.rs`
- **问题**: 每次启动查 N 次 `pragma_table_info` 做属性探测。迁移内容（`ai_summary`、`message_key` 等）本应在初版 schema 中就包含。
- **方案**: 冻结当前 migrate() 内容为版本 1，标记 `PRAGMA user_version = 1`，后续增补用版本号判断。
- **风险**: 🟢 低，纯技术债清理，无功能侧影响。

### H3 — poll 三入口代码大量重复 `②核心引擎`

- **状态**: ❌ 未修复
- **文件**: `src-tauri/src/poll.rs`
- **问题**: `do_poll_async` / `trigger_poll` / `check_single_source` 三个函数共享 ~80% 管道逻辑（读配置、建 client、fetch、save、deepseek、notify），但各写各的。
- **方案**: 提取 `run_poll_cycle(app, sources, config)` 内部函数 + `PollConfig::from_db(conn)`。

### H4 — 日志记录不一致 `②核心引擎`

- **状态**: ❌ 未修复
- **文件**: `src-tauri/src/poll.rs`, `src-tauri/src/deepseek.rs`
- **问题**: poll.rs 用结构化 `write_log_key`，deepseek.rs 用自由文本 `write_log`；deepseek 部分错误只写 `log::error!` 未写入 DB（用户 UI 日志查不到）；某些路径双重写入。
- **方案**: 统一日志策略：所有重要事件用 `write_log_key`，所有错误同时写 DB + stderr。

### H5 — 错误处理风格不统一 `③命令层`

- **状态**: ❌ 未修复
- **文件**: `src-tauri/src/commands/setting.rs`, `src-tauri/src/commands/backup.rs`, `src-tauri/src/commands/source.rs`
- **问题**: 同时存在 i18n key 模式 (`"err.xxx|..."`) 和硬编码中文 (`"请先设置 DeepSeek API Key"`)。前端无法统一处理国际化。
- **方案**: 全部迁移到 i18n key 模式。

### H6 — DTO 边界模糊 `③命令层`

- **状态**: - 建议搁置
- **文件**: `src-tauri/src/types.rs`, `src-tauri/src/db/sources.rs`, `src-tauri/src/db/releases.rs`
- **问题**: 命令层直接返回 `db::sources::Source`、`db::releases::ReleaseInfo` 等数据库 struct 给前端，前后端紧耦合。
- **方案**: 在 `types.rs` 中定义独立 DTO，从 `db::*` 映射转换。
- **评估**: 当前项目规模下是过度设计，等接口量翻倍后再做。

### H7 — `updateSettings` 13 个位置参数 `⑤前端核心`

- **状态**: ~ 已修复（未提交）
- **文件**: `src/api.ts`, `src/components/SettingsTab.vue`
- **问题**: `updateSettings` 接受 13 个位置参数，新增设置需改函数签名 + 所有调用处。
- **修复**: 改为接收 `UpdateSettingsPayload` 命名对象，显式接口定义，TypeScript 强类型保障。

### H8 — ReleaseTab.vue 860 行模板膨胀 `⑥前端组件`

- **状态**: ❌ 未修复
- **文件**: `src/components/ReleaseTab.vue`
- **问题**: 搜索栏重复 4 次、Release 行模板重复 3 次。聚合视图的展开/折叠逻辑与三种视图模式耦合在一个文件中。
- **方案**: 拆分为 ReleaseToolbar / ReleaseRow / ReleaseSimpleList / ReleaseAggregatedList / ReleaseCalendar 子组件。

---

## 🟢 中低优先级改进建议

### M1 — 错误处理用 `String` 类型擦除 `①数据库`

- **状态**: ❌ 未修复
- **问题**: `map_err(|e| e.to_string())` 丢失错误类型信息，无法区分约束冲突/连接断开/磁盘满。
- **方案**: 定义统一 `DbError` 枚举替代 String。

### M2 — `db/settings.rs` KV 层过度抽象 `①数据库`

- **状态**: ❌ 未修复
- **问题**: 17 常量 + 自定义 `OptionalExt` trait + `apply_settings` 业务逻辑入侵。
- **方案**: 删除 `apply_settings` 移到业务层；或直接用 `Settings` struct + JSON 存单行。

### M3 — SQL 查询文本重复 `①数据库`

- **状态**: ❌ 未修复
- **问题**: 16 列 SELECT 在 `releases.rs` 中出现 4 次，增删字段需改 4 处。
- **方案**: 提取 SQL 常量或使用查询构建器。

### M4 — `logs.message` 与 `message_key` 语义重叠 `①数据库`

- **状态**: ❌ 未修复
- **问题**: `write_log_key` 将 `key` 同时写入 `message` 和 `message_key` 列，读取方困惑。
- **方案**: 明确语义：`message` 存可读文本，`message_key` 存结构化 key。

### M5 — 重试逻辑重复 `②核心引擎`

- **状态**: ❌ 未修复
- **问题**: 指数退避在 `github.rs` 和 `deepseek.rs` 两处重复，且缺少 jitter。
- **方案**: 提取共享 `RetryConfig` 工具函数。

### M6 — `update_settings` Rust 端样板代码 `③命令层`

- **状态**: ❌ 未修复
- **问题**: 手动读 13 个旧值 → 构造 13 元组 → 传给 `apply_settings`，大量重复。
- **方案**: 让 `apply_settings` 内部读取旧值，消除调用方样板；或用 serde 自动对比。

### M7 — `commands/mod.rs` 引用风格不统一 `③命令层`

- **状态**: ❌ 未修复
- **问题**: 缺少 `pub use backup::*;`，导致 lib.rs 中扁平路径与限定路径混用。
- **方案**: 补上 `pub use backup::*;`。

### M8 — lib.rs 窗口事件硬编码字符串 `③命令层`

- **状态**: ❌ 未修复
- **问题**: 窗口事件中直接写 `"minimize_to_tray"` 而非使用 `KEY_MINIMIZE_TO_TRAY` 常量。
- **方案**: 使用已有常量。

### M9 — 三平台通知按钮回调重复 `④桌面集成`

- **状态**: ❌ 未修复
- **文件**: `src-tauri/src/notify.rs`
- **问题**: Windows/Linux/回退三个平台的 go/ignore/snooze 回调逻辑 ~70 行完全重复，仅 action 解析方式不同。
- **方案**: 提取共享 `handle_notification_action` 函数。

### M10 — `provide/inject` 使用字符串 key `⑤前端核心`

- **状态**: ~ 已修复（未提交）
- **文件**: `src/injection-keys.ts`（新增）, `src/App.vue`, 各组件
- **问题**: `provide('showToast', ...)` 使用字符串 key，类型不安全。
- **修复**: 创建 `ShowToastKey: InjectionKey`，所有组件改用类型安全的 InjectionKey。

### M11 — `invokeI18n` 死代码 + 丢失堆栈 `⑤前端核心`

- **状态**: ~ 已修复（未提交）
- **文件**: `src/api.ts`
- **问题**: `err.toString = () => msg` 是死代码；`new Error(msg)` 丢失原始 Error 堆栈。
- **修复**: 删除死代码；复用原始 Error 对象，`err.message = msg` 保留原始堆栈。

### M12 — 右键菜单三处独立实现 `⑥前端组件`

- **状态**: ❌ 未修复
- **问题**: 右键菜单在 `SourceTab.vue` / `ReleaseTab.vue` / `App.vue` 三处各自实现。
- **方案**: 提取 `useContextMenu` composable + `<ContextMenu>` 组件。

---

## 各文件涉及问题一览

| 文件 | 涉及问题 |
|------|---------|
| `src-tauri/src/notify.rs` | B1(已修), M9 |
| `src-tauri/src/tray.rs` | B1/B3(均已修) |
| `src-tauri/src/lib.rs` | B3(已修), M8 |
| `src-tauri/src/poll.rs` | H3, H4 |
| `src-tauri/src/deepseek.rs` | H4, M5 |
| `src-tauri/src/github.rs` | M5 |
| `src-tauri/src/crypto.rs` | 非问题(设计取舍) |
| `src-tauri/src/db/init.rs` | H1, H2 |
| `src-tauri/src/db/releases.rs` | H1, M3 |
| `src-tauri/src/db/sources.rs` | M3 |
| `src-tauri/src/db/settings.rs` | M2, M6 |
| `src-tauri/src/db/logs.rs` | M4 |
| `src-tauri/src/commands/setting.rs` | H5, H7(已修), M6 |
| `src-tauri/src/commands/source.rs` | H5 |
| `src-tauri/src/commands/backup.rs` | H5 |
| `src-tauri/src/commands/mod.rs` | M7 |
| `src-tauri/src/types.rs` | H6 |
| `src/api.ts` | H7(已修), M11(已修) |
| `src/App.vue` | B3(已修), M10(已修) |
| `src/components/ReleaseTab.vue` | H8, M10(已修) |
| `src/components/SourceTab.vue` | M10(已修), M12 |
| `src/components/SettingsTab.vue` | H7(已修), M10(已修) |
| `src/injection-keys.ts` | M10(已修, 新增) |

---

## 建议处理顺序

```
优先级:  现在做 ────────────────── 以后做 ───────────────→ 搁置

已处理:   B1 B3  H7  M10 M11
待处理:   H5 → H3 → H4 → H8 → M9 → M12 → M2 M5 → H1 → H2 → H6
          (统一错误) (重复代码) (日志) (模板) (通知) (右键) (小重构) (大版本) (可做可不) (过度设计)
```
