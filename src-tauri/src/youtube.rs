//! YouTube 频道监控适配器（RSS 数据源 + Data API v3 双模式）。
//!
//! ## RSS 模式（默认，无需配置）
//! 对接 YouTube 官方 RSS（`youtube.com/feeds/videos.xml`），利用未公开但长期稳定的
//! playlist_id 前缀技巧按内容类型过滤：
//! - `UULF` + `channel_id[2:]` → 仅长视频（不含 Shorts、不含直播）
//! - `UULV` + `channel_id[2:]` → 仅直播回放（Live VOD）
//!
//! 免费、无需 API Key。社区帖子无 RSS 通道，第一版不支持（前端复选框置灰）。
//! 注意：YouTube 会对数据中心 IP 的 RSS 端点（feeds/videos.xml）做风控，返回 404，
//! 该模式下无法区分“频道无此类型内容”与“被风控”。
//!
//! ## Data API v3 模式（配置 youtube_api_key 后启用，规避 RSS 风控）
//! 走 `youtube.googleapis.com`（Google API 基础设施，对数据中心 IP 宽容），
//! 请求链：`channels.list`（拿 uploads 播放列表）→ `playlistItems.list`（拿视频列表）
//! → 仅当需要区分视频/直播时 `videos.list`（liveBroadcastContent 过滤）。
//! 配额：每源每次检查约 2-3 units（默认每日 10,000 units）。
//! 未配置 key 时自动降级回 RSS 模式。
//!
//! 每个源的订阅内容类型存于 `sources.config`（JSON）：
//! `{"videos":true,"live":true,"posts":false}`；缺失时默认 videos+live 勾选。

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::db::releases;
use crate::db::sources::Source;
use crate::source::SourceAdapter;

const YT_FEED_BASE: &str = "https://www.youtube.com/feeds/videos.xml";
const YT_WATCH_BASE: &str = "https://www.youtube.com/watch";
const YT_API_BASE: &str = "https://youtube.googleapis.com/youtube/v3";

/// playlistItems.list 单页上限（Data API v3 固定为 50）。
const YT_API_PAGE_SIZE: usize = 50;

/// 订阅内容类型（对应 RSS playlist_id 前缀）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum FeedKind {
    /// 仅长视频（UULF）
    Video,
    /// 仅直播回放（UULV）
    Live,
}

impl FeedKind {
    fn playlist_prefix(self) -> &'static str {
        match self {
            FeedKind::Video => "UULF",
            FeedKind::Live => "UULV",
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            FeedKind::Video => "video",
            FeedKind::Live => "live",
        }
    }
}

/// 频道订阅配置（解析自 sources.config JSON）。
#[derive(Debug, Clone, Copy, PartialEq)]
struct SubscribeConfig {
    videos: bool,
    live: bool,
    posts: bool,
}

impl Default for SubscribeConfig {
    fn default() -> Self {
        Self {
            videos: true,
            live: true,
            posts: false,
        }
    }
}

impl SubscribeConfig {
    fn from_source(source: &Source) -> Self {
        let default = Self::default();
        let Some(raw) = source.config.as_deref() else {
            return default;
        };
        match serde_json::from_str::<serde_json::Value>(raw) {
            Ok(v) => Self {
                videos: v["videos"].as_bool().unwrap_or(default.videos),
                live: v["live"].as_bool().unwrap_or(default.live),
                posts: v["posts"].as_bool().unwrap_or(false),
            },
            Err(_) => default,
        }
    }

    /// 需要拉取的 feed 类型列表（按订阅勾选）。
    fn kinds(&self) -> Vec<FeedKind> {
        let mut kinds = Vec::new();
        if self.videos {
            kinds.push(FeedKind::Video);
        }
        if self.live {
            kinds.push(FeedKind::Live);
        }
        kinds
    }
}

/// RSS 条目（fetch 阶段产出，序列化为 JSON 传给 save）。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct FeedEntry {
    video_id: String,
    title: String,
    published: String,
    description: Option<String>,
    thumbnail: Option<String>,
    kind: FeedKind,
    /// Data API 模式的视频时长（ISO 8601，如 `PT12M34S`）；RSS 模式无此字段。
    duration: Option<String>,
    /// Data API 模式的播放量（statistics.viewCount）；RSS 模式无此字段。
    view_count: Option<i64>,
}

impl FeedEntry {
    fn html_url(&self) -> String {
        format!("{}/?v={}", YT_WATCH_BASE, self.video_id)
    }

    fn metadata_json(&self) -> String {
        serde_json::json!({
            "kind": self.kind.as_str(),
            "thumbnail": self.thumbnail,
            "duration": self.duration,
            "view_count": self.view_count,
        })
        .to_string()
    }
}

/// RSS feed 解析结果：频道标题 + 条目列表。
struct FeedParseResult {
    channel_title: String,
    entries: Vec<FeedEntry>,
}

/// 文本收集目标（状态机：区分当前文本属于哪个元素）。
#[derive(Clone, Copy, PartialEq)]
enum TextTarget {
    None,
    FeedTitle,
    EntryTitle,
    VideoId,
    Published,
    Description,
}

/// XML 实体解义：`is_ref = true` 表示输入是实体名（如 `amp`），补 `&;` 后解义。
fn unescape_text(decoded: &str, is_ref: bool) -> Result<String, String> {
    let raw = if is_ref {
        format!("&{};", decoded)
    } else {
        decoded.to_string()
    };
    Ok(quick_xml::escape::unescape(&raw)
        .map_err(|e| format!("err.rss_parse|{}", e))?
        .into_owned())
}

/// 把解析出的文本按 `target` 归属到对应字段（Text / GeneralRef 事件共用）。
fn apply_text_content(
    channel_title: &mut String,
    cur: &mut Option<FeedEntry>,
    target: TextTarget,
    text: &str,
) {
    match target {
        TextTarget::FeedTitle => *channel_title = text.trim().to_string(),
        TextTarget::EntryTitle => {
            if let Some(entry) = cur.as_mut() {
                entry.title.push_str(text);
            }
        }
        TextTarget::VideoId => {
            if let Some(entry) = cur.as_mut() {
                entry.video_id.push_str(text);
            }
        }
        TextTarget::Published => {
            if let Some(entry) = cur.as_mut() {
                entry.published.push_str(text);
            }
        }
        TextTarget::Description => {
            if let Some(entry) = cur.as_mut() {
                if !text.trim().is_empty() {
                    entry.description.get_or_insert_with(String::new).push_str(text);
                }
            }
        }
        TextTarget::None => {}
    }
}

