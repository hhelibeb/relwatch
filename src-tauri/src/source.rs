//! 监控源适配器抽象层。
//!
//! 把每个 source 类型（github / huggingface / 未来 gitlab / gitee ...）的
//! fetch / save / verify 行为收敛到 `SourceAdapter` trait，调用方通过
//! `get_adapter(source_type)` 拿到 `Box<dyn SourceAdapter>`，不再用字符串匹配散落分发。
//!
//! 设计要点：
//! - `save` 设计为 async，吸收 github（同步 `&Connection`）与 huggingface（异步三阶段
//!   insert→fetch_readmes→finalize）的差异。github 实现内部用 `spawn_blocking` 包同步
//!   `save_releases`（与 Phase 2 的 spawn_blocking 改造顺接）。
//! - `save` 接收 `&Pool` 而非 `&Connection` 或 `&AppHandle`，由 trait 实现内部取连接，
//!   编排层不再关心两种取连接方式。
//! - `fetch` / `fetch_all` 对应原来的单页 / 翻页两种拉取模式。
//! - `verify_and_describe` 对应 `commands/source.rs::add_source` 中的
//!   verify + description 分支，消除第三处字符串匹配。

use async_trait::async_trait;
use std::sync::OnceLock;

use crate::db::sources::Source;

/// 源类型的鉴权方式：决定编排层从 settings 中取哪个 token 传给适配器。
///
/// 新增源类型只需在适配器中声明 `auth_kind`，编排层（poll.rs / commands）
/// 通过 `token_for` 统一取 token，无需再按 source_type 字符串匹配。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthKind {
    /// 无需鉴权（匿名请求）。
    None,
    /// GitHub Personal Access Token（KEY_GITHUB_TOKEN）。
    GitHubToken,
    /// YouTube Data API Key（KEY_YOUTUBE_API_KEY）。
    YouTubeApiKey,
    /// B 站登录 Cookie SESSDATA（KEY_BILIBILI_COOKIE，可选，降低风控）。
    BilibiliCookie,
}

impl AuthKind {
    /// 稳定字符串标识（下发前端展示/对拍用）。
    pub fn as_str(&self) -> &'static str {
        match self {
            AuthKind::None => "none",
            AuthKind::GitHubToken => "github_token",
            AuthKind::YouTubeApiKey => "youtube_api_key",
            AuthKind::BilibiliCookie => "bilibili_cookie",
        }
    }
}

/// 根据适配器声明的鉴权方式，从 settings 中选出对应 token。
/// 无鉴权源返回 None（适配器实现忽略 token 即可）。
#[allow(clippy::too_many_arguments)]
pub fn token_for<'a>(
    adapter: &dyn SourceAdapter,
    github_token: Option<&'a str>,
    youtube_api_key: Option<&'a str>,
    bilibili_cookie: Option<&'a str>,
) -> Option<&'a str> {
    match adapter.auth_kind() {
        AuthKind::None => None,
        AuthKind::GitHubToken => github_token,
        AuthKind::YouTubeApiKey => youtube_api_key,
        AuthKind::BilibiliCookie => bilibili_cookie,
    }
}

type AdapterFactory = fn() -> Box<dyn SourceAdapter>;

/// 全部已注册适配器的**单一来源**：source_type 字符串 → 构造器。
///
/// `list_adapters` / `get_adapter` 都从这份注册表取数，新增源类型只需在此
/// 登记一行（类型字符串 + 适配器类型），两份列表不会再有漏改风险。
static ADAPTERS: OnceLock<Vec<(&'static str, AdapterFactory)>> = OnceLock::new();

fn adapters() -> &'static [(&'static str, AdapterFactory)] {
    ADAPTERS.get_or_init(|| {
        vec![
            ("github", || Box::new(crate::github::GithubAdapter)),
            ("huggingface", || Box::new(crate::huggingface::HuggingFaceAdapter)),
            ("youtube", || Box::new(crate::youtube::YoutubeAdapter)),
            ("bilibili", || Box::new(crate::bilibili::BilibiliAdapter)),
        ]
    })
}

/// 返回所有已注册的适配器。
///
/// 用于按能力枚举（如 filter_ai_eligible 收集不参与 AI 摘要的源类型），
/// 新增源类型在 `ADAPTERS` 注册表登记后自动生效，无需在编排层逐个特判。
pub fn list_adapters() -> Vec<Box<dyn SourceAdapter>> {
    adapters().iter().map(|(_, factory)| factory()).collect()
}

/// 监控源适配器 trait：把 fetch / save / verify / description 收敛为统一接口。
///
/// `fetch` / `fetch_all` / `verify_and_describe` 接收的 `client` **不携带 default
/// Authorization header**（见 `http::build_http_client` 的 `set_default_auth` 说明）。
/// 鉴权 token 通过 `token` 参数传入，由适配器实现按请求 `.bearer_auth(token)`
/// 设置，确保 GitHub Token **不会**随 HuggingFace 请求泄露给 huggingface.co。
/// 无需鉴权的源（如 HuggingFace）实现中忽略 `token` 即可。
#[async_trait]
pub trait SourceAdapter: Send + Sync {
    /// 该适配器处理的 source_type 字符串（如 "github" / "huggingface"）。
    fn source_type(&self) -> &'static str;

