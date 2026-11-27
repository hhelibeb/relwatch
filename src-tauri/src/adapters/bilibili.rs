use std::collections::HashMap;
use std::sync::OnceLock;

use md5::Digest as _;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::db::releases;
use crate::db::sources::Source;
use crate::error::{SourceError, SourceErrorKind};
use crate::source::{FetchedRelease, SourceAdapter, SourceExtraMetadata};

const BILI_API_BASE: &str = "https://api.bilibili.com";
const BILI_HOME: &str = "https://www.bilibili.com/";
const BILI_WATCH_BASE: &str = "https://www.bilibili.com/video";
const BILI_SPACE_REFERER: &str = "https://space.bilibili.com/";

const BILI_DYNAMIC_PATH: &str = "/x/polymer/web-dynamic/v1/feed/space";
const BILI_NAV_PATH: &str = "/x/web-interface/nav";
const BILI_SPI_PATH: &str = "/x/frontend/finger/spi";

const MIXIN_KEY_ENC_TAB: [usize; 64] = [
    46, 47, 18, 2, 53, 8, 23, 32, 15, 50, 10, 31, 58, 3, 45, 35, 27, 43, 5, 49, 33, 9, 42, 19,
    29, 28, 14, 39, 12, 38, 41, 13, 37, 48, 7, 16, 24, 55, 40, 61, 26, 17, 0, 1, 60, 51, 30, 4,
    22, 25, 54, 21, 56, 59, 6, 63, 57, 62, 11, 36, 20, 34, 44, 52,
];

const BILI_DYNAMIC_FEATURES: &str =
    "itemOpusStyle,listOnlyfans,opusBigCover,onlyfansVote,decorationCard,noPcover";

const MAX_FETCH_ALL_PAGES: usize = 50;
const MAX_FETCH_ALL_ITEMS: usize = 2000;

const BILI_TICKET_URL: &str =
    "https://api.bilibili.com/bapis/bilibili.api.ticket.v1.Ticket/GenWebTicket";

