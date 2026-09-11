//! 凭据脱敏：日志与源健康状态的**唯一**出口过滤器（V28）。
//!
//! 背景：`check.failed` 曾把 reqwest 的原始错误文本整段落库，其中含
//! `…&key=AIzaSy…`（YouTube Data API key）——日志留存 14 天、可搜索、可导出，
//! 等于凭据明文外泄。原实现 `redact_url_credentials` 只剥离 URL userinfo
//! （`user:pass@`），**不覆盖 query 参数**，而 `?key=` / `?token=` 恰是主流形式。
//!
//! 因此这里把脱敏提到公共位置并覆盖三类载体：
//! 1. URL userinfo（原实现，行为保持不变）；
//! 2. URL query 参数中名字敏感的项（`key` / `token` / `api_key` / `secret` …）；
//! 3. 已知形状的凭据字面量（`AIza…` / `ghp_…` / `sk-…`），用于「错误文本里
//!    只回显了密钥本身、没有 URL 包裹」的情况。
//!
//! 调用点（单一出口原则，勿绕过）：
//! - `db::logs::write_log` / `write_log_key`：**所有**日志写入在此默认脱敏，
//!   并由 `insert_into_logs_is_confined_to_this_module` 测试护栅（绕开本模块直写 logs 表会红）；
//! - `db::sources::record_check_failure`：`sources.last_check_message` 列的唯一写入者，
//!   该列不经 `write_log_key`、会被源列表展示并随备份导出，故在同一位置收口；
//! - `poll::check_one_source` 的失败分支：该错误文本除落库外还**返回给前端**（手动检查
//!   失败时进 toast，用户截图报障即外泄），三处去向不可能逐出口处理，故在源头脱敏；
//! - `db::init` Migration 18：清理历史明文（一次性）。

/// 敏感 query 参数名（比较前会转小写、`-` 归一为 `_`）。
///
/// 只收「名字即凭据」的词，不收 `id` / `code` / `url` 等通用名——`code` 在
/// 日志里广泛用于表达 HTTP 状态码，误伤会让排障信息失真。
const SENSITIVE_PARAM_NAMES: &[&str] = &[
    "key",
    "apikey",
    "api_key",
    "token",
    "access_token",
    "accesstoken",
    "refresh_token",
    "refreshtoken",
    "secret",
    "client_secret",
    "clientsecret",
    "password",
    "passwd",
    "pwd",
    "auth",
    "authorization",
    "signature",
    "sig",
];

/// 已知凭据前缀 → 其后的字符数下限（低于下限视为普通文本，不脱敏）。
///
/// 下限取各平台实际长度的保守下界，避免把 `sk-` 开头的普通标识（如短 tag）
/// 误判为密钥。命中后连同前缀一起替换为 `***`。
const TOKEN_PREFIXES: &[(&str, usize)] = &[
    ("AIza", 30),        // Google API key（实际 39 字符）
    ("ghp_", 30),        // GitHub PAT
    ("gho_", 30),        // GitHub OAuth token
    ("ghu_", 30),        // GitHub user token
    ("ghs_", 30),        // GitHub server token
    ("ghr_", 30),        // GitHub refresh token
    ("github_pat_", 20), // GitHub fine-grained PAT
    ("sk-", 20),         // OpenAI / DeepSeek 兼容 API key
    ("xoxb-", 20),       // Slack bot token
    ("xoxp-", 20),       // Slack user token
];

/// 凭据字符集：`[A-Za-z0-9_-]`。
fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// URL 终止符：空白与引号/尖括号。
///
/// **不**把 `)` `,` `'` `]` 当终止符：它们在 RFC 3986 里属 sub-delims，可合法出现在
/// userinfo 或 query 值中；拿它们切分会把含 `)` 的口令截断成「无 `@` 的 authority」
/// 从而原样输出，反把凭据留在明文里。安全优先——多切不如少切。
fn is_url_end(c: char) -> bool {
    c.is_ascii_whitespace() || matches!(c, '"' | '\'' | '<' | '>')
}

