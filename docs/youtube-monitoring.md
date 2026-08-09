# YouTube 频道监控功能规划

> 状态：**已实现**（v1.9.0 发布；v1.10.0 起配套 B 站 UP 主监控，实现思路见 `bilibili.rs` 与 `commands/bilibili_login.rs`）
> 范围：新增 YouTube 频道监控源；每个源通过复选框配置订阅内容（视频/直播）；不接入 DeepSeek 摘要；前端融入现有设计。
> v2 变更：新增 **Data API v3 双模式**——配置 `youtube_api_key` 后走官方 API（规避 RSS 风控），未配置时降级 RSS。

## 1. 需求

1. 支持添加 YouTube 频道为监控源（输入频道链接 / @handle / channel_id）
2. 每个 YouTube 源通过**复选框**配置订阅内容类型：
   - 视频（默认勾选）
   - 直播（默认勾选）
   - 帖子（暂不支持，复选框置灰并提示）
3. YouTube 源**不**生成 DeepSeek 摘要/翻译（用户明确要求无需摘要功能）
4. 前端界面融入现有设计（复用 SourceTab / ReleaseTab 现有交互与视觉体系）

## 2. 技术方案：RSS 订阅源 + Data API v3 双模式

> v2 起支持双模式：**配置 YouTube Data API Key 时走官方 API**（`youtube.googleapis.com`），否则降级 RSS。
> 动机：2025 年末起 YouTube 对数据中心 IP 的 RSS 端点（`feeds/videos.xml`）做 IP 风控，统一返回 404，
> RSS 模式无法区分“频道无内容”与“被风控”；Data API 走 Google API 基础设施，对数据中心 IP 宽容。

### 2.1 数据获取：RSS 模式（playlist_id 前缀技巧）

YouTube 官方 RSS（`youtube.com/feeds/videos.xml`）支持按 `channel_id` 或 `playlist_id` 拉取。经社区验证，将频道 ID 前缀替换可得到**按内容类型过滤**的 feed（未公开但长期稳定）：

| 订阅类型 | playlist_id 构造 | 说明 |
|---|---|---|
| 视频 | `UULF` + `channel_id[2:]` | 仅长视频（不含 Shorts、不含直播） |
| 直播 | `UULV` + `channel_id[2:]` | 仅直播回放（Live VOD） |
| 全上传 | `UU` + `channel_id[2:]` | 兜底（不细分） |
| 帖子 | — | 无 RSS 通道，**第一版不支持** |

示例：频道 `UCXuqSBlHAE6Xw-yeJA0Tunw`
- 视频 feed：`https://www.youtube.com/feeds/videos.xml?playlist_id=UULFXuqSBlHAE6Xw-yeJA0Tunw`
- 直播 feed：`https://www.youtube.com/feeds/videos.xml?playlist_id=UULVXuqSBlHAE6Xw-yeJA0Tunw`

轮询逻辑：按该源勾选的类型拉取对应 feed；多选时并发拉取多个 feed 后合并（`UNIQUE(source_id, tag_name)` 天然去重，tag_name = videoId）。

### 2.4 Data API v3 模式（配置 youtube_api_key 后启用）

请求链（全部带 `key` 参数，默认每日 10,000 units 配额）：

| 步骤 | 端点 | 用途 | 配额 |
|---|---|---|---|
| 1 | `channels.list?part=contentDetails&id={channelId}` | 取 uploads 播放列表 id | 1 unit |
| 2 | `playlistItems.list?part=snippet&playlistId={uploads}&maxResults=50` | 翻页拉视频列表 | 1 unit/页 |
| 3（始终） | `videos.list?part=snippet,contentDetails&id={ids}` | `liveBroadcastContent` 区分视频/直播 + `contentDetails.duration` 时长 | 1 unit/50个 |

- 每源每次检查约 2-3 units（50 视频以内）；`videos.list` 同时补全时长与精确类型，RSS 模式无时长字段
- 历史拉取：源开启「拉取历史版本」后，YouTube 源**每次**检查都按 `fetch_history_count` 拉取（0=全部翻页），save 按 `UNIQUE(source_id, tag_name)` 去重——重新配置 API Key 后无需删除源即可补拉历史
- `resolve_owner`：`@handle` → `channels.list?forHandle=`；`c/` `user/` 链接 API 不支持，回退 HTML 解析
- `verify_and_describe`：`channels.list?part=snippet` 的 `snippet.title` 即真实频道名
- key 无效（`keyInvalid`/`ipRefererBlocked`）与配额用尽（`quotaExceeded`）映射为专属 i18n 错误
- 存储：`app_settings.youtube_api_key`（与 github_token 相同 crypto 加密）；`set_youtube_api_key` 命令独立保存；`AppSettings.youtube_api_key_set` 仅暴露“是否已设置”
- 调度：poll / add_source 按 `source_type == "youtube"` 将 key 作为 token 传给 adapter；未配置时自动降级 RSS