pub(crate) const BILI_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                       (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36";
const BILI_ACCEPT: &str = "application/json, text/plain, */*";
const BILI_ACCEPT_LANG: &str = "zh-CN,zh;q=0.9,en;q=0.8";
const BILI_TICKET_KEY: &str = "XgwSnGZ1p";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BiliEntry {
    bvid: String,
    title: String,
    published: String,
    description: String,
    thumbnail: String,
    duration: String,
    view_count: Option<i64>,
    up_name: String,
}

impl BiliEntry {
    fn html_url(&self) -> String {
        format!("{}/{}", BILI_WATCH_BASE, self.bvid)
    }

    fn into_fetched_release(self) -> FetchedRelease {
        let html_url = self.html_url();
        let thumbnail = self.thumbnail.replacen("http://", "https://", 1);
        FetchedRelease {
            tag: self.bvid,
            name: self.title,
            html_url,
            published: self.published,
            prerelease: false,
            body: Some(self.description),
            extra: SourceExtraMetadata::Video {
                kind: "video".to_string(),
                thumbnail: Some(thumbnail),
                duration: Some(self.duration),
                view_count: self.view_count,
            },
        }
    }
}

#[derive(Debug, Clone)]
struct WbiKeys {
    img_key: String,
    sub_key: String,
}

impl WbiKeys {
    fn mixin_key(&self) -> String {
        let raw = format!("{}{}", self.img_key, self.sub_key);
        MIXIN_KEY_ENC_TAB
            .iter()
            .take(32)
            .map(|&i| raw.as_bytes()[i] as char)
            .collect()
    }
}

fn encode_wbi_component(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            0x21 | 0x27 | 0x28 | 0x29 | 0x2A => {}
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

fn sign_wbi_query(params: &HashMap<String, String>, mixin_key: &str, wts: i64) -> String {
    let mut kv: Vec<(String, String)> = params
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    kv.push(("wts".to_string(), wts.to_string()));
    kv.sort_by(|a, b| a.0.cmp(&b.0));
    let base: Vec<String> = kv
        .iter()
        .map(|(k, v)| format!("{}={}", encode_wbi_component(k), encode_wbi_component(v)))
        .collect();
    let base = base.join("&");
    let w_rid = format!(
        "{:x}",
        md5::Md5::digest(format!("{}{}", base, mixin_key).as_bytes())
    );
    format!("{}&w_rid={}", base, w_rid)
}

async fn bili_get(
    client: &reqwest::Client,
    url: &str,
    cookie: Option<&str>,
    referer: &str,
) -> Result<String, SourceError> {
    crate::http::get_text_with_retry(
        client,
        url,
        |req| {
            let req = req
                .header("User-Agent", BILI_UA)
                .header("Referer", referer)
                .header("Accept", BILI_ACCEPT)
                .header("Accept-Language", BILI_ACCEPT_LANG);
            match cookie {
                Some(c) => req.header("Cookie", c),
                None => req,
            }
        },
        |e: &SourceError| {
            if matches!(e.status(), 412 | 429 | 400 | 403) {
                return false;
            }
            log::warn!("B 站请求失败(状态={}), 将重试: {}", e.status(), e);
            true
        },
        |status, _body| {
            if status == 412 {
                SourceError::new(
                    412,
                    SourceErrorKind::BiliRisk {
                        detail: Some("HTTP 412".to_string()),
                    },
                )
            } else {
                crate::http::default_api_error(status, _body)
            }
        },
    )
    .await
}

fn parse_bili_json(body: &str) -> Result<serde_json::Value, SourceError> {
    let v: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| SourceError::coded(0, "err.parse_failed", [e.to_string()]))?;
    let code = v.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
    if code != 0 {
        let msg = v
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("")
            .to_string();
        return Err(map_bili_code(code, &msg));
    }
    Ok(v)
}

fn map_bili_code(code: i64, msg: &str) -> SourceError {
    match code {
        -352 => SourceError::new(403, SourceErrorKind::BiliRisk { detail: None }),
        -799 => SourceError::new(429, SourceErrorKind::BiliRateLimit),
        -404 => SourceError::key(404, "err.bili_up_not_found"),
        -400 => SourceError::coded(400, "err.bili_invalid_params", [msg]),
        _ => SourceError::coded(0, "err.bili_api_error", [code.to_string(), msg.to_string()]),
    }
}

fn hmac_sha256_hex(key: &[u8], msg: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut k = key.to_vec();
    if k.len() > 64 {
        k = Sha256::digest(&k).to_vec();
    }
    k.resize(64, 0);
    let mut ipad = k.clone();
    let mut opad = k;
    for b in ipad.iter_mut() {
        *b ^= 0x36;
    }
    for b in opad.iter_mut() {
        *b ^= 0x5c;
    }
    let mut inner = ipad.clone();
    inner.extend_from_slice(msg);
    let inner_hash = Sha256::digest(&inner);
    let mut outer = opad.clone();
    outer.extend_from_slice(&inner_hash);
    format!("{:x}", Sha256::digest(&outer))
}

async fn fetch_bili_ticket(client: &reqwest::Client) -> Option<String> {
    let ts = chrono::Utc::now().timestamp();
    let hexsign = hmac_sha256_hex(BILI_TICKET_KEY.as_bytes(), format!("ts{}", ts).as_bytes());
    let url = format!(
        "{}?key_id=ec02&hexsign={}&context%5Bts%5D={}&csrf=",
        BILI_TICKET_URL, hexsign, ts
    );
    let resp = client
        .post(&url)
        .header("User-Agent", BILI_UA)
        .header("Referer", "https://www.bilibili.com/")
        .header("Accept", BILI_ACCEPT)
        .send()
        .await
        .ok()?;
    let v: serde_json::Value =
        crate::http::read_json_limited(resp, crate::http::MAX_JSON_BYTES)
            .await
            .ok()?;
    let ticket = v["data"]["ticket"].as_str()?.to_string();
    if ticket.is_empty() {
        return None;
    }
    Some(ticket)
}

const COOKIE_CACHE_TTL_SECS: i64 = 60 * 60;
const WBI_CACHE_TTL_SECS: i64 = 6 * 60 * 60;

struct CachedCookie {
    value: String,
    sessdata: Option<String>,
    expires_at: i64,
}

struct CachedWbiKeys {
    keys: WbiKeys,
    expires_at: i64,
}

#[derive(Default)]
struct BiliCookieCache {
    cookie: Option<CachedCookie>,
}

impl BiliCookieCache {
    fn cookie_hit(&self, sessdata: Option<&str>, now: i64) -> Option<&str> {
        let sd = sessdata.filter(|s| !s.is_empty());
        self.cookie
            .as_ref()
            .filter(|c| c.expires_at > now)
            .filter(|c| c.sessdata.as_deref() == sd)
            .map(|c| c.value.as_str())
    }
}

#[derive(Default)]
struct BiliWbiCache {
    wbi: Option<CachedWbiKeys>,
}

impl BiliWbiCache {
    fn wbi_hit(&self, now: i64) -> Option<&WbiKeys> {
        self.wbi
            .as_ref()
            .filter(|c| c.expires_at > now)
            .map(|c| &c.keys)
    }
}

fn bili_cookie_cache() -> &'static tokio::sync::Mutex<BiliCookieCache> {
    static CACHE: OnceLock<tokio::sync::Mutex<BiliCookieCache>> = OnceLock::new();
    CACHE.get_or_init(|| tokio::sync::Mutex::new(BiliCookieCache::default()))
}

fn bili_wbi_cache() -> &'static tokio::sync::Mutex<BiliWbiCache> {
    static CACHE: OnceLock<tokio::sync::Mutex<BiliWbiCache>> = OnceLock::new();
    CACHE.get_or_init(|| tokio::sync::Mutex::new(BiliWbiCache::default()))
}