/// URL authority 终止符：路径 / query / fragment 起始，或 URL 整体结束。
fn is_authority_end(c: char) -> bool {
    matches!(c, '/' | '?' | '#') || is_url_end(c)
}

/// 参数名是否为凭据类：精确名命中，或以 `_key` / `_token` / `_secret` /
/// `_password` 结尾（覆盖 `x-api-key`、`hf_token`、`my_secret` 等变体）。
fn is_sensitive_param_name(name: &str) -> bool {
    let normalized = name.trim().to_ascii_lowercase().replace('-', "_");
    if SENSITIVE_PARAM_NAMES.contains(&normalized.as_str()) {
        return true;
    }
    ["_key", "_token", "_secret", "_password"]
        .iter()
        .any(|suffix| normalized.ends_with(suffix))
}

/// 抹去单个 query 参数对的值（保留参数名与值尾部的标点，如 reqwest 的 `…key=abc)`）。
fn redact_query_pair(pair: &str) -> String {
    let Some(eq) = pair.find('=') else {
        return pair.to_string();
    };
    let (name, value_with_eq) = pair.split_at(eq);
    let value = &value_with_eq[1..];
    if !is_sensitive_param_name(name) {
        return pair.to_string();
    }
    // 尾部标点（`)` `]` `}` `,` `;` `.`）回填到 `***` 之后：脱敏的是值本身，
    // 不是句子里的标点——否则错误文本会丢掉闭合括号，可读性受损。
    let core_len = value
        .trim_end_matches([')', ']', '}', ',', ';', '.'])
        .len();
    format!("{}=***{}", name, &value[core_len..])
}

/// 脱敏单个 URL（已切出，不含前后文）：userinfo + 敏感 query 参数。
fn redact_one_url(url: &str) -> String {
    let authority_end = url.find(is_authority_end).unwrap_or(url.len());
    let (authority, tail) = url.split_at(authority_end);

    let mut out = String::with_capacity(url.len());
    match authority.rfind('@') {
        // at > 0 排除空 userinfo（`http://@host` 无需脱敏）
        Some(at) if at > 0 => {
            out.push_str("***:***@");
            out.push_str(&authority[at + 1..]);
        }
        _ => out.push_str(authority),
    }

    match tail.find('?') {
        Some(qpos) => {
            let (before_query, query_and_rest) = tail.split_at(qpos);
            out.push_str(before_query);
            out.push('?');
            let after_q = &query_and_rest[1..];
            let (query, fragment) = match after_q.find('#') {
                Some(h) => after_q.split_at(h),
                None => (after_q, ""),
            };
            let redacted: Vec<String> = query.split('&').map(redact_query_pair).collect();
            out.push_str(&redacted.join("&"));
            out.push_str(fragment);
        }
        None => out.push_str(tail),
    }

    out
}

/// 脱敏文本中的全部 URL（可能有多条，如「A 失败 / B 失败」拼接的日志）。
fn redact_urls(text: &str) -> String {
    const SCHEME_SEP: &str = "://";

    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(sep_pos) = rest.find(SCHEME_SEP) {
        let scheme_end = sep_pos + SCHEME_SEP.len();
        out.push_str(&rest[..scheme_end]);
        let after = &rest[scheme_end..];
        let url_end = after.find(is_url_end).unwrap_or(after.len());
        let (url, tail) = after.split_at(url_end);
        out.push_str(&redact_one_url(url));
        rest = tail;
    }
    out.push_str(rest);
    out
}