/// 解析 YouTube 视频 RSS（Atom + media 命名空间扩展）。
///
/// `kind` 决定条目标记为视频还是直播。解析器只关心 local_name，
/// 不受命名空间前缀（`yt:` / `media:` / 默认）影响。
fn parse_feed(xml: &str, kind: FeedKind) -> Result<FeedParseResult, String> {
    use quick_xml::events::Event;

    let mut reader = quick_xml::Reader::from_str(xml);
    // 注意：不用 trim_text(true)——实体引用（&amp; 等）会被拆成独立事件，
    // trim 会误删片段间空格；改由收集完成后统一 trim。
    let mut buf: Vec<u8> = Vec::new();

    let mut channel_title = String::new();
    let mut entries: Vec<FeedEntry> = Vec::new();

    let mut in_entry = false;
    let mut in_media_group = false;
    let mut target = TextTarget::None;
    let mut cur: Option<FeedEntry> = None;

    loop {
        let ev = reader
            .read_event_into(&mut buf)
            .map_err(|e| format!("err.rss_parse|{}", e))?;
        match ev {
            Event::Start(e) => match e.local_name().as_ref() {
                b"entry" if !in_entry => {
                    in_entry = true;
                    target = TextTarget::None;
                    cur = Some(FeedEntry {
                        video_id: String::new(),
                        title: String::new(),
                        published: String::new(),
                        description: None,
                        thumbnail: None,
                        kind,
                        duration: None,
                        view_count: None,
                    });
                }
                b"group" if in_entry => in_media_group = true,
                b"title" if in_entry && !in_media_group => target = TextTarget::EntryTitle,
                b"title" if !in_entry => target = TextTarget::FeedTitle,
                b"videoId" if in_entry => target = TextTarget::VideoId,
                b"published" if in_entry => target = TextTarget::Published,
                b"description" if in_media_group => target = TextTarget::Description,
                _ => {}
            },
            Event::Empty(e)
                if in_media_group && e.local_name().as_ref() == b"thumbnail" =>
            {
                // <media:thumbnail url="..." />（自闭合）
                if let Some(entry) = cur.as_mut() {
                    for attr in e.attributes().flatten() {
                        if attr.key.local_name().as_ref() == b"url" {
                            if let Ok(v) = attr.unescape_value() {
                                entry.thumbnail = Some(v.into_owned());
                            }
                        }
                    }
                }
            }
            Event::Text(t) => {
                let decoded = t
                    .decode()
                    .map_err(|e| format!("err.rss_parse|{}", e))?;
                let text = quick_xml::escape::unescape(&decoded)
                    .map_err(|e| format!("err.rss_parse|{}", e))?;
                apply_text_content(&mut channel_title, &mut cur, target, &text);
            }
            Event::GeneralRef(t) => {
                // GeneralRef 内容为实体名（已去掉 `&` 与 `;`），补全后解义
                let decoded = t
                    .decode()
                    .map_err(|e| format!("err.rss_parse|{}", e))?;
                let text = unescape_text(&decoded, true)?;
                apply_text_content(&mut channel_title, &mut cur, target, &text);
            }
            Event::End(e) => match e.local_name().as_ref() {
                b"entry" if in_entry => {
                    if let Some(mut entry) = cur.take() {
                        entry.video_id = entry.video_id.trim().to_string();
                        entry.published = entry.published.trim().to_string();
                        entry.title = entry.title.trim().to_string();
                        if let Some(d) = entry.description.as_mut() {
                            let trimmed = d.trim().to_string();
                            if trimmed.is_empty() {
                                entry.description = None;
                            } else {
                                *d = trimmed;
                            }
                        }
                        if !entry.video_id.is_empty() && !entry.published.is_empty() {
                            entries.push(entry);
                        }
                    }
                    in_entry = false;
                    target = TextTarget::None;
                }
                b"group" if in_media_group => in_media_group = false,
                b"title" | b"videoId" | b"published" | b"description" => {
                    target = TextTarget::None;
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    Ok(FeedParseResult {
        channel_title,
        entries,
    })
}

/// 构造按内容类型过滤的 RSS feed URL（playlist_id 前缀技巧）。
fn feed_url(channel_id: &str, kind: FeedKind) -> Result<String, (u16, String)> {
    let prefix = kind.playlist_prefix();
    let tail = channel_id
        .strip_prefix("UC")
        .ok_or((0, format!("err.youtube_invalid_channel|{}", channel_id)))?;
    Ok(format!(
        "{}?playlist_id={}{}",
        YT_FEED_BASE, prefix, tail
    ))
}

/// 从用户输入提取 channel_id（UC 开头）。不触发网络请求。
fn extract_channel_id_from_input(input: &str) -> Option<String> {
    let input = input.trim();
    // 已是 channel_id
    if input.starts_with("UC") && input.len() >= 12 {
        let end = input
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-')
            .unwrap_or(input.len());
        let id = &input[..end];
        if id.len() >= 12 {
            return Some(id.to_string());
        }
    }
    // https://www.youtube.com/channel/UCxxx / youtube.com/channel/UCxxx
    if let Some(idx) = input.find("channel/") {
        let rest = &input[idx + "channel/".len()..];
        let end = rest
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-')
            .unwrap_or(rest.len());
        let id = &rest[..end];
        if id.starts_with("UC") && id.len() >= 12 {
            return Some(id.to_string());
        }
    }
    None
}

/// 构造频道页 URL（@handle / c/ / user/ / 纯文本 → 可请求的页面地址）。
///
/// 安全约束：`http(s)://` 输入只允许 youtube.com 域（host 白名单），
/// 防止 IPC 传入的 owner 字符串驱动任意域名出网（SSRF）。
fn channel_page_url(input: &str) -> Result<String, (u16, String)> {
    let input = input.trim();
    if input.starts_with("http://") || input.starts_with("https://") {
        let after_scheme = &input[input.find("://").unwrap() + 3..];
        let host = after_scheme
            .split(['/', '?', '#'])
            .next()
            .unwrap_or("")
            .split(':')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        if !matches!(host.as_str(), "youtube.com" | "www.youtube.com" | "m.youtube.com") {
            return Err((400, "err.youtube_invalid_channel|non-youtube-host".to_string()));
        }
        return Ok(input.to_string());
    }
    Ok(format!(
        "https://www.youtube.com/@{}",
        input.trim_start_matches('@')
    ))
}

/// 从频道页 HTML 提取 channel_id。
///
/// 依次尝试三种标记：
/// 1. `"channelId":"UC..."`（ytInitialData / ytcfg JSON）
/// 2. `<meta itemprop="channelId" content="UC...">`
/// 3. `<link rel="canonical" href="https://www.youtube.com/channel/UC...">`
fn extract_channel_id_from_html(html: &str) -> Option<String> {
    // 1) "channelId":"UC..."
    if let Some(idx) = html.find("\"channelId\"") {
        let after = &html[idx + "\"channelId\"".len()..];
        if let Some(colon) = after.find(':') {
            let val = &after[colon + 1..];
            let val = val.trim_start_matches([' ', '"']);
            if val.starts_with("UC") {
                let end = val
                    .find(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-')
                    .unwrap_or(val.len());
                let id = &val[..end];
                if id.len() >= 12 {
                    return Some(id.to_string());
                }
            }
        }
    }
    // 2) <meta itemprop="channelId" content="UC...">
    if let Some(idx) = html.find("itemprop=\"channelId\"") {
        let rest = &html[idx..];
        if let Some(cid) = rest.find("content=\"") {
            let start = idx + cid + "content=\"".len();
            let end = html[start..].find('"')?;
            let id = &html[start..start + end];
            if id.starts_with("UC") && id.len() >= 12 {
                return Some(id.to_string());
            }
        }
    }
    // 3) canonical link
    if let Some(idx) = html.find("rel=\"canonical\"") {
        let rest = &html[idx..];
        if let Some(href) = rest.find("href=\"") {
            let start = idx + href + "href=\"".len();
            let end = html[start..].find('"')?;
            let url = &html[start..start + end];
            return extract_channel_id_from_input(url);
        }
    }
    None
}

/// 验证频道可达并返回真实频道名。
///
/// 优先级：频道页 `<meta property="og:title">`（真实频道名，如“时局眼”）；
/// 页面失败时回退 UULF feed 的标题（注意：RSS 标题是播放列表名如 "Videos"，
/// 仅作兜底，正常情况下不会用到）。
async fn verify_and_describe_channel(
    client: &reqwest::Client,
    channel_id: &str,
) -> Result<String, (u16, String)> {
    let page_url = format!("https://www.youtube.com/channel/{}", channel_id);
    if let Ok(html) = fetch_text_with_retry(client, &page_url).await {
        if let Some(name) = extract_og_title(&html) {
            return Ok(name);
        }
    }
    // 兜底：UULF feed 的标题（播放列表名）
    let url = feed_url(channel_id, FeedKind::Video)?;
    let xml = fetch_feed_with_retry(client, &url).await?;
    let parsed =
        parse_feed(&xml, FeedKind::Video).map_err(|e| (0, format!("err.parse_failed|{}", e)))?;
    if parsed.channel_title.is_empty() {
        return Err((404, format!("err.youtube_channel_not_found|{}", channel_id)));
    }
    Ok(parsed.channel_title)
}

/// 从 HTML 提取 og:title（频道页的真实频道名）。
fn extract_og_title(html: &str) -> Option<String> {
    let needle = "<meta property=\"og:title\" content=\"";
    let idx = html.find(needle)?;
    let rest = &html[idx + needle.len()..];
    let end = rest.find('"')?;
    let raw = &rest[..end];
    if raw.is_empty() {
        return None;
    }
    quick_xml::escape::unescape(raw).ok().map(|s| s.into_owned())
}

/// 拉取 URL 原始文本（带重试，仅限 HTML 页面等文本内容）。
async fn fetch_text_with_retry(
    client: &reqwest::Client,
    url: &str,
) -> Result<String, (u16, String)> {
    crate::retry::retry_with_backoff(
        &Default::default(),
        |e: &(u16, String)| {
            if e.0 == 403 {
                return false;
            }
            log::warn!("请求失败(状态={}), 将重试: {}", e.0, e.1);
            true
        },
        || async {
            let resp = client
                .get(url)
                .send()
                .await
                .map_err(|e| (0, format!("err.request_failed|{}", e)))?;
            let status = resp.status().as_u16();
            if !resp.status().is_success() {
                let reason = resp.status().canonical_reason().unwrap_or("").to_string();
                return Err((status, format!("err.api_error|{}|{}", status, reason)));
            }
            resp.text()
                .await
                .map_err(|e| (0, format!("err.parse_failed|{}", e)))
        },
    )
    .await
}

/// 拉取单个 feed 的原始 XML（带重试）。
async fn fetch_feed_raw(
    client: &reqwest::Client,
    url: &str,
) -> Result<String, (u16, String)> {
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| (0, format!("err.request_failed|{}", e)))?;
    let status = resp.status().as_u16();
    if !resp.status().is_success() {
        let reason = resp.status().canonical_reason().unwrap_or("").to_string();
        return Err((status, format!("err.api_error|{}|{}", status, reason)));
    }
    resp.text()
        .await
        .map_err(|e| (0, format!("err.parse_failed|{}", e)))
}

async fn fetch_feed_with_retry(
    client: &reqwest::Client,
    url: &str,
) -> Result<String, (u16, String)> {
    crate::retry::retry_with_backoff(
        &Default::default(),
        |e: &(u16, String)| {
            if e.0 == 403 {
                return false;
            }
            log::warn!("请求失败(状态={}), 将重试: {}", e.0, e.1);
            true
        },
        || fetch_feed_raw(client, url),
    )
    .await
}

/// 拉取单个 feed 并解析条目（不含 per_page 截断）。
async fn fetch_feed_entries(
    client: &reqwest::Client,
    channel_id: &str,
    kind: FeedKind,
) -> Result<Vec<FeedEntry>, (u16, String)> {
    let url = feed_url(channel_id, kind)?;
    let xml = fetch_feed_with_retry(client, &url).await?;
    let parsed =
        parse_feed(&xml, kind).map_err(|e| (0, format!("err.parse_failed|{}", e)))?;
    Ok(parsed.entries)
}

/// 聚合多个内容类型的拉取结果。
///
/// - 404：该内容类型无条目（如频道没有直播时 UULV feed 不存在），跳过不阻断；
/// - 其它错误：记录最后一个，若最终无任何条目则上报；
/// - 全部 404/空：视为无内容，返回空列表。
async fn aggregate_kinds<F, Fut>(
    kinds: &[FeedKind],
    mut fetch_one: F,
) -> Result<Vec<FeedEntry>, (u16, String)>
where
    F: FnMut(FeedKind) -> Fut,
    Fut: std::future::Future<Output = Result<Vec<FeedEntry>, (u16, String)>>,
{
    let mut out = Vec::new();
    let mut last_err: Option<(u16, String)> = None;
    for &kind in kinds {
        match fetch_one(kind).await {
            Ok(entries) => out.extend(entries),
            Err((404, _)) => {
                log::warn!(
                    "YouTube {} feed 不存在（频道可能没有该类型内容）",
                    kind.as_str()
                );
            }
            Err(e) => last_err = Some(e),
        }
    }
    if out.is_empty() {
        if let Some(e) = last_err {
            return Err(e);
        }
    }
    Ok(out)
}

// ── Data API v3 模式（配置 youtube_api_key 后启用）──────────────────

/// 请求 YouTube Data API 并解析 JSON。
/// 非 2xx 时把 `{"error":{...}}` 映射为 i18n 错误（区分 key 无效 / 配额用尽）。
async fn api_get_json(
    client: &reqwest::Client,
    url: &str,
) -> Result<serde_json::Value, (u16, String)> {
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| (0, format!("err.request_failed|{}", e)))?;
    let status = resp.status().as_u16();
    let text = resp
        .text()
        .await
        .map_err(|e| (0, format!("err.parse_failed|{}", e)))?;
    if !(200..300).contains(&status) {
        return Err(map_api_error(status, &text));
    }
    serde_json::from_str(&text).map_err(|e| (0, format!("err.parse_failed|{}", e)))
}

/// 把 Data API 错误响应映射为 i18n 错误串。
/// - `keyInvalid` / `referrerNotAllowed` / `ipRefererBlocked` → key 配置问题
/// - `quotaExceeded` / `dailyLimitExceeded` → 配额用尽
fn map_api_error(status: u16, body: &str) -> (u16, String) {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(body) {
        if let Some(err) = v.get("error") {
            let reason = err
                .get("errors")
                .and_then(|e| e.get(0))
                .and_then(|e| e.get("reason"))
                .and_then(|r| r.as_str());
            if let Some(r) = reason {
                if r.contains("keyInvalid")
                    || r.contains("referrerNotAllowed")
                    || r.contains("ipRefererBlocked")
                {
                    return (status, format!("err.youtube_api_key_invalid|{}", r));
                }
                if r.contains("quotaExceeded") || r.contains("dailyLimitExceeded") {
                    return (status, format!("err.youtube_api_quota|{}", r));
                }
            }
            let msg = err
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("")
                .to_string();
            return (status, format!("err.api_error|{}|{}", status, msg));
        }
    }
    (status, format!("err.api_error|{}|{}", status, body))
}

/// 带重试的 API GET。key/配额类错误（401/403/400）不重试，其余可重试。
async fn api_get_json_with_retry(
    client: &reqwest::Client,
    url: &str,
) -> Result<serde_json::Value, (u16, String)> {
    crate::retry::retry_with_backoff(
        &Default::default(),
        |e: &(u16, String)| {
            if matches!(e.0, 400 | 401 | 403) {
                return false;
            }
            log::warn!("YouTube API 请求失败(状态={}), 将重试: {}", e.0, e.1);
            true
        },
        || api_get_json(client, url),
    )
    .await
}

/// 从用户输入提取 @handle（供 Data API `forHandle` 参数使用）。
///
/// 仅接受 youtube.com 域（host 白名单与 `channel_page_url` 一致，防 SSRF）；
/// `channel/UC...` 等由 `extract_channel_id_from_input` 提前处理，`c/` / `user/`
/// 形式 API 不支持，返回 `None` 让调用方回退 HTML 解析。
fn extract_handle_from_input(input: &str) -> Result<Option<String>, (u16, String)> {
    let input = input.trim();
    if input.starts_with("http://") || input.starts_with("https://") {
        let after_scheme = &input[input.find("://").unwrap() + 3..];
        let host = after_scheme
            .split(['/', '?', '#'])
            .next()
            .unwrap_or("")
            .split(':')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        if !matches!(
            host.as_str(),
            "youtube.com" | "www.youtube.com" | "m.youtube.com"
        ) {
            return Err((400, "err.youtube_invalid_channel|non-youtube-host".to_string()));
        }
        let rest = after_scheme.split_once('/').map(|(_, r)| r).unwrap_or("");
        if let Some(handle) = rest.strip_prefix('@') {
            let handle = handle.split(['/', '?', '#']).next().unwrap_or("");
            if !handle.is_empty() {
                return Ok(Some(handle.to_string()));
            }
        }
        return Ok(None);
    }
    if let Some(handle) = input.strip_prefix('@') {
        if !handle.is_empty() {
            return Ok(Some(handle.to_string()));
        }
    }
    if !input.is_empty() {
        // 纯文本当 handle 处理（与 channel_page_url 行为一致）
        return Ok(Some(input.to_string()));
    }
    Ok(None)
}

/// Data API 模式解析 channel_id：
/// - 直接 UC id / channel/ URL → 无需 API；
/// - @handle / 纯文本 → `channels.list?forHandle=`；
/// - c/ user/ 等形式 API 不支持 → 返回错误由调用方回退 HTML 解析。
async fn resolve_channel_id_via_api(
    client: &reqwest::Client,
    input: &str,
    api_key: &str,
    api_base: &str,
) -> Result<String, (u16, String)> {
    if let Some(id) = extract_channel_id_from_input(input) {
        return Ok(id);
    }
    let Some(handle) = extract_handle_from_input(input)? else {
        return Err((400, format!("err.youtube_invalid_channel|{}", input)));
    };
    let url = format!(
        "{}/channels?part=id&forHandle={}&key={}",
        api_base, handle, api_key
    );
    let data = api_get_json_with_retry(client, &url).await?;
    let id = data
        .get("items")
        .and_then(|items| items.get(0))
        .and_then(|it| it.get("id"))
        .and_then(|id| id.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    id.ok_or((404, format!("err.youtube_channel_not_found|{}", input)))
}

/// Data API 模式验证频道并返回真实频道名（channels.list snippet.title）。
async fn verify_and_describe_channel_via_api(
    client: &reqwest::Client,
    channel_id: &str,
    api_key: &str,
    api_base: &str,
) -> Result<String, (u16, String)> {
    let url = format!(
        "{}/channels?part=snippet&id={}&key={}",
        api_base, channel_id, api_key
    );
    let data = api_get_json_with_retry(client, &url).await?;
    let title = data
        .get("items")
        .and_then(|items| items.get(0))
        .and_then(|it| it.get("snippet"))
        .and_then(|s| s.get("title"))
        .and_then(|t| t.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    title.ok_or((404, format!("err.youtube_channel_not_found|{}", channel_id)))
}

/// 取频道 uploads 播放列表 id（Data API `channels.list` contentDetails）。
async fn api_get_uploads_playlist_id(
    client: &reqwest::Client,
    channel_id: &str,
    api_key: &str,
    api_base: &str,
) -> Result<String, (u16, String)> {
    let url = format!(
        "{}/channels?part=contentDetails&id={}&key={}",
        api_base, channel_id, api_key
    );
    let data = api_get_json_with_retry(client, &url).await?;
    let uploads = data
        .get("items")
        .and_then(|items| items.get(0))
        .and_then(|it| it.get("contentDetails"))
        .and_then(|c| c.get("relatedPlaylists"))
        .and_then(|r| r.get("uploads"))
        .and_then(|u| u.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    uploads.ok_or((404, format!("err.youtube_channel_not_found|{}", channel_id)))
}

/// 拉取一页 playlistItems，返回 (条目, 下一页 token)。
/// 条目结构：`snippet.publishedAt/title/description/thumbnails/resourceId.videoId`。
async fn api_get_playlist_items_page(
    client: &reqwest::Client,
    playlist_id: &str,
    api_key: &str,
    page_token: Option<&str>,
    api_base: &str,
) -> Result<(Vec<serde_json::Value>, Option<String>), (u16, String)> {
    let mut url = format!(
        "{}/playlistItems?part=snippet&playlistId={}&maxResults={}&key={}",
        api_base, playlist_id, YT_API_PAGE_SIZE, api_key
    );
    if let Some(tok) = page_token {
        url.push_str(&format!("&pageToken={}", tok));
    }
    let data = api_get_json_with_retry(client, &url).await?;
    let items = data
        .get("items")
        .and_then(|i| i.as_array())
        .cloned()
        .unwrap_or_default();
    let next = data
        .get("nextPageToken")
        .and_then(|t| t.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    Ok((items, next))
}

/// 把 playlistItems 条目转为 FeedEntry（kind 由调用方决定）。
fn entry_from_api_item(item: &serde_json::Value, kind: FeedKind) -> Option<FeedEntry> {
    let snippet = item.get("snippet")?;
    let video_id = snippet
        .get("resourceId")?
        .get("videoId")?
        .as_str()?
        .to_string();
    let title = snippet
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let published = snippet
        .get("publishedAt")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if video_id.is_empty() || published.is_empty() {
        return None;
    }
    let description = snippet
        .get("description")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let thumbnail = snippet
        .get("thumbnails")
        .and_then(|t| {
            t.get("medium")
                .or_else(|| t.get("high"))
                .or_else(|| t.get("default"))
        })
        .and_then(|t| t.get("url"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    Some(FeedEntry {
        video_id,
        title,
        published,
        description,
        thumbnail,
        kind,
        duration: None,
        view_count: None,
    })
}

/// Data API 视频详情（videos.list 返回）。
#[derive(Debug, Clone, Default)]
struct VideoDetail {
    /// `liveBroadcastContent`：none / upcoming / live / completed
    live: Option<String>,
    /// `contentDetails.duration`（ISO 8601，如 `PT1H2M3S`）
    duration: Option<String>,
    /// `statistics.viewCount`（API 返回字符串，如 "123456"）
    view_count: Option<i64>,
}

/// 批量查询视频详情（videos.list `part=snippet,contentDetails,statistics`，每请求最多 50 个 id）。
/// 返回 `video_id → VideoDetail`。
async fn api_get_videos_details(
    client: &reqwest::Client,
    video_ids: &[String],
    api_key: &str,
    api_base: &str,
) -> Result<std::collections::HashMap<String, VideoDetail>, (u16, String)> {
    let mut map = std::collections::HashMap::new();
    for chunk in video_ids.chunks(YT_API_PAGE_SIZE) {
        let url = format!(
            "{}/videos?part=snippet,contentDetails,statistics&id={}&key={}",
            api_base,
            chunk.join(","),
            api_key
        );
        let data = api_get_json_with_retry(client, &url).await?;
        if let Some(items) = data.get("items").and_then(|i| i.as_array()) {
            for it in items {
                let Some(id) = it.get("id").and_then(|v| v.as_str()) else {
                    continue;
                };
                let mut detail = VideoDetail::default();
                if let Some(s) = it.get("snippet") {
                    detail.live = s
                        .get("liveBroadcastContent")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                }
                if let Some(cd) = it.get("contentDetails") {
                    detail.duration = cd
                        .get("duration")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                        .map(|s| s.to_string());
                }
                if let Some(st) = it.get("statistics") {
                    detail.view_count = st.get("viewCount").and_then(|v| {
                        // API 返回字符串；数字形态（测试/异常）也兼容
                        v.as_str()
                            .and_then(|s| s.parse::<i64>().ok())
                            .or_else(|| v.as_i64())
                    }).filter(|&n| n > 0);
                }
                map.insert(id.to_string(), detail);
            }
        }
    }
    Ok(map)
}

/// Data API 模式拉取频道最新内容。
///
/// 流程：channels.list（uploads 播放列表）→ playlistItems.list（翻页，最多 50/页）
/// → 仅当订阅类型只勾一种时用 videos.list 按 liveBroadcastContent 过滤。
/// 两种都勾（默认）时不做分类、不额外消耗配额。
async fn fetch_via_api(
    client: &reqwest::Client,
    source: &Source,
    api_key: &str,
    max_count: Option<usize>,
    api_base: &str,
) -> Result<Vec<FeedEntry>, (u16, String)> {
    let config = SubscribeConfig::from_source(source);
    let kinds = config.kinds();
    if kinds.is_empty() {
        return Ok(vec![]);
    }
    let need_classify = kinds.len() == 1;

    let uploads = api_get_uploads_playlist_id(client, &source.owner, api_key, api_base).await?;

    let mut entries = Vec::new();
    let mut page_token: Option<String> = None;
    loop {
        let (items, next) = api_get_playlist_items_page(
            client,
            &uploads,
            api_key,
            page_token.as_deref(),
            api_base,
        )
        .await?;
        for item in &items {
            if let Some(e) = entry_from_api_item(item, FeedKind::Video) {
                entries.push(e);
            }
        }
        if let Some(limit) = max_count {
            if entries.len() >= limit {
                break;
            }
        }
        match next {
            Some(tok) => page_token = Some(tok),
            None => break,
        }
    }

    if need_classify {
        let ids: Vec<String> = entries.iter().map(|e| e.video_id.clone()).collect();
        let details = api_get_videos_details(client, &ids, api_key, api_base).await?;
        let want_videos = config.videos;
        entries.retain(|e| {
            let live = details
                .get(&e.video_id)
                .and_then(|d| d.live.as_deref())
                .map(|s| s != "none")
                .unwrap_or(false);
            if want_videos {
                !live
            } else {
                live
            }
        });
        for e in entries.iter_mut() {
            if let Some(d) = details.get(&e.video_id) {
                if let Some(dur) = &d.duration {
                    e.duration = Some(dur.clone());
                }
                if let Some(vc) = d.view_count {
                    e.view_count = Some(vc);
                }
                if d.live.as_deref().map(|s| s != "none").unwrap_or(false) {
                    e.kind = FeedKind::Live;
                }
            }
        }
    } else {
        // 两种类型都勾选（默认）：仍拉 videos.list 补全时长（+ 精确标注类型 + 播放量），
        // 每 50 个视频额外 1 unit 配额。
        let ids: Vec<String> = entries.iter().map(|e| e.video_id.clone()).collect();
        let details = api_get_videos_details(client, &ids, api_key, api_base).await?;
        for e in entries.iter_mut() {
            if let Some(d) = details.get(&e.video_id) {
                if let Some(dur) = &d.duration {
                    e.duration = Some(dur.clone());
                }
                if let Some(vc) = d.view_count {
                    e.view_count = Some(vc);
                }
                if d.live.as_deref().map(|s| s != "none").unwrap_or(false) {
                    e.kind = FeedKind::Live;
                }
            }
        }
    }

    if let Some(limit) = max_count {
        entries.truncate(limit);
    }
    Ok(entries)
}

/// 条目列表 → 序列化 JSON（供 save 阶段反序列化 FeedEntry）。
fn entries_to_json(entries: Vec<FeedEntry>) -> Vec<serde_json::Value> {
    entries
        .into_iter()
        .map(|e| serde_json::to_value(e).unwrap_or(serde_json::Value::Null))
        .collect()
}

/// 解析用户输入为 channel_id（可能触发一次频道页请求）。
///
/// 支持的输入：
/// - `UC...`（已是 channel_id）
/// - `https://www.youtube.com/channel/UC...`
/// - `@handle` / `https://www.youtube.com/@handle` / `youtube.com/@handle`
/// - `https://www.youtube.com/c/name` / `/user/name`
pub async fn resolve_channel_id(
    client: &reqwest::Client,
    input: &str,
) -> Result<String, (u16, String)> {
    if let Some(id) = extract_channel_id_from_input(input) {
        return Ok(id);
    }
    // 其余形式：请求频道页提取 channelId（channel_page_url 已做 youtube.com host 白名单）
    let page_url = channel_page_url(input)?;
    let resp = client
        .get(&page_url)
        .send()
        .await
        .map_err(|e| (0, format!("err.request_failed|{}", e)))?;
    let status = resp.status().as_u16();
    if !resp.status().is_success() {
        let reason = resp.status().canonical_reason().unwrap_or("").to_string();
        return Err((status, format!("err.api_error|{}|{}", status, reason)));
    }
    let html = resp
        .text()
        .await
        .map_err(|e| (0, format!("err.parse_failed|{}", e)))?;
    extract_channel_id_from_html(&html)
        .ok_or((404, format!("err.youtube_channel_not_found|{}", input)))
}

// ── SourceAdapter 实现 ─────────────────────────────────

/// YouTube 监控源适配器。实现 `SourceAdapter` trait。
pub struct YoutubeAdapter;

#[async_trait::async_trait]
impl SourceAdapter for YoutubeAdapter {
    fn source_type(&self) -> &'static str {
        "youtube"
    }

    fn auth_kind(&self) -> crate::source::AuthKind {
        crate::source::AuthKind::YouTubeApiKey
    }

    /// YouTube：每次检查都按 fetch_history_count 拉历史（配 key 后无需删源即可补拉）。
    fn always_fetch_history(&self) -> bool {
        true
    }

    /// YouTube 视频不生成 AI 摘要/翻译（用户明确要求，与 filter_ai_eligible 对应）。
    fn ai_eligible(&self) -> bool {
        false
    }

    /// 手动检查后刷新真实频道名（RSS 标题是播放列表名，频道页 og:title 才是真名）。
    fn refresh_description_after_check(&self) -> bool {
        true
    }

    async fn fetch(
        &self,
        client: &reqwest::Client,
        source: &Source,
        per_page: usize,
        token: Option<&str>,
    ) -> Result<Vec<serde_json::Value>, (u16, String)> {
        // 配置了 Data API Key 时走 API 模式（规避 RSS 风控）；否则降级 RSS。
        if let Some(key) = token.filter(|k| !k.is_empty()) {
            let entries = fetch_via_api(client, source, key, Some(per_page), YT_API_BASE).await?;
            return Ok(entries_to_json(entries));
        }
        let kinds = SubscribeConfig::from_source(source).kinds();
        if kinds.is_empty() {
            return Ok(vec![]);
        }
        let entries =
            aggregate_kinds(&kinds, |kind| fetch_feed_entries(client, &source.owner, kind)).await?;
        let out: Vec<serde_json::Value> = entries
            .into_iter()
            .take(per_page)
            .map(|e| serde_json::to_value(e).unwrap_or(serde_json::Value::Null))
            .collect();
        Ok(out)
    }

    async fn fetch_all(
        &self,
        client: &reqwest::Client,
        source: &Source,
        max_count: Option<usize>,
        token: Option<&str>,
    ) -> Result<Vec<serde_json::Value>, (u16, String)> {
        // API 模式：支持翻页（uploads 列表可拉历史）；RSS 单页（约 15 条）无需翻页，
        // max_count 由 save 阶段截断。
        if let Some(key) = token.filter(|k| !k.is_empty()) {
            let entries = fetch_via_api(client, source, key, max_count, YT_API_BASE).await?;
            return Ok(entries_to_json(entries));
        }
        self.fetch(client, source, usize::MAX, token).await
    }

    async fn save(
        &self,
        db: &r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>,
        source: &Source,
        data: &[serde_json::Value],
        max_count: usize,
        _client: &reqwest::Client,
    ) -> Vec<(i64, Option<String>)> {
        // 同步入库，用 spawn_blocking 转包避免在 async 上下文阻塞
        // （与 github 适配器一致）。
        let db = db.clone();
        let source_id = source.id;
        let data = data.to_vec();
        tokio::task::spawn_blocking(move || {
            let conn = match db.get() {
                Ok(c) => c,
                Err(e) => {
                    log::error!("err.db_lock|{}", e);
                    return vec![];
                }
            };
            save_entries(&conn, source_id, &data, max_count)
        })
        .await
        .unwrap_or_else(|e| {
            log::error!("youtube save spawn_blocking panic: {}", e);
            vec![]
        })
    }

    async fn verify_and_describe(
        &self,
        client: &reqwest::Client,
        owner: &str,
        _repo: &str,
        token: Option<&str>,
    ) -> Result<String, (u16, String)> {
        // owner 此时已由 resolve_owner 归一化为 channel_id。
        // API 模式：channels.list snippet.title 即真实频道名；
        // RSS 模式：频道页 og:title（RSS 标题是播放列表名如 "Videos"，仅兜底）。
        if let Some(key) = token.filter(|k| !k.is_empty()) {
            return verify_and_describe_channel_via_api(client, owner, key, YT_API_BASE).await;
        }
        verify_and_describe_channel(client, owner).await
    }

    async fn resolve_owner(
        &self,
        client: &reqwest::Client,
        owner: &str,
        token: Option<&str>,
    ) -> Result<String, (u16, String)> {
        // API 模式优先（@handle → forHandle 查询，更稳定）；
        // API 处理不了的形式（c/ / user/ 链接）回退 HTML 解析。
        if let Some(key) = token.filter(|k| !k.is_empty()) {
            if let Ok(id) = resolve_channel_id_via_api(client, owner, key, YT_API_BASE).await {
                return Ok(id);
            }
        }
        resolve_channel_id(client, owner).await
    }
}

/// 保存 RSS 条目到 releases 表（tag_name = videoId，天然去重）。
///
/// 行为收敛到 `db::save::save_entries_generic`：按 published 降序排列，
/// `max_count=1` 时遇到已入库记录立即返回空；历史模式跳过已存在记录继续。
pub fn save_entries(
    conn: &Connection,
    source_id: i64,
    items: &[serde_json::Value],
    max_count: usize,
) -> Vec<(i64, Option<String>)> {
    let entries: Vec<crate::db::save::SaveEntry> = items
        .iter()
        .filter_map(|v| serde_json::from_value::<FeedEntry>(v.clone()).ok())
        .filter(|e| !e.video_id.is_empty() && !e.published.is_empty())
        .map(|e| {
            let html_url = e.html_url();
            let metadata = e.metadata_json();
            crate::db::save::SaveEntry {
                tag: e.video_id,
                name: e.title,
                html_url,
                published: e.published,
                prerelease: false,
                body: e.description,
                metadata: Some(metadata),
            }
        })
        .collect();
    crate::db::save::save_entries_generic(
        conn,
        source_id,
        &entries,
        max_count,
        // 插入成功：写入正文与元数据
        |conn, id, entry| {
            let _ = releases::set_release_body_and_metadata(
                conn,
                id,
                entry.body.as_deref(),
                entry.metadata.as_deref(),
            );
        },
        // 去重命中：刷新元数据（播放量/封面/时长随轮询更新）
        |conn, source_id, entry| {
            let _ = releases::update_release_metadata(
                conn,
                source_id,
                &entry.tag,
                entry.body.as_deref(),
                entry.metadata.as_deref(),
            );
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // ── Data API v3 模式 ──

    fn api_item(video_id: &str, title: &str, published: &str, desc: &str) -> serde_json::Value {
        serde_json::json!({
            "snippet": {
                "publishedAt": published,
                "title": title,
                "description": desc,
                "thumbnails": {"medium": {"url": format!("https://i.ytimg.com/vi/{}/mqdefault.jpg", video_id)}},
                "resourceId": {"videoId": video_id}
            }
        })
    }

    #[test]
    fn test_map_api_error_key_invalid() {
        let body = r#"{"error":{"code":403,"message":"denied","errors":[{"reason":"ipRefererBlocked"}]}}"#;
        let (status, msg) = map_api_error(403, body);
        assert_eq!(status, 403);
        assert!(msg.contains("err.youtube_api_key_invalid"), "{}", msg);
    }

    #[test]
    fn test_map_api_error_quota() {
        let body = r#"{"error":{"code":403,"message":"quota","errors":[{"reason":"quotaExceeded"}]}}"#;
        let (status, msg) = map_api_error(403, body);
        assert_eq!(status, 403);
        assert!(msg.contains("err.youtube_api_quota"));
    }

    #[test]
    fn test_map_api_error_plain() {
        let (status, msg) = map_api_error(500, "boom");
        assert_eq!(status, 500);
        assert!(msg.contains("err.api_error|500"));
    }

    #[test]
    fn test_extract_handle_from_input_variants() {
        assert_eq!(
            extract_handle_from_input("@handle").unwrap(),
            Some("handle".to_string())
        );
        assert_eq!(
            extract_handle_from_input("handle").unwrap(),
            Some("handle".to_string())
        );
        assert_eq!(
            extract_handle_from_input("https://www.youtube.com/@handle").unwrap(),
            Some("handle".to_string())
        );
        assert_eq!(
            extract_handle_from_input("https://m.youtube.com/@h?x=1").unwrap(),
            Some("h".to_string())
        );
        // channel/ c/ 等形式 → None（由调用方回退 HTML 解析）
        assert_eq!(
            extract_handle_from_input("https://www.youtube.com/channel/UCabc123").unwrap(),
            None
        );
        assert_eq!(
            extract_handle_from_input("https://www.youtube.com/c/name").unwrap(),
            None
        );
        // 非 youtube 域拒绝（SSRF 防护）
        assert!(extract_handle_from_input("https://evil.example.com/@x").is_err());
    }

    #[test]
    fn test_entry_from_api_item_basic() {
        let item = api_item("aaa111", "Hello", "2024-01-01T00:00:00Z", "desc");
        let e = entry_from_api_item(&item, FeedKind::Video).unwrap();
        assert_eq!(e.video_id, "aaa111");
        assert_eq!(e.title, "Hello");
        assert_eq!(e.published, "2024-01-01T00:00:00Z");
        assert_eq!(e.description.as_deref(), Some("desc"));
        assert_eq!(
            e.thumbnail.as_deref(),
            Some("https://i.ytimg.com/vi/aaa111/mqdefault.jpg")
        );
        assert_eq!(e.kind, FeedKind::Video);
    }

    #[test]
    fn test_entry_from_api_item_missing_id_skipped() {
        let item = serde_json::json!({"snippet": {"title": "no id"}});
        assert!(entry_from_api_item(&item, FeedKind::Video).is_none());
    }

    #[tokio::test]
    async fn test_resolve_channel_id_via_api_handle() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/channels"))
            .and(query_param("forHandle", "somehandle"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [{"id": "UCResolvedFromApi12345678"}]
            })))
            .mount(&mock)
            .await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let id = resolve_channel_id_via_api(&client, "@somehandle", "test-key", &mock.uri())
            .await
            .unwrap();
        assert_eq!(id, "UCResolvedFromApi12345678");
    }

    #[tokio::test]
    async fn test_resolve_channel_id_via_api_direct_id_no_http() {
        // 直接 UC id 无需 API 请求
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let id = resolve_channel_id_via_api(&client, "UCXuqSBlHAE6Xw-yeJA0Tunw", "k", "http://unused")
            .await
            .unwrap();
        assert_eq!(id, "UCXuqSBlHAE6Xw-yeJA0Tunw");
    }

    #[tokio::test]
    async fn test_verify_and_describe_channel_via_api() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/channels"))
            .and(query_param("id", "UCabc123"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [{"snippet": {"title": "Freesia"}}]
            })))
            .mount(&mock)
            .await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let title = verify_and_describe_channel_via_api(&client, "UCabc123", "k", &mock.uri())
            .await
            .unwrap();
        assert_eq!(title, "Freesia");
    }

    #[tokio::test]
    async fn test_verify_and_describe_channel_via_api_not_found() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/channels"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": []
            })))
            .mount(&mock)
            .await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let err = verify_and_describe_channel_via_api(&client, "UCabc123", "k", &mock.uri())
            .await
            .unwrap_err();
        assert_eq!(err.0, 404);
        assert!(err.1.contains("err.youtube_channel_not_found"));
    }

    #[tokio::test]
    async fn test_fetch_via_api_basic() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/channels"))
            .and(query_param("id", "UCabc123"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [{"contentDetails": {"relatedPlaylists": {"uploads": "UUabc123"}}}]
            })))
            .mount(&mock)
            .await;
        Mock::given(method("GET"))
            .and(path("/playlistItems"))
            .and(query_param("playlistId", "UUabc123"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [
                    api_item("v3", "Three", "2024-03-01T00:00:00Z", "d3"),
                    api_item("v2", "Two", "2024-02-01T00:00:00Z", "d2"),
                ]
            })))
            .mount(&mock)
            .await;
        // 默认 config（videos+live 都勾）：仍调 videos.list 补全时长
        Mock::given(method("GET"))
            .and(path("/videos"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [
                    {"id": "v3", "snippet": {"liveBroadcastContent": "none"}, "contentDetails": {"duration": "PT3M21S"}, "statistics": {"viewCount": "123456"}},
                    {"id": "v2", "snippet": {"liveBroadcastContent": "none"}, "contentDetails": {"duration": "PT1H2M3S"}, "statistics": {"viewCount": "987654"}},
                ]
            })))
            .mount(&mock)
            .await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        // 默认 config：videos+live 都勾 → 拉全部 + 补全时长
        let source = make_source(Some(r#"{"videos":true,"live":true,"posts":false}"#));
        let entries = fetch_via_api(&client, &source, "test-key", Some(10), &mock.uri())
            .await
            .unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].video_id, "v3");
        assert_eq!(entries[0].kind, FeedKind::Video);
        assert_eq!(entries[0].duration.as_deref(), Some("PT3M21S"));
        assert_eq!(entries[0].view_count, Some(123456));
        assert_eq!(entries[1].view_count, Some(987654));
        assert_eq!(entries[1].duration.as_deref(), Some("PT1H2M3S"));
        assert_eq!(
            entries[1].thumbnail.as_deref(),
            Some("https://i.ytimg.com/vi/v2/mqdefault.jpg")
        );
        let meta = serde_json::from_str::<serde_json::Value>(&entries[0].metadata_json()).unwrap();
        assert_eq!(meta["duration"], "PT3M21S");
        assert_eq!(meta["view_count"], 123456);
    }

    #[tokio::test]
    async fn test_fetch_via_api_classify_videos_only() {
        // 只勾视频：playlistItems 含一个直播回放 + 一个普通视频，
        // videos.list 按 liveBroadcastContent 过滤后只留普通视频。
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/channels"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [{"contentDetails": {"relatedPlaylists": {"uploads": "UUabc"}}}]
            })))
            .mount(&mock)
            .await;
        Mock::given(method("GET"))
            .and(path("/playlistItems"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [
                    api_item("live1", "L", "2024-01-01T00:00:00Z", ""),
                    api_item("vid1", "V", "2024-01-02T00:00:00Z", ""),
                ]
            })))
            .mount(&mock)
            .await;
        Mock::given(method("GET"))
            .and(path("/videos"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [
                    {"id": "live1", "snippet": {"liveBroadcastContent": "completed"}, "contentDetails": {"duration": "PT2H0M0S"}},
                    {"id": "vid1", "snippet": {"liveBroadcastContent": "none"}, "contentDetails": {"duration": "PT5M30S"}},
                ]
            })))
            .mount(&mock)
            .await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let source = make_source(Some(r#"{"videos":true,"live":false,"posts":false}"#));
        let entries = fetch_via_api(&client, &source, "k", Some(10), &mock.uri())
            .await
            .unwrap();
        assert_eq!(entries.len(), 1, "只勾视频时应过滤掉直播回放");
        assert_eq!(entries[0].video_id, "vid1");
        assert_eq!(entries[0].kind, FeedKind::Video);
        assert_eq!(entries[0].duration.as_deref(), Some("PT5M30S"));
    }

    #[tokio::test]
    async fn test_fetch_via_api_classify_live_only() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/channels"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [{"contentDetails": {"relatedPlaylists": {"uploads": "UUabc"}}}]
            })))
            .mount(&mock)
            .await;
        Mock::given(method("GET"))
            .and(path("/playlistItems"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [
                    api_item("live1", "L", "2024-01-01T00:00:00Z", ""),
                    api_item("vid1", "V", "2024-01-02T00:00:00Z", ""),
                ]
            })))
            .mount(&mock)
            .await;
        Mock::given(method("GET"))
            .and(path("/videos"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [
                    {"id": "live1", "snippet": {"liveBroadcastContent": "completed"}, "contentDetails": {"duration": "PT2H0M0S"}},
                    {"id": "vid1", "snippet": {"liveBroadcastContent": "none"}, "contentDetails": {"duration": "PT5M30S"}},
                ]
            })))
            .mount(&mock)
            .await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let source = make_source(Some(r#"{"videos":false,"live":true,"posts":false}"#));
        let entries = fetch_via_api(&client, &source, "k", Some(10), &mock.uri())
            .await
            .unwrap();
        assert_eq!(entries.len(), 1, "只勾直播时应只留直播回放");
        assert_eq!(entries[0].video_id, "live1");
        assert_eq!(entries[0].kind, FeedKind::Live);
        assert_eq!(entries[0].duration.as_deref(), Some("PT2H0M0S"));
    }

    // ── feed_url ──

    #[test]
    fn test_feed_url_video_prefix() {
        let url = feed_url("UCabcdefghijklmnopqrst", FeedKind::Video).unwrap();
        assert_eq!(
            url,
            "https://www.youtube.com/feeds/videos.xml?playlist_id=UULFabcdefghijklmnopqrst"
        );
    }

    #[test]
    fn test_feed_url_live_prefix() {
        let url = feed_url("UCabcdefghijklmnopqrst", FeedKind::Live).unwrap();
        assert_eq!(
            url,
            "https://www.youtube.com/feeds/videos.xml?playlist_id=UULVabcdefghijklmnopqrst"
        );
    }

    #[test]
    fn test_feed_url_invalid_channel() {
        let err = feed_url("XXabcdefghijklmnopqrst", FeedKind::Video).unwrap_err();
        assert_eq!(err.0, 0);
        assert!(err.1.contains("err.youtube_invalid_channel"));
    }

    // ── aggregate_kinds（404 宽容）──

    #[tokio::test]
    async fn test_aggregate_kinds_skips_404_kind() {
        let kinds = vec![FeedKind::Video, FeedKind::Live];
        let result = aggregate_kinds(&kinds, |kind| async move {
            match kind {
                FeedKind::Video => Ok(vec![FeedEntry {
                    video_id: "v1".into(),
                    title: "V".into(),
                    published: "2024-01-01T00:00:00+00:00".into(),
                    description: None,
                    thumbnail: None,
                    kind,
                    duration: None,
                    view_count: None,
                }]),
                // 直播 feed 404（频道无直播）→ 应跳过，不阻断
                FeedKind::Live => Err((404, "err.api_error|404|Not Found".into())),
            }
        })
        .await;
        assert!(result.is_ok());
        let entries = result.unwrap();
        assert_eq!(entries.len(), 1, "404 类型应被跳过，保留视频条目");
        assert_eq!(entries[0].video_id, "v1");
    }

    #[tokio::test]
    async fn test_aggregate_kinds_all_404_returns_empty() {
        let kinds = vec![FeedKind::Video, FeedKind::Live];
        let result = aggregate_kinds(&kinds, |_| async { Err((404, "not found".into())) }).await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty(), "全部 404 视为无内容，不报错");
    }

    #[tokio::test]
    async fn test_aggregate_kinds_propagates_real_error() {
        let kinds = vec![FeedKind::Video, FeedKind::Live];
        let result = aggregate_kinds(&kinds, |kind| async move {
            match kind {
                FeedKind::Video => Err((500, "err.api_error|500|Internal Server Error".into())),
                FeedKind::Live => Err((404, "not found".into())),
            }
        })
        .await;
        assert!(result.is_err(), "存在真实错误时即使有 404 也应上报");
        assert_eq!(result.unwrap_err().0, 500);
    }

    // ── SubscribeConfig ──

    fn make_source(config: Option<&str>) -> Source {
        Source {
            id: 1,
            source_type: "youtube".into(),
            owner: "UCabc123".into(),
            repo: String::new(),
            poll_interval_minutes: 30,
            enabled: true,
            last_checked_at: None,
            last_check_status: "unknown".into(),
            last_check_message: None,
            consecutive_failures: 0,
            last_new_count: 0,
            muted: false,
            created_at: String::new(),
            updated_at: String::new(),
            description: None,
            config: config.map(|s| s.to_string()),
        }
    }

    #[test]
    fn test_subscribe_config_default() {
        let s = make_source(None);
        let cfg = SubscribeConfig::from_source(&s);
        assert!(cfg.videos);
        assert!(cfg.live);
        assert!(!cfg.posts);
        assert_eq!(cfg.kinds(), vec![FeedKind::Video, FeedKind::Live]);
    }

    #[test]
    fn test_subscribe_config_partial() {
        let s = make_source(Some(r#"{"videos":true,"live":false,"posts":false}"#));
        let cfg = SubscribeConfig::from_source(&s);
        assert!(cfg.videos);
        assert!(!cfg.live);
        assert_eq!(cfg.kinds(), vec![FeedKind::Video]);
    }

    #[test]
    fn test_subscribe_config_none_subscribed() {
        let s = make_source(Some(r#"{"videos":false,"live":false,"posts":false}"#));
        assert!(SubscribeConfig::from_source(&s).kinds().is_empty());
    }

    #[test]
    fn test_subscribe_config_invalid_json_falls_back() {
        let s = make_source(Some("not-json"));
        let cfg = SubscribeConfig::from_source(&s);
        assert!(cfg.videos && cfg.live);
    }

    #[test]
    fn test_subscribe_config_missing_fields_default() {
        let s = make_source(Some(r#"{"posts":true}"#));
        let cfg = SubscribeConfig::from_source(&s);
        assert!(cfg.videos, "缺省字段应回退默认值");
        assert!(cfg.live);
        assert!(cfg.posts, "显式声明的字段应生效");
    }

    // ── parse_feed ──

    fn sample_feed(videos: &[(&str, &str, &str, Option<&str>)]) -> String {
        let mut xml = String::from(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns:yt="http://www.youtube.com/xml/schemas/2015" xmlns:media="http://search.yahoo.com/mrss/" xmlns="http://www.w3.org/2005/Atom">
  <title>Test Channel</title>
  <link rel="self" href="https://www.youtube.com/feeds/videos.xml?channel_id=UCabc"/>
"#,
        );
        for (id, title, published, desc) in videos {
            xml.push_str(&format!(
                r#"  <entry>
    <id>yt:video:{}</id>
    <yt:videoId>{}</yt:videoId>
    <yt:channelId>UCabc</yt:channelId>
    <title>{}</title>
    <link rel="alternate" href="https://www.youtube.com/watch?v={}"/>
    <published>{}</published>
    <media:group>
      <media:title>{}</media:title>
      <media:thumbnail url="https://i.ytimg.com/vi/{}/hqdefault.jpg" width="480" height="360"/>
      <media:description>{}</media:description>
    </media:group>
  </entry>
"#,
                id,
                id,
                title,
                id,
                published,
                title,
                id,
                desc.unwrap_or(""),
            ));
        }
        xml.push_str("</feed>");
        xml
    }

    #[test]
    fn test_parse_feed_basic() {
        let xml = sample_feed(&[("aaa111", "Hello World", "2024-01-01T00:00:00+00:00", Some("视频描述"))]);
        let parsed = parse_feed(&xml, FeedKind::Video).unwrap();
        assert_eq!(parsed.channel_title, "Test Channel");
        assert_eq!(parsed.entries.len(), 1);
        let e = &parsed.entries[0];
        assert_eq!(e.video_id, "aaa111");
        assert_eq!(e.title, "Hello World");
        assert_eq!(e.published, "2024-01-01T00:00:00+00:00");
        assert_eq!(e.description.as_deref(), Some("视频描述"));
        assert_eq!(
            e.thumbnail.as_deref(),
            Some("https://i.ytimg.com/vi/aaa111/hqdefault.jpg")
        );
        assert_eq!(e.kind, FeedKind::Video);
        assert_eq!(e.html_url(), "https://www.youtube.com/watch/?v=aaa111");
        let meta = serde_json::from_str::<serde_json::Value>(&e.metadata_json()).unwrap();
        assert_eq!(meta["kind"], "video");
        assert_eq!(meta["thumbnail"], "https://i.ytimg.com/vi/aaa111/hqdefault.jpg");
    }

    #[test]
    fn test_parse_feed_multiple_entries_and_live_kind() {
        let xml = sample_feed(&[
            ("vid2", "Second", "2024-01-02T00:00:00+00:00", Some("b")),
            ("vid1", "First", "2024-01-01T00:00:00+00:00", None),
        ]);
        let parsed = parse_feed(&xml, FeedKind::Live).unwrap();
        assert_eq!(parsed.entries.len(), 2);
        // feed 顺序保持（save 阶段再排序）
        assert_eq!(parsed.entries[0].video_id, "vid2");
        assert_eq!(parsed.entries[1].video_id, "vid1");
        // 无描述时为空
        assert!(parsed.entries[1].description.is_none());
        // 直播 kind 标记
        assert_eq!(parsed.entries[0].kind, FeedKind::Live);
        let meta = serde_json::from_str::<serde_json::Value>(&parsed.entries[0].metadata_json()).unwrap();
        assert_eq!(meta["kind"], "live");
    }

    #[test]
    fn test_parse_feed_unescapes_entities() {
        let xml = sample_feed(&[("v1", "A &amp; B &lt;tag&gt;", "2024-01-01T00:00:00+00:00", Some("x &amp; y"))]);
        let parsed = parse_feed(&xml, FeedKind::Video).unwrap();
        assert_eq!(parsed.entries[0].title, "A & B <tag>");
        assert_eq!(parsed.entries[0].description.as_deref(), Some("x & y"));
    }

    #[test]
    fn test_parse_feed_missing_video_id_skipped() {
        let xml = r#"<?xml version="1.0"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>Chan</title>
  <entry>
    <title>No ID</title>
    <published>2024-01-01T00:00:00+00:00</published>
  </entry>
</feed>"#;
        let parsed = parse_feed(xml, FeedKind::Video).unwrap();
        assert!(parsed.entries.is_empty(), "缺 videoId 的条目应被跳过");
    }

    #[test]
    fn test_parse_feed_malformed_returns_error() {
        assert!(parse_feed("<feed><entry></feed>", FeedKind::Video).is_err());
    }

    // ── extract_channel_id_from_input / html ──

    #[test]
    fn test_extract_channel_id_from_input_direct() {
        assert_eq!(
            extract_channel_id_from_input("UCXuqSBlHAE6Xw-yeJA0Tunw").as_deref(),
            Some("UCXuqSBlHAE6Xw-yeJA0Tunw")
        );
    }

    #[test]
    fn test_extract_channel_id_from_input_channel_url() {
        assert_eq!(
            extract_channel_id_from_input("https://www.youtube.com/channel/UCXuqSBlHAE6Xw-yeJA0Tunw")
                .as_deref(),
            Some("UCXuqSBlHAE6Xw-yeJA0Tunw")
        );
    }

    #[test]
    fn test_extract_channel_id_from_input_channel_url_with_suffix() {
        assert_eq!(
            extract_channel_id_from_input("youtube.com/channel/UCXuqSBlHAE6Xw-yeJA0Tunw/featured")
                .as_deref(),
            Some("UCXuqSBlHAE6Xw-yeJA0Tunw")
        );
    }

    #[test]
    fn test_extract_channel_id_from_input_handle_not_resolved() {
        assert!(extract_channel_id_from_input("@somehandle").is_none());
    }

    #[test]
    fn test_channel_page_url_variants() {
        assert_eq!(
            channel_page_url("@handle").unwrap(),
            "https://www.youtube.com/@handle"
        );
        assert_eq!(
            channel_page_url("handle").unwrap(),
            "https://www.youtube.com/@handle"
        );
        assert_eq!(
            channel_page_url("https://www.youtube.com/c/name").unwrap(),
            "https://www.youtube.com/c/name"
        );
        assert_eq!(
            channel_page_url("youtube.com/user/old").unwrap(),
            "https://www.youtube.com/@youtube.com/user/old"
        );
    }

    #[test]
    fn test_channel_page_url_rejects_non_youtube_host() {
        // SSRF 防护：非 youtube.com 域的 http(s):// 输入一律拒绝
        assert!(channel_page_url("http://127.0.0.1:8080/x").is_err());
        assert!(channel_page_url("https://evil.example.com/x").is_err());
        assert!(channel_page_url("https://youtube.com.evil.example.com/x").is_err());
        assert!(channel_page_url("http://youtube.com@evil.example.com/x").is_err());
        assert!(channel_page_url("https://www.youtube.com:443/@handle").is_ok());
        assert!(channel_page_url("https://m.youtube.com/@handle").is_ok());
        assert!(channel_page_url("https://www.youtube.com/c/name").is_ok());
    }

    #[test]
    fn test_extract_channel_id_from_html_json() {
        let html = r#"<script>var ytInitialData = {"header":{"channelId":"UCXuqSBlHAE6Xw-yeJA0Tunw"}};</script>"#;
        assert_eq!(
            extract_channel_id_from_html(html).as_deref(),
            Some("UCXuqSBlHAE6Xw-yeJA0Tunw")
        );
    }

    #[test]
    fn test_extract_channel_id_from_html_meta() {
        let html = r#"<meta itemprop="channelId" content="UCXuqSBlHAE6Xw-yeJA0Tunw">"#;
        assert_eq!(
            extract_channel_id_from_html(html).as_deref(),
            Some("UCXuqSBlHAE6Xw-yeJA0Tunw")
        );
    }

    #[test]
    fn test_extract_channel_id_from_html_canonical() {
        let html = r#"<link rel="canonical" href="https://www.youtube.com/channel/UCXuqSBlHAE6Xw-yeJA0Tunw">"#;
        assert_eq!(
            extract_channel_id_from_html(html).as_deref(),
            Some("UCXuqSBlHAE6Xw-yeJA0Tunw")
        );
    }

    #[test]
    fn test_extract_channel_id_from_html_not_found() {
        assert!(extract_channel_id_from_html("<html><body>nothing</body></html>").is_none());
    }

    // ── extract_og_title（频道页真实频道名）──

    #[test]
    fn test_extract_og_title_basic() {
        let html = r#"<html><head><meta property="og:title" content="时局眼"></head></html>"#;
        assert_eq!(extract_og_title(html).as_deref(), Some("时局眼"));
    }

    #[test]
    fn test_extract_og_title_unescapes_entities() {
        let html = r#"<meta property="og:title" content="A &amp; B">"#;
        assert_eq!(extract_og_title(html).as_deref(), Some("A & B"));
    }

    #[test]
    fn test_extract_og_title_missing_returns_none() {
        assert!(extract_og_title("<html>no og title</html>").is_none());
        assert!(extract_og_title("").is_none());
        assert!(extract_og_title("<meta property=\"og:title\" content=\"\">").is_none());
    }

    // ── resolve_channel_id（HTTP）──

    #[tokio::test]
    async fn test_resolve_channel_id_direct_no_http() {
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let id = resolve_channel_id(&client, "UCXuqSBlHAE6Xw-yeJA0Tunw")
            .await
            .unwrap();
        assert_eq!(id, "UCXuqSBlHAE6Xw-yeJA0Tunw");
    }

    #[tokio::test]
    async fn test_resolve_channel_id_via_page() {
        let mock = MockServer::start().await;
        let html = r#"<html><meta itemprop="channelId" content="UCResolvedFromPage12345678"></html>"#;
        Mock::given(method("GET"))
            .and(path("/@somehandle"))
            .respond_with(ResponseTemplate::new(200).set_body_string(html))
            .mount(&mock)
            .await;

        // channel_page_url 固定指向 youtube.com，测试无法重定向，
        // 这里直接验证 extract + page fetch 的核心逻辑：用 mock URL 走 fetch 分支。
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        // 通过内部函数模拟：先构造页面 URL（mock.uri() 充当频道页）
        let resp = client.get(format!("{}/@somehandle", mock.uri())).send().await.unwrap();
        assert!(resp.status().is_success());
        let body = resp.text().await.unwrap();
        assert_eq!(
            extract_channel_id_from_html(&body).as_deref(),
            Some("UCResolvedFromPage12345678")
        );
    }

    #[tokio::test]
    async fn test_fetch_feed_with_retry_success() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/feeds/videos.xml"))
            .and(query_param("playlist_id", "UULFabcdefghijklmnopqrst"))
            .respond_with(ResponseTemplate::new(200).set_body_string(sample_feed(&[(
                "aaa111",
                "Hello",
                "2024-01-01T00:00:00+00:00",
                Some("d"),
            )])))
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let xml = fetch_feed_raw(&client, &format!("{}/feeds/videos.xml?playlist_id=UULFabcdefghijklmnopqrst", mock.uri()))
            .await
            .unwrap();
        let parsed = parse_feed(&xml, FeedKind::Video).unwrap();
        assert_eq!(parsed.entries.len(), 1);
    }

    // ── save_entries ──

    fn entry_value(id: &str, title: &str, published: &str) -> serde_json::Value {
        serde_json::json!({
            "video_id": id,
            "title": title,
            "published": published,
            "description": Some(format!("desc-{}", id)),
            "thumbnail": Some(format!("https://i.ytimg.com/vi/{}/hqdefault.jpg", id)),
            "kind": "Video",
        })
    }

    #[test]
    fn test_save_entries_max_count_1_saves_latest() {
        let conn = db::init::init_memory_db().unwrap();
        let sid = db::sources::add_source(&conn, "youtube", "UCabc123", "", "").unwrap();
        let data = vec![
            entry_value("v3", "Three", "2024-03-01T00:00:00+00:00"),
            entry_value("v2", "Two", "2024-02-01T00:00:00+00:00"),
        ];
        let saved = save_entries(&conn, sid, &data, 1);
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].1.as_deref(), Some("desc-v3"));
        let rel = db::releases::get_releases_with_state(&conn).unwrap();
        assert_eq!(rel.len(), 1);
        assert_eq!(rel[0].tag_name, "v3");
        assert_eq!(rel[0].release_name, "Three");
        assert_eq!(rel[0].html_url, "https://www.youtube.com/watch/?v=v3");
        assert_eq!(rel[0].published_at, "2024-03-01T00:00:00+00:00");
        assert!(!rel[0].prerelease);
        let meta = rel[0].extra_metadata.as_ref().unwrap();
        assert!(meta.contains("\"kind\":\"video\""));
        assert!(meta.contains("hqdefault.jpg"));
    }

    #[test]
    fn test_save_entries_max_count_1_existing_returns_empty() {
        let conn = db::init::init_memory_db().unwrap();
        let sid = db::sources::add_source(&conn, "youtube", "UCabc123", "", "").unwrap();
        let data = vec![
            entry_value("v3", "Three", "2024-03-01T00:00:00+00:00"),
            entry_value("v2", "Two", "2024-02-01T00:00:00+00:00"),
        ];
        assert_eq!(save_entries(&conn, sid, &data, 1).len(), 1);
        // 再次保存：v3 已存在 → 返回空
        assert_eq!(save_entries(&conn, sid, &data, 1).len(), 0);
    }

    #[test]
    fn test_save_entries_historical_skips_existing() {
        let conn = db::init::init_memory_db().unwrap();
        let sid = db::sources::add_source(&conn, "youtube", "UCabc123", "", "").unwrap();
        let data = vec![
            entry_value("v3", "Three", "2024-03-01T00:00:00+00:00"),
            entry_value("v2", "Two", "2024-02-01T00:00:00+00:00"),
            entry_value("v1", "One", "2024-01-01T00:00:00+00:00"),
        ];
        assert_eq!(save_entries(&conn, sid, &data, 1).len(), 1);
        // 历史模式 max_count=2：v3 已存在跳过，v2/v1 新增
        let saved = save_entries(&conn, sid, &data, 2);
        assert_eq!(saved.len(), 2);
        assert_eq!(saved[0].1.as_deref(), Some("desc-v2"));
        assert_eq!(saved[1].1.as_deref(), Some("desc-v1"));
    }

    #[test]
    fn test_save_entries_sorts_by_published_desc() {
        let conn = db::init::init_memory_db().unwrap();
        let sid = db::sources::add_source(&conn, "youtube", "UCabc123", "", "").unwrap();
        // 输入乱序（v1 在前）
        let data = vec![
            entry_value("v1", "One", "2024-01-01T00:00:00+00:00"),
            entry_value("v3", "Three", "2024-03-01T00:00:00+00:00"),
            entry_value("v2", "Two", "2024-02-01T00:00:00+00:00"),
        ];
        let saved = save_entries(&conn, sid, &data, 3);
        assert_eq!(saved.len(), 3);
        let rel = db::releases::get_releases_with_state(&conn).unwrap();
        assert_eq!(rel[0].tag_name, "v3");
        assert_eq!(rel[1].tag_name, "v2");
        assert_eq!(rel[2].tag_name, "v1");
    }

    #[test]
    fn test_save_entries_skips_empty_video_id() {
        let conn = db::init::init_memory_db().unwrap();
        let sid = db::sources::add_source(&conn, "youtube", "UCabc123", "", "").unwrap();
        let data = vec![
            serde_json::json!({"video_id": "", "title": "NoId", "published": "2024-01-01T00:00:00+00:00", "kind": "Video"}),
            entry_value("v1", "One", "2024-01-01T00:00:00+00:00"),
        ];
        let saved = save_entries(&conn, sid, &data, 2);
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].1.as_deref(), Some("desc-v1"));
    }
}