async fn init_anonymous_cookie_via_client(client: &reqwest::Client) -> Result<String, SourceError> {
    let resp = client
        .get(BILI_HOME)
        .header("User-Agent", BILI_UA)
        .send()
        .await
        .map_err(|e| crate::http::describe_request_source_error(&e))?;
    let mut cookies: Vec<(String, String)> = Vec::new();
    for header in resp.headers().get_all("set-cookie") {
        if let Ok(raw) = header.to_str() {
            let kv = raw.split(';').next().unwrap_or("");
            if let Some((k, v)) = kv.split_once('=') {
                cookies.push((k.trim().to_string(), v.trim().to_string()));
            }
        }
    }
    if let Ok(spi) = client
        .get(format!("{}{}", BILI_API_BASE, BILI_SPI_PATH))
        .header("User-Agent", BILI_UA)
        .header("Referer", "https://www.bilibili.com/")
        .header("Accept", BILI_ACCEPT)
        .header("Accept-Language", BILI_ACCEPT_LANG)
        .send()
        .await
    {
        if let Ok(v) = crate::http::read_json_limited::<serde_json::Value>(
            spi,
            crate::http::MAX_JSON_BYTES,
        )
        .await
        {
            if let Some(b4) = v["data"]["b_4"].as_str() {
                if !b4.is_empty() {
                    cookies.push(("buvid4".to_string(), b4.to_string()));
                }
            }
        }
    }
    if let Some(t) = fetch_bili_ticket(client).await {
        cookies.push(("bili_ticket".to_string(), t));
    }
    Ok(cookies
        .iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<_>>()
        .join("; "))
}

fn parse_wbi_keys(body: &str) -> Result<WbiKeys, SourceError> {
    let v: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| SourceError::coded(0, "err.parse_failed", [e.to_string()]))?;
    let img_url = v["data"]["wbi_img"]["img_url"]
        .as_str()
        .ok_or_else(|| SourceError::coded(0, "err.bili_wbi_keys", ["img"]))?;
    let sub_url = v["data"]["wbi_img"]["sub_url"]
        .as_str()
        .ok_or_else(|| SourceError::coded(0, "err.bili_wbi_keys", ["sub"]))?;
    let img_key = img_url
        .rsplit('/')
        .next()
        .unwrap_or("")
        .split('.')
        .next()
        .unwrap_or("")
        .to_string();
    let sub_key = sub_url
        .rsplit('/')
        .next()
        .unwrap_or("")
        .split('.')
        .next()
        .unwrap_or("")
        .to_string();
    if img_key.is_empty() || sub_key.is_empty() {
        return Err(SourceError::coded(0, "err.bili_wbi_keys", ["empty"]));
    }
    if img_key.len() + sub_key.len() < 64 {
        return Err(SourceError::coded(0, "err.bili_wbi_keys", ["short"]));
    }
    Ok(WbiKeys { img_key, sub_key })
}

async fn fetch_wbi_keys(
    client: &reqwest::Client,
    cookie: Option<&str>,
) -> Result<WbiKeys, SourceError> {
    let now = chrono::Utc::now().timestamp();
    let mut cache = bili_wbi_cache().lock().await;
    if let Some(keys) = cache.wbi_hit(now) {
        return Ok(keys.clone());
    }
    let body = bili_get(
        client,
        &format!("{}{}", BILI_API_BASE, BILI_NAV_PATH),
        cookie,
        "https://www.bilibili.com/",
    )
    .await?;
    let keys = parse_wbi_keys(&body)?;
    cache.wbi = Some(CachedWbiKeys {
        keys: keys.clone(),
        expires_at: now + WBI_CACHE_TTL_SECS,
    });
    Ok(keys)
}

fn parse_play_count(play: &str) -> Option<i64> {
    let s = play.trim();
    if s.is_empty() || s == "--" {
        return None;
    }
    let (num, unit): (&str, f64) = if let Some(rest) = s.strip_suffix("万") {
        (rest, 10_000.0)
    } else if let Some(rest) = s.strip_suffix("亿") {
        (rest, 100_000_000.0)
    } else {
        (s, 1.0)
    };
    let n = (num.parse::<f64>().ok()? * unit).round() as i64;
    (n > 0).then_some(n)
}