    /// 该适配器的鉴权方式（决定编排层传哪个 token）。默认无鉴权。
    fn auth_kind(&self) -> AuthKind {
        AuthKind::None
    }

    /// 是否每次检查都按 fetch_history 数量拉历史（YouTube 频道用：
    /// 重新配置 Data API Key 后可补拉历史视频）。默认 false：仅首次查询时全量拉历史。
    fn always_fetch_history(&self) -> bool {
        false
    }

    /// 该源类型是否参与 AI 摘要/翻译。返回 false 的源（如 YouTube 视频）
    /// 在生成摘要前被编排层过滤。默认 true。
    fn ai_eligible(&self) -> bool {
        true
    }

    /// 检查成功后是否用 `verify_and_describe` 刷新 description
    /// （GitHub 仓库描述 / YouTube 真实频道名）。默认 false。
    fn refresh_description_after_check(&self) -> bool {
        false
    }

    /// 通知标题中的源显示名（owner 的可读替代）。默认原样返回 owner
    /// （GitHub/HF 通知标题拼 `owner / repo` 即可）；YouTube/B 站覆写为
    /// description（频道名/UP 主名，channel_id/UID 无阅读意义）。
    fn notification_source_name(
        &self,
        owner: &str,
        _repo: &str,
        _description: Option<&str>,
    ) -> String {
        owner.to_string()
    }

    /// 通知标题的来源标签（拼在可读源名前，如 `哔哩哔哩 / UP主名`）。
    /// 默认 None：GitHub/HF 标题本身是 `owner / repo`，无需标签。
    fn notification_source_label(&self) -> Option<&'static str> {
        None
    }

    /// 通知正文是否显示 tag（版本标识）。YouTube videoId / B 站 bvid 对用户无意义，
    /// 覆写为 false 后正文只显示视频标题。默认 true。
    fn notification_show_tag(&self) -> bool {
        true
    }

    /// 单页拉取（非翻页模式）。`token` 为该 source 对应的鉴权 token（可忽略）。
    async fn fetch(
        &self,
        client: &reqwest::Client,
        source: &Source,
        per_page: usize,
        token: Option<&str>,
    ) -> Result<Vec<serde_json::Value>, (u16, String)>;

    /// 翻页拉取全部（fetch_history 开启时的首次全量查询）。
    /// `max_count = None` 表示不设上限，`Some(n)` 表示最多拉取 n 条。
    async fn fetch_all(
        &self,
        client: &reqwest::Client,
        source: &Source,
        max_count: Option<usize>,
        token: Option<&str>,
    ) -> Result<Vec<serde_json::Value>, (u16, String)>;

    /// 保存到数据库。返回 `(id, body)` 对，供 `mark_older_as_read` 使用。
    /// 由实现内部从 `db` 取连接，吸收 github 同步 / HF 异步三阶段的差异。
    async fn save(
        &self,
        db: &r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>,
        source: &Source,
        data: &[serde_json::Value],
        max_count: usize,
        client: &reqwest::Client,
    ) -> Vec<(i64, Option<String>)>;

    /// 验证源可达并返回展示用的 description。`token` 为该 source 对应的鉴权
    /// token（无鉴权源可忽略）。对应 `commands/source.rs::add_source` 分支。
    async fn verify_and_describe(
        &self,
        client: &reqwest::Client,
        owner: &str,
        repo: &str,
        token: Option<&str>,
    ) -> Result<String, (u16, String)>;

    /// 将用户输入归一化为该源类型的标准 owner（默认原样返回）。
    /// YouTube 用它把 @handle / 频道链接解析为 channel_id，保证 RSS 拉取与
    /// 去重（UNIQUE(source_type, owner, repo)）都基于统一的 channel_id。
    /// `token` 为该 source 对应的鉴权 token（无鉴权源可忽略）。
    async fn resolve_owner(
        &self,
        _client: &reqwest::Client,
        owner: &str,
        _token: Option<&str>,
    ) -> Result<String, (u16, String)> {
        Ok(owner.to_string())
    }
}