### 2.2 频道 ID 解析（verify / 添加时）

用户输入支持四种形式，统一解析为 `channel_id`（UC 开头）存入 `sources.owner`：

| 输入 | 解析方式 |
|---|---|
| `https://www.youtube.com/channel/UCxxxx` | 直接提取 `UCxxxx` |
| `https://www.youtube.com/@handle` / `c/xxx` / `user/xxx` | 请求页面 HTML，提取 `<meta itemprop="channelId" content="UC...">`（或 `"channelId":"UC..."`） |
| `@handle` 纯文本 | 同上 |
| 已解析的 channel_id | 直接接受 |

解析在 `verify_and_describe` 中完成：拉取频道页 → 提取 channelId → 拉 RSS feed 验证 → 返回 feed `<title>` 作为频道名（description）。

### 2.3 RSS 条目 → releases 表映射

| releases 字段 | RSS 来源 |
|---|---|
| tag_name | `<yt:videoId>`（天然唯一，天然去重） |
| release_name | `<title>`（视频/直播标题） |
| html_url | `<link rel="alternate">`（watch?v=ID） |
| published_at | `<published>` |
| body | `<media:description>`（可为空） |
| extra_metadata | JSON：`{"kind":"video"/"live","thumbnail":"<media:thumbnail url>"}` |

## 3. 数据模型变更

### 3.1 Migration 11：sources.config

```sql
ALTER TABLE sources ADD COLUMN config TEXT;
```

每个 YouTube 源存 JSON：`{"videos":true,"live":true,"posts":false}`（github/huggingface 源为 NULL）。

- `db/sources.rs`：`Source` 结构体加 `config: Option<String>`；`add_source` / `get_source` / `list_sources` 同步；新增 `update_source_config(conn, id, config)`
- `db/init.rs`：migrate() 增加列存在性检测（沿用现有模式）

## 4. Rust 端改动清单

### 4.1 新增 `src-tauri/src/youtube.rs`（核心）

- `pub struct YoutubeAdapter` 实现 `source::SourceAdapter`
- `resolve_channel_id(client, input) -> Result<String, (u16, String)>`：四种输入归一化
- `fetch_feed(client, channel_id, kind) -> Result<Vec<Entry>, (u16, String)>`：构造 playlist_id URL，GET XML
- RSS 解析：新增 `quick-xml` 依赖（Cargo.toml），解析 Atom 命名空间 + `<media:*>` 扩展
- `save`：条目 → `insert_release`，extra_metadata 写缩略图/类型；`max_count=1` 语义与 github 对齐（遇到已入库即停）
- `verify_and_describe`：解析 channelId + 拉 feed 验证 + 返回频道名
- `fetch_all`：RSS 单页（约 15 条），与 `fetch` 同实现（复用），无需翻页

### 4.2 `source.rs`

`get_adapter` 增加 `"youtube" => Ok(Box::new(crate::youtube::YoutubeAdapter))`

### 4.3 命令层 `commands/source.rs`

- `add_source`：新增 `config: Option<String>` 参数（前端传 YouTube 订阅复选框 JSON），透传到 `db::sources::add_source`
- `update_source`：支持 `config: Option<String>` 参数更新

### 4.4 轮询 `poll.rs`：跳过 YouTube 摘要/翻译

用户明确要求 YouTube 源无需 DeepSeek 摘要。改动点：

1. `do_poll_core` / `check_single_source`：调用 `generate_summaries_for_new` / `generate_translations_for_new` **前**，过滤掉 source_type == "youtube" 的 saved 条目（按 release id 反查 source）
2. 重试查询 `get_releases_without_summary` / `get_releases_without_translation`：SQL 改为 `JOIN sources` 并排除 `source_type = 'youtube'`（`db/releases.rs`）

> 前端无需改动 AI 逻辑：YouTube 源 ai_summary/body_translated 恒为 null，现有 UI 自动不显示摘要/译文视图。

### 4.5 通知 `notify.rs`

无需改动：`send_release_notification` 已通用（title = release_name，url = html_url）。

## 5. 前端改动清单

### 5.1 `src/api/sources.ts`