async fn fetch_dynamic_page(
    client: &reqwest::Client,
    mid: &str,
    offset: Option<&str>,
    cookie: Option<&str>,
    keys: &WbiKeys,
) -> Result<(Vec<BiliEntry>, Option<String>), SourceError> {
    let mut params = HashMap::new();
    params.insert("host_mid".to_string(), mid.to_string());
    params.insert("timezone_offset".to_string(), "-480".to_string());
    params.insert("features".to_string(), BILI_DYNAMIC_FEATURES.to_string());
    if let Some(o) = offset {
        params.insert("offset".to_string(), o.to_string());
    }
    let wts = chrono::Utc::now().timestamp();
    let query = sign_wbi_query(&params, &keys.mixin_key(), wts);
    let url = format!("{}{}?{}", BILI_API_BASE, BILI_DYNAMIC_PATH, query);
    let body = bili_get(
        client,
        &url,
        cookie,
        &format!("{}{}", BILI_SPACE_REFERER, mid),
    )
    .await?;
    let v = parse_bili_json(&body)?;
    let data = &v["data"];
    let items = data["items"].as_array().cloned().unwrap_or_default();
    let next_offset = data["offset"].as_str().map(|s| s.to_string());
    let mut entries = Vec::new();
    for it in &items {
        if it["type"].as_str() != Some("DYNAMIC_TYPE_AV") {
            continue;
        }
        let archive = &it["modules"]["module_dynamic"]["major"]["archive"];
        let bvid = archive["bvid"].as_str().unwrap_or("").trim().to_string();
        if bvid.is_empty() {
            continue;
        }
        let title = archive["title"].as_str().unwrap_or("").to_string();
        let description = archive["desc"].as_str().unwrap_or("").to_string();
        let thumbnail = archive["cover"].as_str().unwrap_or("").to_string();
        let duration = archive["duration_text"].as_str().unwrap_or("").to_string();
        let view_count = archive["stat"]["play"].as_str().and_then(parse_play_count);
        let up_name = it["modules"]["module_author"]["name"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let published = match parse_pub_ts(it) {
            Some(ts) => chrono::DateTime::from_timestamp(ts, 0)
                .map(|d| d.to_rfc3339())
                .unwrap_or_default(),
            None => chrono::Utc::now().to_rfc3339(),
        };
        if title.is_empty() || published.is_empty() {
            continue;
        }
        entries.push(BiliEntry {
            bvid,
            title,
            published,
            description,
            thumbnail,
            duration,
            view_count,
            up_name,
        });
    }
    Ok((entries, next_offset))
}

fn parse_pub_ts(item: &serde_json::Value) -> Option<i64> {
    let v = &item["modules"]["module_author"]["pub_ts"];
    v.as_i64()
        .or_else(|| v.as_str().and_then(|s| s.parse::<i64>().ok()))
        .filter(|&ts| ts > 0)
}

fn extract_uid(input: &str) -> Option<String> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }
    if input.chars().all(|c| c.is_ascii_digit()) && input.len() <= 16 {
        return Some(input.to_string());
    }
    for pat in ["space.bilibili.com/", "bilibili.com/space/"] {
        if let Some(idx) = input.find(pat) {
            let rest = &input[idx + pat.len()..];
            let uid: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if !uid.is_empty() {
                return Some(uid);
            }
        }
    }
    if let Some(idx) = input.find("bilibili.com/") {
        let rest = &input[idx + "bilibili.com/".len()..];
        if !rest.starts_with("space") && !rest.starts_with("video") {
            let uid: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if !uid.is_empty() {
                return Some(uid);
            }
        }
    }
    None
}

async fn fetch_up_name(
    client: &reqwest::Client,
    mid: &str,
    cookie: Option<&str>,
) -> Result<String, SourceError> {
    let keys = fetch_wbi_keys(client, cookie).await?;
    let (entries, _) = fetch_dynamic_page(client, mid, None, cookie, &keys).await?;
    if let Some(e) = entries.first() {
        if !e.up_name.is_empty() {
            return Ok(e.up_name.clone());
        }
    }
    let page_url = format!("https://space.bilibili.com/{}", mid);
    if let Ok(body) = bili_get(client, &page_url, cookie, &page_url).await {
        if let Some(name) = extract_og_title(&body) {
            return Ok(name);
        }
    }
    Err(SourceError::coded(404, "err.bili_up_not_found", [mid]))
}

fn extract_og_title(html: &str) -> Option<String> {
    let name = crate::http::extract_og_title(html)?;
    if name.starts_with("bili_") {
        return None;
    }
    Some(name)
}