/// 替换已知形状的凭据字面量（前缀 + 足够长的 `[A-Za-z0-9_-]` 片段 → `***`）。
///
/// 每次只处理「剩余文本中最早出现」的一个候选，处理完必定前移游标，故无死循环；
/// 长度不达下限的形似片段（如 tag `sk-1.2`）原样保留，只跳过该前缀继续扫描。
fn redact_token_shapes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while !rest.is_empty() {
        let earliest = TOKEN_PREFIXES
            .iter()
            .filter_map(|p| rest.find(p.0).map(|pos| (pos, p)))
            .min_by_key(|(pos, _)| *pos);

        let Some((pos, (prefix, min_len))) = earliest else {
            out.push_str(rest);
            break;
        };

        let after_prefix = &rest[pos + prefix.len()..];
        let run_len = after_prefix
            .chars()
            .take_while(|c| is_token_char(*c))
            .map(|c| c.len_utf8())
            .sum::<usize>();

        if run_len >= *min_len {
            out.push_str(&rest[..pos]);
            out.push_str("***");
            rest = &after_prefix[run_len..];
        } else {
            let keep = pos + prefix.len();
            out.push_str(&rest[..keep]);
            rest = &rest[keep..];
        }
    }
    out
}

/// 对任意文本做凭据脱敏（userinfo + query 参数 + 已知凭据形状）。
///
/// 幂等：脱敏结果再次脱敏不再变化（值已被替换为 `***`，无敏感参数名可命中）。
pub fn redact(text: &str) -> String {
    redact_token_shapes(&redact_urls(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── userinfo（原 redact_url_credentials 的行为，迁移后必须保持）──

    #[test]
    fn redact_strips_userinfo_from_url() {
        assert_eq!(
            redact("error sending request for url (http://user:pass@proxy.example.com:8080)"),
            "error sending request for url (http://***:***@proxy.example.com:8080)"
        );
    }

    #[test]
    fn redact_strips_userinfo_without_password() {
        assert_eq!(redact("http://alice@host/path"), "http://***:***@host/path");
    }

    #[test]
    fn redact_handles_multiple_urls() {
        assert_eq!(
            redact("a http://u:p@h1 b https://u2:p2@h2/x?y=1 c"),
            "a http://***:***@h1 b https://***:***@h2/x?y=1 c"
        );
    }

    #[test]
    fn redact_handles_url_wrapped_in_parentheses() {
        // 回归：reqwest 的错误文本形如 `... for url (http://...)`。
        // 早期实现把 `)` 当 authority 边界，导致带 `)` 的口令被截断、凭据漏网。
        assert_eq!(
            redact("error sending request for url (http://u:p@h:8080)"),
            "error sending request for url (http://***:***@h:8080)"
        );
        // 口令自身含 `)`：必须从最后一个 `@` 切分，而非从 `)` 断词
        assert_eq!(redact("http://u:p)ss@h/x"), "http://***:***@h/x");
    }

    #[test]
    fn redact_uses_last_at_as_userinfo_boundary() {
        // host 不允许含 `@`，故最后一个 `@` 即 userinfo 结尾
        assert_eq!(redact("http://a@b@c/d"), "http://***:***@c/d");
    }

    #[test]
    fn redact_leaves_plain_text_untouched() {
        // 普通错误文本不含 `://`，须原样保留
        for s in [
            "connection reset by peer",
            "err.invalid_url",
            "error sending request for url (https://objects.githubusercontent.com/...)",
        ] {
            assert_eq!(redact(s), s);
        }
    }

    #[test]
    fn redact_leaves_empty_userinfo_untouched() {
        // 空 userinfo 无需脱敏（at > 0 判断）
        assert_eq!(redact("http://@host"), "http://@host");
    }

    // ── query 参数（V28 的核心缺口：key/token 以 query 形式出现）──

    #[test]
    fn redact_strips_query_api_key() {
        // 现网实证样本（YouTube Data API）
        let raw = "err.request_failed|error sending request for url (https://youtube.googleapis.com/youtube/v3/channels?part=contentDetails&id=UCrD39DnkX5QjIvH3yssXqJA&key=AIzaSyFAKEKEY0000000000000000000000)";
        let out = redact(raw);
        assert!(!out.contains("AIzaSy"), "API key 泄露: {out}");
        assert!(out.contains("&key=***)"), "应保留参数名与闭合括号: {out}");
        // 非凭据参数不受影响，排障信息不丢
        assert!(out.contains("part=contentDetails"));
        assert!(out.contains("id=UCrD39DnkX5QjIvH3yssXqJA"));
    }

    #[test]
    fn redact_covers_common_sensitive_param_names() {
        for (raw, expected) in [
            ("https://h/p?token=abc123", "https://h/p?token=***"),
            ("https://h/p?api_key=abc123", "https://h/p?api_key=***"),
            ("https://h/p?apiKey=abc123", "https://h/p?apiKey=***"),
            ("https://h/p?access_token=abc123", "https://h/p?access_token=***"),
            ("https://h/p?refresh_token=abc123", "https://h/p?refresh_token=***"),
            ("https://h/p?secret=abc123", "https://h/p?secret=***"),
            ("https://h/p?client_secret=abc123", "https://h/p?client_secret=***"),
            ("https://h/p?password=abc123", "https://h/p?password=***"),
            ("https://h/p?signature=abc123", "https://h/p?signature=***"),
            ("https://h/p?hf_token=abc123", "https://h/p?hf_token=***"),
            ("https://h/p?x-api-key=abc123", "https://h/p?x-api-key=***"),
        ] {
            assert_eq!(redact(raw), expected, "raw={raw}");
        }
    }

    #[test]
    fn redact_keeps_non_sensitive_params() {
        // `code` / `id` / `part` 等通用名不得误伤（日志排障依赖它们）
        for raw in [
            "https://h/p?code=404&id=123&part=snippet",
            "https://h/api?page=2&per_page=100",
        ] {
            assert_eq!(redact(raw), raw);
        }
    }

    #[test]
    fn redact_strips_all_params_in_multi_param_query() {
        assert_eq!(
            redact("https://h/p?a=1&key=secret&token=secret2&b=2"),
            "https://h/p?a=1&key=***&token=***&b=2"
        );
    }

    #[test]
    fn redact_keeps_fragment_after_query() {
        assert_eq!(
            redact("https://h/p?key=secret#section"),
            "https://h/p?key=***#section"
        );
    }

    #[test]
    fn redact_strips_query_credentials_inside_json_args() {
        // 日志 args 是 JSON 字符串：URL 收尾是 `"`，脱敏须在 JSON 文本上同样生效
        let raw = r#"{"error":"request failed for url (https://api.example.com/v1?token=abcdef&page=2)","owner":"o"}"#;
        let out = redact(raw);
        assert!(!out.contains("abcdef"), "token 泄露: {out}");
        assert!(out.contains("token=***"));
        assert!(out.contains("page=2") && out.contains(r#""owner":"o""#));
    }

    #[test]
    fn redact_is_idempotent() {
        let raw = "https://youtube.googleapis.com/youtube/v3/channels?id=UC1&key=AIzaSyFAKEKEY0000000000000000000000 gh_token ghp_abcdefghijklmnopqrstuvwxyz0123456789";
        let once = redact(raw);
        assert_eq!(redact(&once), once);
    }

    // ── 已知凭据形状（无 URL 包裹的裸密钥）──

    #[test]
    fn redact_strips_bare_google_api_key() {
        let out = redact("key rejected: AIzaSyFAKEKEY0000000000000000000000");
        assert!(!out.contains("AIzaSy"), "{out}");
        assert!(out.contains("key rejected: ***"));
    }

    #[test]
    fn redact_strips_bare_github_and_openai_tokens() {
        let out = redact(
            "auth failed (ghp_abcdefghijklmnopqrstuvwxyz0123456789 / sk-abcdefghijklmnopqrstuvwx)",
        );
        assert!(!out.contains("ghp_"), "{out}");
        assert!(!out.contains("sk-abc"), "{out}");
        assert_eq!(out, "auth failed (*** / ***)");
    }

    #[test]
    fn redact_keeps_short_lookalike_tokens() {
        // 长度不足的「形似」片段不是凭据，不得误伤（如 release tag `sk-1.2`）
        for raw in ["sk-1.2", "AIza short", "ghp_x"] {
            assert_eq!(redact(raw), raw);
        }
    }

    #[test]
    fn redact_handles_multiple_token_shapes_in_one_text() {
        let out = redact("AIzaSyFAKEKEY0000000000000000000000 and AIzaSyAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA");
        assert_eq!(out, "*** and ***");
    }
}
