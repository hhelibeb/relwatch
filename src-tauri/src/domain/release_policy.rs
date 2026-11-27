/// 从 tag 中提取至少两段的数字序列组成 semver 三元组（缺段补 0）。
/// 要求至少两段数字是刻意收紧：避免把视频 id / BV 号误读为版本号。
pub fn parse_semver(tag: &str) -> Option<(u64, u64, u64)> {
    let bytes = tag.as_bytes();
    let mut segs: Vec<u64> = Vec::with_capacity(3);
    let mut i = 0;
    while i < bytes.len() && segs.len() < 3 {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i - start > 10 {
                return None;
            }
            segs.push(tag[start..i].parse().ok()?);
            if segs.len() < 3
                && i + 1 < bytes.len()
                && bytes[i] == b'.'
                && bytes[i + 1].is_ascii_digit()
            {
                i += 1;
            } else {
                break;
            }
        } else {
            i += 1;
        }
    }
    if segs.len() < 2 {
        return None;
    }
    while segs.len() < 3 {
        segs.push(0);
    }
    Some((segs[0], segs[1], segs[2]))
}

pub fn bump_between(prev: (u64, u64, u64), cur: (u64, u64, u64)) -> Option<&'static str> {
    if cur.0 > prev.0 {
        Some("major")
    } else if cur.0 == prev.0 && cur.1 > prev.1 {
        Some("minor")
    } else if cur.0 == prev.0 && cur.1 == prev.1 && cur.2 > prev.2 {
        Some("patch")
    } else {
        None
    }
}

pub fn importance_ge(a: &str, b: &str) -> bool {
    let rank = |s: &str| match s {
        "大" => 3,
        "中" => 2,
        _ => 1,
    };
    rank(a) >= rank(b)
}

pub fn is_release_due(status: &str, snooze_until: Option<&str>) -> bool {
    if status == "snoozed" {
        if let Some(until) = snooze_until {
            if !until.is_empty() {
                if let Ok(until_time) = chrono::DateTime::parse_from_rfc3339(until) {
                    if until_time >= chrono::Utc::now() {
                        return false;
                    }
                }
            }
        }
    }
    true
}

pub fn failure_log_level(status: u16) -> &'static str {
    if matches!(status, 0 | 401 | 403 | 429) || status >= 500 {
        "WARN"
    } else {
        "ERROR"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_semver_extracts_triples_and_rejects_non_versions() {
        assert_eq!(parse_semver("v1.16.0"), Some((1, 16, 0)));
        assert_eq!(parse_semver("release-1.2.3-beta.1"), Some((1, 2, 3)));
        assert_eq!(parse_semver("1.16"), Some((1, 16, 0)));
        assert_eq!(parse_semver("BV11xxx"), None);
        assert_eq!(parse_semver("abc"), None);
    }

    #[test]
    fn bump_between_classifies_changes() {
        assert_eq!(bump_between((1, 0, 0), (2, 0, 0)), Some("major"));
        assert_eq!(bump_between((1, 0, 0), (1, 1, 0)), Some("minor"));
        assert_eq!(bump_between((1, 1, 0), (1, 1, 1)), Some("patch"));
        assert_eq!(bump_between((1, 1, 1), (1, 1, 1)), None);
        assert_eq!(bump_between((2, 0, 0), (1, 9, 9)), None);
    }

    #[test]
    fn importance_and_log_level_rules() {
        assert!(importance_ge("大", "中"));
        assert!(importance_ge("中", "中"));
        assert!(!importance_ge("小", "中"));
        assert_eq!(failure_log_level(0), "WARN");
        assert_eq!(failure_log_level(429), "WARN");
        assert_eq!(failure_log_level(502), "WARN");
        assert_eq!(failure_log_level(404), "ERROR");
    }
}