- `Source` 接口加 `config: string | null`
- `parseYoutubeUrl(raw)`：解析 channel / @handle / c/ / user / 纯 handle → `{ type: 'youtube', owner, repo: '' }`
- `parseSourceUrl`：增加 youtube 分支（含 `youtube.com` / `youtu.be` / `@handle` 识别）
- `addSource(sourceType, owner, repo, config?)` / `updateSource(..., config?)`

### 5.2 `src/components/SourceTab.vue`

- **添加区**：检测输入为 YouTube 时，输入行下方展开订阅复选框行（视频 / 直播 / 帖子），帖子 disabled + tooltip「暂不支持」；添加时要求至少勾选一项
- **每行更多菜单**：YouTube 源增加「订阅内容」入口，弹复选框面板即时保存（复用 `moreDropdown` 结构）
- **类型徽标**：`.source-type-badge.youtube` 样式 + `youtube-icon`
- **跳转**：`openSourceUrl` 对 youtube 打开 `https://www.youtube.com/channel/{owner}`；`sourceQuery`/`sourceKey` 沿用 `owner`（repo 为空时拼接逻辑需兼容）

### 5.3 `public/icons.svg`

新增 `youtube-icon`（标准 YouTube 播放按钮造型，红底白三角，与现有 github/huggingface 徽标风格一致）

### 5.4 i18n（zh-CN.ts / en-US.ts / index.ts）

新增：`source.placeholder` 提示追加 YouTube、`source.type_youtube`、`source.subscribe_videos`、`source.subscribe_live`、`source.subscribe_posts`、`source.posts_unsupported`、`source.require_subscribe` 等

### 5.5 展示增强（可选）

`ReleaseItem.vue` / `ReleaseDetailModal.vue`：extra_metadata 含 thumbnail 时展示缩略图 + 类型角标（复用 HF metadata 解析模式）；此部分可与主功能分离后单独做。

## 6. 测试计划

| 层 | 用例 |
|---|---|
| Rust `youtube.rs` | RSS XML 解析（wiremock 返回 XML）；playlist_id 前缀构造；save 去重 / max_count / kind 标记；resolve_channel_id 各输入形态（mock 频道页 HTML） |
| Rust `db` | migration 列存在性；config 默认 NULL / 读写 |
| Rust `poll` | 摘要/翻译跳过 youtube 源（saved 过滤 + retry 查询排除） |
| 前端 | parseYoutubeUrl 单测（vitest）；SourceTab 复选框显隐 / 提交参数 / 帖子禁用 |
| 回归 | 现有 `pnpm test` + `cargo test` + `pnpm lint` 全绿；`coverage` 门槛不降 |

## 7. 实施顺序

1. **Phase 1 — 后端骨架**：Cargo.toml 加 quick-xml；Migration 11；db/sources.rs config 支持；source.rs 分发；youtube.rs 适配器 + 单测
2. **Phase 2 — 编排接入**：commands/source.rs 参数透传；poll.rs 摘要跳过（含 db/releases.rs retry 排除）；相关单测
3. **Phase 3 — 前端**：api/sources.ts 解析与参数；icons.svg；SourceTab 复选框 + 徽标 + 更多菜单；i18n
4. **Phase 4 — 收尾**：补测 + lint + coverage + commit skill 提交流程

## 8. 风险与说明

- **playlist_id 前缀 hack**：官方未公开、社区多年验证稳定；若未来失效，退路是 `channel_id` 全量 feed（视频/直播合并）或 Data API
- **RSS 端点 IP 风控（已实际触发）**：数据中心 IP 下 `feeds/videos.xml` 统一 404（2025 年末起全球性现象，UA/HTTP2/Cookie 伪装均无效）。**配置 Data API Key 是正解**；未配置时保持 RSS 行为不变
- **@handle 解析依赖频道页 HTML 结构**：YouTube 改版可能影响；API 模式下 `forHandle` 查询更稳定，失败时提示用户改用 channel 链接/ID
- **RSS 深度**：每 feed 约 15 条，仅够增量监控；API 模式支持翻页（50/页）拉取历史，`fetch_history` 对 youtube 源更有意义
- **帖子**：RSS/Data API 均无公开端点，第一版不做；复选框保留占位并置灰，后续可单独评估 Innertube 内部接口或网页解析
- **直播语义**：按用户确认，直播回放（Live VOD）发布即视为一条新内容通知，不区分「直播中/即将开播」
- **API 配额**：免费配额 10,000 units/天（按 Google 项目共享）；配额用尽时该轮检查失败并记 WARN，不静默