pub fn save_entries(
    conn: &Connection,
    source_id: i64,
    items: &[FetchedRelease],
    max_count: usize,
) -> Vec<(i64, Option<String>)> {
    let entries: Vec<crate::db::save::SaveEntry> = items
        .iter()
        .filter(|e| !e.tag.is_empty() && !e.published.is_empty())
        .map(FetchedRelease::to_save_entry)
        .collect();
    crate::db::save::save_entries_generic(
        conn,
        source_id,
        &entries,
        max_count,
        |conn, id, entry| {
            let _ = releases::set_release_body_and_metadata(
                conn,
                id,
                entry.body.as_deref(),
                entry.metadata.as_deref(),
            );
        },
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

pub struct BilibiliAdapter;

#[async_trait::async_trait]
impl SourceAdapter for BilibiliAdapter {
    fn source_type(&self) -> &'static str {
        "bilibili"
    }

    fn auth_kind(&self) -> crate::source::AuthKind {
        crate::source::AuthKind::BilibiliCookie
    }

    fn ai_eligible(&self) -> bool {
        false
    }

    fn notification_source_name(
        &self,
        owner: &str,
        _repo: &str,
        description: Option<&str>,
    ) -> String {
        let d = description.unwrap_or("").trim();
        if d.is_empty() {
            owner.to_string()
        } else {
            d.to_string()
        }
    }

    fn notification_source_label(&self) -> Option<&'static str> {
        Some("哔哩哔哩")
    }

    fn notification_show_tag(&self) -> bool {
        false
    }

    fn log_source_ident(
        &self,
        owner: &str,
        _repo: &str,
        description: Option<&str>,
    ) -> (String, String) {
        let name = description.filter(|d| !d.is_empty()).unwrap_or(owner);
        (name.to_string(), String::new())
    }

    fn log_release_tag<'a>(&self, _tag_name: &'a str, release_name: &'a str) -> &'a str {
        release_name
    }

    fn release_body_label(&self) -> &'static str {
        "视频简介"
    }

    fn refresh_description_after_check(&self) -> bool {
        true
    }

    async fn fetch(
        &self,
        client: &reqwest::Client,
        source: &Source,
        per_page: usize,
        token: Option<&str>,
    ) -> Result<Vec<FetchedRelease>, SourceError> {
        let cookie = build_cookie(client, token).await?;
        let keys = fetch_wbi_keys(client, cookie.as_deref()).await?;
        let (entries, _) =
            fetch_dynamic_page(client, &source.owner, None, cookie.as_deref(), &keys).await?;
        Ok(entries
            .into_iter()
            .take(per_page)
            .map(BiliEntry::into_fetched_release)
            .collect())
    }

    async fn fetch_all(
        &self,
        client: &reqwest::Client,
        source: &Source,
        max_count: Option<usize>,
        token: Option<&str>,
    ) -> Result<Vec<FetchedRelease>, SourceError> {
        let cookie = build_cookie(client, token).await?;
        let keys = fetch_wbi_keys(client, cookie.as_deref()).await?;
        let mut entries = Vec::new();
        let mut offset: Option<String> = None;
        let mut pages = 0usize;
        loop {
            if pages >= MAX_FETCH_ALL_PAGES {
                log::warn!(
                    "bilibili {} 翻页达到上限 {} 页，停止拉取（已获 {} 条）",
                    source.owner,
                    MAX_FETCH_ALL_PAGES,
                    entries.len()
                );
                break;
            }
            let (page, next) = fetch_dynamic_page(
                client,
                &source.owner,
                offset.as_deref(),
                cookie.as_deref(),
                &keys,
            )
            .await?;
            entries.extend(page);
            pages += 1;
            if let Some(limit) = max_count {
                if entries.len() >= limit {
                    break;
                }
            }
            if entries.len() >= MAX_FETCH_ALL_ITEMS {
                log::warn!(
                    "bilibili {} 条目数达到上限 {}，停止拉取",
                    source.owner,
                    MAX_FETCH_ALL_ITEMS
                );
                break;
            }
            match next {
                Some(o) if !o.is_empty() && Some(o.as_str()) != offset.as_deref() => {
                    offset = Some(o)
                }
                _ => break,
            }
        }
        if let Some(limit) = max_count {
            entries.truncate(limit);
        }
        Ok(entries
            .into_iter()
            .map(BiliEntry::into_fetched_release)
            .collect())
    }

    async fn save(
        &self,
        db: &r2d2::Pool<crate::db::SqliteConnectionManager>,
        source: &Source,
        data: &[FetchedRelease],
        max_count: usize,
        _client: &reqwest::Client,
    ) -> Vec<(i64, Option<String>)> {
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
            log::error!("bilibili save spawn_blocking panic: {}", e);
            vec![]
        })
    }

    async fn verify_and_describe(
        &self,
        client: &reqwest::Client,
        owner: &str,
        _repo: &str,
        token: Option<&str>,
    ) -> Result<String, SourceError> {
        let cookie = build_cookie(client, token).await?;
        fetch_up_name(client, owner, cookie.as_deref()).await
    }

    async fn resolve_owner(
        &self,
        _client: &reqwest::Client,
        owner: &str,
        _token: Option<&str>,
    ) -> Result<String, SourceError> {
        extract_uid(owner).ok_or_else(|| SourceError::coded(400, "err.bili_invalid_uid", [owner]))
    }
}

