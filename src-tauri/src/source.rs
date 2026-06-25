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

use crate::db::sources::Source;

/// 监控源适配器 trait：把 fetch / save / verify / description 收敛为统一接口。
#[async_trait]
pub trait SourceAdapter: Send + Sync {
    /// 该适配器处理的 source_type 字符串（如 "github" / "huggingface"）。
    fn source_type(&self) -> &'static str;

    /// 单页拉取（非翻页模式）。
    async fn fetch(
        &self,
        client: &reqwest::Client,
        source: &Source,
        per_page: usize,
    ) -> Result<Vec<serde_json::Value>, (u16, String)>;

    /// 翻页拉取全部（fetch_history 开启时的首次全量查询）。
    /// `max_count = None` 表示不设上限，`Some(n)` 表示最多拉取 n 条。
    async fn fetch_all(
        &self,
        client: &reqwest::Client,
        source: &Source,
        max_count: Option<usize>,
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

    /// 验证源可达并返回展示用的 description。
    /// 对应 `commands/source.rs::add_source` 的 verify + description 分支。
    async fn verify_and_describe(
        &self,
        client: &reqwest::Client,
        owner: &str,
        repo: &str,
    ) -> Result<String, (u16, String)>;
}

/// 根据 source_type 字符串取得对应适配器。
///
/// 把原先散落在 `poll.rs`(2 处) / `commands/source.rs`(1 处) 的字符串匹配
/// 收敛为这一处分发；新增 source 类型只需在此加一个分支并实现 `SourceAdapter`。
pub fn get_adapter(source_type: &str) -> Result<Box<dyn SourceAdapter>, (u16, String)> {
    match source_type {
        "github" => Ok(Box::new(crate::github::GithubAdapter)),
        "huggingface" => Ok(Box::new(crate::huggingface::HuggingFaceAdapter)),
        other => Err((0, format!("err.unsupported_source|{}", other))),
    }
}