/// 根据 source_type 字符串取得对应适配器。
///
/// 把原先散落在 `poll.rs`(2 处) / `commands/source.rs`(1 处) 的字符串匹配
/// 收敛为这一处分发；新增 source 类型只需在 `ADAPTERS` 注册表登记并实现
/// `SourceAdapter`。
pub fn get_adapter(source_type: &str) -> Result<Box<dyn SourceAdapter>, (u16, String)> {
    adapters()
        .iter()
        .find(|(t, _)| *t == source_type)
        .map(|(_, factory)| factory())
        .ok_or_else(|| (0, format!("err.unsupported_source|{}", source_type)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 能力位覆盖由 `test_list_adapters_capabilities`（按列枚举断言）兜底，
    /// 不再维护与实现逐行拷贝的元组快照（实现即事实）。
    #[test]
    fn test_auth_kind_as_str() {
        assert_eq!(AuthKind::None.as_str(), "none");
        assert_eq!(AuthKind::GitHubToken.as_str(), "github_token");
        assert_eq!(AuthKind::YouTubeApiKey.as_str(), "youtube_api_key");
        assert_eq!(AuthKind::BilibiliCookie.as_str(), "bilibili_cookie");
    }

    #[test]
    fn test_get_adapter_unknown_type() {
        match get_adapter("gitlab") {
            Err((0, msg)) => assert!(msg.contains("unsupported_source")),
            other => panic!("预期失败，实际返回 Ok: {}", other.is_ok()),
        }
    }

    #[test]
    fn test_token_for_picks_by_auth_kind() {
        let github = get_adapter("github").unwrap();
        let hf = get_adapter("huggingface").unwrap();
        let yt = get_adapter("youtube").unwrap();
        let bili = get_adapter("bilibili").unwrap();

        // GitHub：取 github token
        assert_eq!(
            token_for(github.as_ref(), Some("ghp"), Some("yt-key"), Some("sess")),
            Some("ghp")
        );
        // HuggingFace：无鉴权 → None（即使配置了 token 也不传）
        assert_eq!(
            token_for(hf.as_ref(), Some("ghp"), Some("yt-key"), Some("sess")),
            None
        );
        // YouTube：取 Data API Key
        assert_eq!(
            token_for(yt.as_ref(), Some("ghp"), Some("yt-key"), Some("sess")),
            Some("yt-key")
        );
        // Bilibili：取 SESSDATA cookie
        assert_eq!(
            token_for(bili.as_ref(), Some("ghp"), Some("yt-key"), Some("sess")),
            Some("sess")
        );
        // 未配置对应 token → None
        assert_eq!(token_for(yt.as_ref(), None, None, None), None);
        assert_eq!(token_for(bili.as_ref(), None, None, None), None);
    }

    #[test]
    fn test_list_adapters_capabilities() {
        let adapters = list_adapters();
        assert_eq!(adapters.len(), 4);

        // AI 排除集合应只含 youtube + bilibili（filter_ai_eligible 依赖此枚举）
        let ineligible: Vec<&str> = adapters
            .iter()
            .filter(|a| !a.ai_eligible())
            .map(|a| a.source_type())
            .collect();
        assert_eq!(ineligible, vec!["youtube", "bilibili"]);

        // 每次检查都拉历史的只有 youtube
        let always: Vec<&str> = adapters
            .iter()
            .filter(|a| a.always_fetch_history())
            .map(|a| a.source_type())
            .collect();
        assert_eq!(always, vec!["youtube"]);

        // 检查后刷新描述的：github + youtube + bilibili
        let refresh: Vec<&str> = adapters
            .iter()
            .filter(|a| a.refresh_description_after_check())
            .map(|a| a.source_type())
            .collect();
        assert_eq!(refresh, vec!["github", "youtube", "bilibili"]);

        // 通知正文不显示 tag 的：youtube + bilibili（videoId/bvid 无阅读意义）
        let hide_tag: Vec<&str> = adapters
            .iter()
            .filter(|a| !a.notification_show_tag())
            .map(|a| a.source_type())
            .collect();
        assert_eq!(hide_tag, vec!["youtube", "bilibili"]);

        // 通知标题默认能力：非视频源返回 owner 原样（标题由 notify 拼 owner / repo）
        let github_adapter = adapters.iter().find(|a| a.source_type() == "github").unwrap();
        assert_eq!(
            github_adapter.notification_source_name("o", "r", Some("任意描述")),
            "o"
        );
        // 非视频源无来源标签（GitHub 标题本身是 owner / repo）
        assert_eq!(github_adapter.notification_source_label(), None);
        // 视频源用 description 作可读源名（youtube 兼容旧版前缀）并带来源标签
        let yt_adapter = adapters.iter().find(|a| a.source_type() == "youtube").unwrap();
        assert_eq!(
            yt_adapter.notification_source_name("UCabc", "", Some("YouTube channel: Freesia")),
            "Freesia"
        );
        assert_eq!(yt_adapter.notification_source_label(), Some("YouTube"));
        let bili_adapter = adapters.iter().find(|a| a.source_type() == "bilibili").unwrap();
        assert_eq!(
            bili_adapter.notification_source_name("123456", "", Some("某UP主")),
            "某UP主"
        );
        assert_eq!(bili_adapter.notification_source_label(), Some("哔哩哔哩"));
    }
}