async fn build_cookie(
    client: &reqwest::Client,
    sessdata: Option<&str>,
) -> Result<Option<String>, SourceError> {
    let sessdata = sessdata.filter(|s| !s.is_empty());
    let now = chrono::Utc::now().timestamp();
    let mut cache = bili_cookie_cache().lock().await;
    if let Some(hit) = cache.cookie_hit(sessdata, now) {
        return Ok(Some(hit.to_string()));
    }
    let mut parts = init_anonymous_cookie_via_client(client).await?;
    if let Some(s) = sessdata {
        if !parts.is_empty() {
            parts.push_str("; ");
        }
        parts.push_str(&format!("SESSDATA={}", s));
    }
    if parts.is_empty() {
        return Ok(None);
    }
    cache.cookie = Some(CachedCookie {
        value: parts.clone(),
        sessdata: sessdata.map(|s| s.to_string()),
        expires_at: now + COOKIE_CACHE_TTL_SECS,
    });
    Ok(Some(parts))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::db::init::init_memory_db;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn test_mixin_key_from_doc_case() {
        let keys = WbiKeys {
            img_key: "7cd084941338484aae1ad9425b84077c".to_string(), // gitleaks:allow WBI 文档测试用例示例密钥
            sub_key: "4932caff0ff746eab6f01bf08b70ac45".to_string(), // gitleaks:allow WBI 文档测试用例示例密钥
        };
        assert_eq!(keys.mixin_key(), "ea1db124af3c7062474693fa704f4ff8");
    }

    #[test]
    fn test_sign_wbi_from_doc_case() {
        let mixin = "ea1db124af3c7062474693fa704f4ff8";
        let mut params = HashMap::new();
        params.insert("bar".to_string(), "514".to_string());
        params.insert("foo".to_string(), "114".to_string());
        params.insert("zab".to_string(), "1919810".to_string());
        let query = sign_wbi_query(&params, mixin, 1702204169);
        assert!(query.contains("wts=1702204169"));
        assert!(query.contains("w_rid=8f6f2b5b3d485fe1886cec6a0be8c5d4"));
    }

    #[test]
    fn test_encode_wbi_component_filters_and_uppercase() {
        assert_eq!(encode_wbi_component("a b"), "a%20b");
        assert_eq!(encode_wbi_component("ab!c'd(e)f*g"), "abcdefg");
        assert_eq!(encode_wbi_component("视频"), "%E8%A7%86%E9%A2%91");
        assert_eq!(encode_wbi_component("one one four"), "one%20one%20four");
    }

    #[test]
    fn test_extract_uid_variants() {
        assert_eq!(extract_uid("12345678").as_deref(), Some("12345678"));
        assert_eq!(extract_uid(" 1234567 ").as_deref(), Some("1234567"));
        assert_eq!(
            extract_uid("1234567890123456").as_deref(),
            Some("1234567890123456")
        );
        assert_eq!(
            extract_uid("https://space.bilibili.com/1234567890123456").as_deref(),
            Some("1234567890123456")
        );
        assert_eq!(
            extract_uid("https://space.bilibili.com/12345678/video").as_deref(),
            Some("12345678")
        );
        assert_eq!(
            extract_uid("space.bilibili.com/12345678?from=search").as_deref(),
            Some("12345678")
        );
        assert_eq!(
            extract_uid("https://bilibili.com/space/1234567").as_deref(),
            Some("1234567")
        );
        assert_eq!(extract_uid("@someuser"), None);
        assert_eq!(extract_uid("https://www.bilibili.com/video/BV1xx"), None);
        assert_eq!(extract_uid(""), None);
    }

    #[test]
    fn test_parse_pub_ts_variants() {
        let mk = |v: serde_json::Value| {
            serde_json::json!({"type": "DYNAMIC_TYPE_AV", "modules": {"module_author": v}})
        };
        assert_eq!(parse_pub_ts(&mk(serde_json::json!({"pub_ts": 1746450829}))), Some(1746450829));
        assert_eq!(parse_pub_ts(&mk(serde_json::json!({"pub_ts": "1746450829"}))), Some(1746450829));
        assert_eq!(parse_pub_ts(&mk(serde_json::json!({}))), None);
        assert_eq!(parse_pub_ts(&mk(serde_json::json!({"pub_ts": null}))), None);
        assert_eq!(parse_pub_ts(&mk(serde_json::json!({"pub_ts": 0}))), None);
        assert_eq!(parse_pub_ts(&mk(serde_json::json!({"pub_ts": "abc"}))), None);
    }

    fn dynamic_item(bvid: &str, title: &str, pub_ts: i64, up_name: &str) -> serde_json::Value {
        serde_json::json!({
            "type": "DYNAMIC_TYPE_AV",
            "modules": {
                "module_author": {"name": up_name, "pub_ts": pub_ts},
                "module_dynamic": {
                    "major": {
                        "type": "MAJOR_TYPE_ARCHIVE",
                        "archive": {
                            "bvid": bvid,
                            "title": title,
                            "desc": "简介",
                            "cover": "https://i0.hdslb.com/bfs/archive/xxx.jpg",
                            "duration_text": "12:34",
                            "stat": {"danmaku": "2506", "play": "23.4万", "vt": ""}
                        }
                    }
                }
            }
        })
    }

    fn dynamic_body(items: Vec<serde_json::Value>, offset: Option<&str>) -> String {
        serde_json::json!({
            "code": 0, "message": "0", "ttl": 1,
            "data": {"has_more": true, "offset": offset, "items": items}
        })
        .to_string()
    }

    #[test]
    fn test_fetch_dynamic_page_parses_video_entries() {
        let body = dynamic_body(
            vec![
                dynamic_item("BV1a1b2c3d4e5f", "测试视频", 1723959548, "某UP主"),
                serde_json::json!({"type": "DYNAMIC_TYPE_FORWARD", "modules": {}}),
            ],
            Some("1160055449596198920"),
        );
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let data = &v["data"];
        let items = data["items"].as_array().unwrap();
        let mut entries = Vec::new();
        for it in items {
            if it["type"].as_str() != Some("DYNAMIC_TYPE_AV") {
                continue;
            }
            let archive = &it["modules"]["module_dynamic"]["major"]["archive"];
            let bvid = archive["bvid"].as_str().unwrap().to_string();
            let pub_ts = it["modules"]["module_author"]["pub_ts"].as_i64().unwrap();
            let view_count = archive["stat"]["play"].as_str().and_then(parse_play_count);
            entries.push(BiliEntry {
                bvid,
                title: archive["title"].as_str().unwrap().to_string(),
                published: chrono::DateTime::from_timestamp(pub_ts, 0)
                    .unwrap()
                    .to_rfc3339(),
                description: archive["desc"].as_str().unwrap().to_string(),
                thumbnail: archive["cover"].as_str().unwrap().to_string(),
                duration: archive["duration_text"].as_str().unwrap().to_string(),
                view_count,
                up_name: it["modules"]["module_author"]["name"].as_str().unwrap().to_string(),
            });
        }
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].bvid, "BV1a1b2c3d4e5f");
        assert_eq!(entries[0].up_name, "某UP主");
        assert!(entries[0].published.starts_with("2024-08-18"));
        assert_eq!(entries[0].html_url(), "https://www.bilibili.com/video/BV1a1b2c3d4e5f");
        assert_eq!(entries[0].view_count, Some(234000));
    }

    #[test]
    fn test_parse_play_count_variants() {
        assert_eq!(parse_play_count("1000"), Some(1000));
        assert_eq!(parse_play_count("999"), Some(999));
        assert_eq!(parse_play_count("23.4万"), Some(234000));
        assert_eq!(parse_play_count("1.2亿"), Some(120000000));
        assert_eq!(parse_play_count("1万"), Some(10000));
        assert_eq!(parse_play_count(""), None);
        assert_eq!(parse_play_count("--"), None);
        assert_eq!(parse_play_count("abc"), None);
        assert_eq!(parse_play_count("万"), None);
    }

    #[test]
    fn test_map_bili_code() {
        let e = map_bili_code(-352, "风控校验失败");
        assert_eq!(e.status(), 403);
        assert!(e.to_ipc_string().contains("err.bili_risk"));
        assert!(e.is_uncounted_failure());
        assert_eq!(map_bili_code(-799, "").status(), 429);
        assert!(map_bili_code(-799, "").is_uncounted_failure());
        assert_eq!(map_bili_code(-404, "").status(), 404);
        let e = map_bili_code(-123, "boom");
        assert_eq!(e.status(), 0);
        assert!(e.to_ipc_string().contains("err.bili_api_error|-123|boom"));
    }

    #[test]
    fn test_save_entries_dedup() {
        let conn = init_memory_db().unwrap();
        let source_id = db::sources::add_source(&conn, "bilibili", "12345678", "", "").unwrap();
        let entry = BiliEntry {
            bvid: "BV1a1b2c3d4e5f".to_string(),
            title: "新视频".to_string(),
            published: "2024-08-18T02:39:08+00:00".to_string(),
            description: "简介".to_string(),
            thumbnail: "https://i0.hdslb.com/bfs/archive/xxx.jpg".to_string(),
            duration: "12:34".to_string(),
            view_count: Some(123456),
            up_name: "UP".to_string(),
        };
        let items = vec![entry.into_fetched_release()];
        let saved = save_entries(&conn, source_id, &items, 1);
        assert_eq!(saved.len(), 1);
        let saved2 = save_entries(&conn, source_id, &items, 1);
        assert!(saved2.is_empty());
        let saved3 = save_entries(&conn, source_id, &items, 100);
        assert!(saved3.is_empty());
    }

    #[test]
    fn test_notification_source_name_uses_up_name() {
        let adapter = BilibiliAdapter;
        assert_eq!(
            adapter.notification_source_name("123456", "", Some("某UP主")),
            "某UP主"
        );
        assert_eq!(adapter.notification_source_name("123456", "", None), "123456");
        assert_eq!(adapter.notification_source_name("123456", "", Some(" ")), "123456");
    }

    #[test]
    fn test_notification_show_tag_false_for_video() {
        assert!(!BilibiliAdapter.notification_show_tag());
    }

    #[test]
    fn test_notification_source_label() {
        assert_eq!(BilibiliAdapter.notification_source_label(), Some("哔哩哔哩"));
    }

    #[test]
    fn test_parse_wbi_keys_accepts_unlogged_nav() {
        let body = serde_json::json!({
            "code": -101, "message": "账号未登录", "ttl": 1,
            "data": {"wbi_img": {
                "img_url": "https://i0.hdslb.com/bfs/wbi/7cd084941338484aae1ad9425b84077c.png", // gitleaks:allow WBI 文档测试用例示例密钥
                "sub_url": "https://i0.hdslb.com/bfs/wbi/4932caff0ff746eab6f01bf08b70ac45.png" // gitleaks:allow WBI 文档测试用例示例密钥
            }}
        })
        .to_string();
        let keys = parse_wbi_keys(&body).unwrap();
        assert_eq!(keys.img_key, "7cd084941338484aae1ad9425b84077c"); // gitleaks:allow WBI 文档测试用例示例密钥
        assert_eq!(keys.sub_key, "4932caff0ff746eab6f01bf08b70ac45"); // gitleaks:allow WBI 文档测试用例示例密钥
        assert_eq!(keys.mixin_key(), "ea1db124af3c7062474693fa704f4ff8");
    }

    #[test]
    fn test_parse_wbi_keys_missing_fields() {
        let body = serde_json::json!({"code": 0, "data": {}}).to_string();
        let err = parse_wbi_keys(&body).unwrap_err();
        assert!(err.to_ipc_string().contains("err.bili_wbi_keys"));
    }

    #[test]
    fn test_parse_wbi_keys_rejects_short_keys() {
        let body = serde_json::json!({
            "code": 0, "data": {"wbi_img": {
                "img_url": "https://i0.hdslb.com/bfs/wbi/7cd08494133848.png",
                "sub_url": "https://i0.hdslb.com/bfs/wbi/4932caff0ff746.png"
            }}
        })
        .to_string();
        let err = parse_wbi_keys(&body).unwrap_err();
        assert_eq!(err.to_ipc_string(), "err.bili_wbi_keys|short");
    }

    #[test]
    fn test_bili_cache_cookie_hit_rules() {
        let now = 1_000_000i64;
        let mut cache = BiliCookieCache::default();
        assert!(cache.cookie_hit(None, now).is_none());

        cache.cookie = Some(CachedCookie {
            value: "buvid3=x; bili_ticket=t".to_string(),
            sessdata: None,
            expires_at: now + 10,
        });
        assert_eq!(cache.cookie_hit(None, now), Some("buvid3=x; bili_ticket=t"));
        assert_eq!(cache.cookie_hit(Some(""), now), Some("buvid3=x; bili_ticket=t"));
        assert!(cache.cookie_hit(None, now + 10).is_none());
        assert!(cache.cookie_hit(Some("sess"), now).is_none());

        cache.cookie = Some(CachedCookie {
            value: "buvid3=x; SESSDATA=sess".to_string(),
            sessdata: Some("sess".to_string()),
            expires_at: now + 10,
        });
        assert_eq!(cache.cookie_hit(Some("sess"), now), Some("buvid3=x; SESSDATA=sess"));
        assert!(cache.cookie_hit(Some("other"), now).is_none());
    }

    #[test]
    fn test_bili_cache_wbi_hit_rules() {
        let mut cache = BiliWbiCache::default();
        assert!(cache.wbi_hit(100).is_none());
        cache.wbi = Some(CachedWbiKeys {
            keys: WbiKeys {
                img_key: "a".repeat(32),
                sub_key: "b".repeat(32),
            },
            expires_at: 200,
        });
        assert!(cache.wbi_hit(199).is_some());
        assert!(cache.wbi_hit(200).is_none());
    }

    #[tokio::test]
    async fn test_fetch_wbi_keys_via_nav() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/x/web-interface/nav"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": -101, "message": "账号未登录", "ttl": 1,
                "data": {
                    "wbi_img": {
                        "img_url": "https://i0.hdslb.com/bfs/wbi/7cd084941338484aae1ad9425b84077c.png", // gitleaks:allow WBI 文档测试用例示例密钥
                        "sub_url": "https://i0.hdslb.com/bfs/wbi/4932caff0ff746eab6f01bf08b70ac45.png" // gitleaks:allow WBI 文档测试用例示例密钥
                    }
                }
            })))
            .mount(&mock)
            .await;

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let body = client
            .get(format!("{}/x/web-interface/nav", mock.uri()))
            .header("User-Agent", BILI_UA)
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        let keys = parse_wbi_keys(&body).unwrap();
        assert_eq!(keys.img_key, "7cd084941338484aae1ad9425b84077c"); // gitleaks:allow WBI 文档测试用例示例密钥
    }

    #[test]
    fn test_metadata_json_upgrades_http_thumbnail_to_https() {
        let e = BiliEntry {
            bvid: "BV1xx".to_string(),
            title: "t".to_string(),
            published: "2024-01-01T00:00:00+00:00".to_string(),
            description: "d".to_string(),
            thumbnail: "http://i0.hdslb.com/bfs/archive/abc.jpg".to_string(),
            duration: "10:00".to_string(),
            view_count: Some(123456),
            up_name: "up".to_string(),
        };
        let meta: serde_json::Value = serde_json::from_str(
            &e.clone().into_fetched_release().extra.to_metadata_json().unwrap(),
        )
        .unwrap();
        assert_eq!(meta["thumbnail"], "https://i0.hdslb.com/bfs/archive/abc.jpg");
        assert_eq!(meta["view_count"], 123456);
        let e2 = BiliEntry {
            thumbnail: "https://i0.hdslb.com/bfs/archive/def.jpg".to_string(),
            ..e
        };
        let meta2: serde_json::Value = serde_json::from_str(
            &e2.into_fetched_release().extra.to_metadata_json().unwrap(),
        )
        .unwrap();
        assert_eq!(meta2["thumbnail"], "https://i0.hdslb.com/bfs/archive/def.jpg");
    }
}
