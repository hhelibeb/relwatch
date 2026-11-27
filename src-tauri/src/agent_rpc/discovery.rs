use serde_json::Value;

use crate::db::agent::AgentConfig;
use super::{os, RpcAvailableModel};

pub(crate) fn normalize_path(p: &str) -> String {
    let mut s = p.replace('\\', "/");
    if cfg!(windows) {
        s = s.to_lowercase();
    }
    s.trim_end_matches('/').to_string()
}

pub fn ensure_supported_type(config: &AgentConfig) -> Result<(), String> {
    match config.agent_type.as_str() {
        "pi" => Ok(()),
        other => Err(format!("err.agent.unsupported_type|{}", other)),
    }
}

pub fn pi_agent_dir() -> Option<std::path::PathBuf> {
    if let Ok(dir) = std::env::var("PI_CODING_AGENT_DIR") {
        let t = dir.trim();
        if !t.is_empty() {
            return Some(std::path::PathBuf::from(t));
        }
    }
    dirs::home_dir().map(|h| h.join(".pi").join("agent"))
}

pub fn scoped_settings_path() -> Option<std::path::PathBuf> {
    pi_agent_dir().map(|dir| dir.join("settings.json"))
}

pub fn read_scoped_model_patterns() -> Vec<String> {
    let Some(path) = scoped_settings_path() else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(v) = serde_json::from_str::<Value>(&text) else {
        return Vec::new();
    };
    v.get("enabledModels")
        .and_then(|a| a.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn is_thinking_level(s: &str) -> bool {
    matches!(
        s,
        "off" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max"
    )
}

fn strip_thinking_suffix(pattern: &str) -> &str {
    if let Some(idx) = pattern.rfind(':') {
        if is_thinking_level(&pattern[idx + 1..]) {
            return &pattern[..idx];
        }
    }
    pattern
}

fn has_glob(pattern: &str) -> bool {
    pattern.contains('*') || pattern.contains('?')
}

/// 线性时间双指针 glob 匹配（支持 `*` / `?`，ASCII 大小写不敏感），消除多 `*` 指数回溯。
pub(crate) fn glob_match(pattern: &str, text: &str) -> bool {
    let p = pattern.as_bytes();
    let t = text.as_bytes();
    let (mut pi, mut ti) = (0usize, 0usize);
    let mut star_pi: Option<usize> = None;
    let mut star_ti: usize = 0;

    while ti < t.len() {
        if pi < p.len() && (p[pi] == b'?' || p[pi].eq_ignore_ascii_case(&t[ti])) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == b'*' {
            star_pi = Some(pi);
            star_ti = ti;
            pi += 1;
        } else if let Some(sp) = star_pi {
            star_ti += 1;
            ti = star_ti;
            pi = sp + 1;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == b'*' {
        pi += 1;
    }
    pi == p.len()
}

pub fn model_matches_scoped_pattern(pattern: &str, m: &RpcAvailableModel) -> bool {
    let pat = strip_thinking_suffix(pattern);
    let full = format!("{}/{}", m.provider, m.id);
    if has_glob(pat) {
        return glob_match(pat, &full) || glob_match(pat, &m.id);
    }
    let patl = pat.to_lowercase();
    let full_l = full.to_lowercase();
    let id_l = m.id.to_lowercase();
    let name_l = m.name.as_deref().unwrap_or("").to_lowercase();
    if full_l == patl || id_l == patl || (!name_l.is_empty() && name_l == patl) {
        return true;
    }
    full_l.contains(&patl) || id_l.contains(&patl) || (!name_l.is_empty() && name_l.contains(&patl))
}

pub fn filter_scoped_models(
    models: Vec<RpcAvailableModel>,
    patterns: &[String],
) -> Vec<RpcAvailableModel> {
    if patterns.is_empty() {
        return models;
    }
    let mut out: Vec<RpcAvailableModel> = Vec::new();
    for m in &models {
        if patterns.iter().any(|p| model_matches_scoped_pattern(p, m)) && !out.contains(m) {
            out.push(m.clone());
        }
    }
    if out.is_empty() {
        models
    } else {
        out
    }
}

pub fn resolve_agent_binary(config: &AgentConfig) -> Result<String, String> {
    if let Some(bin) = &config.binary {
        return Ok(bin.clone());
    }
    let probe = if cfg!(windows) { "where" } else { "which" };
    let mut probe_cmd = std::process::Command::new(probe);
    probe_cmd.arg("pi");
    os::configure_probe_command(&mut probe_cmd);
    if let Ok(out) = probe_cmd.output() {
        if out.status.success() {
            if let Ok(s) = String::from_utf8(out.stdout) {
                if let Some(line) = s
                    .lines()
                    .next()
                    .map(|l| l.trim().to_string())
                    .filter(|l| !l.is_empty())
                {
                    return Ok(line);
                }
            }
        }
    }
    if cfg!(windows) {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            let candidate = std::path::Path::new(&appdata).join("npm").join("pi.cmd");
            if candidate.exists() {
                return Ok(candidate.to_string_lossy().to_string());
            }
        }
    }
    Err("err.agent.pi_not_found".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(provider: &str, id: &str, name: Option<&str>) -> RpcAvailableModel {
        RpcAvailableModel {
            provider: provider.to_string(),
            id: id.to_string(),
            name: name.map(|s| s.to_string()),
        }
    }

    #[test]
    fn normalize_path_handles_separators_and_case() {
        assert_eq!(normalize_path("/data/ws/"), "/data/ws");
        if cfg!(windows) {
            assert_eq!(
                normalize_path(r"C:\Data\Sessions\ws-1.jsonl"),
                "c:/data/sessions/ws-1.jsonl"
            );
            assert_eq!(normalize_path(r"E:\a\b"), "e:/a/b");
        }
    }

    #[test]
    fn glob_match_handles_star_and_question() {
        assert!(glob_match("opencode-go/*", "opencode-go/qwen3.7-max"));
        assert!(glob_match("cline-pass/*deepseek*", "cline-pass/deepseek-v4-flash"));
        assert!(!glob_match("opencode-go/*", "clinepass/deepseek-v4-flash"));
        assert!(glob_match("gpt?", "gpt1"));
        assert!(!glob_match("gpt?", "gpt12"));
        assert!(glob_match("*sonnet*", "anthropic/claude-sonnet-4"));
    }

    #[test]
    fn glob_match_linear_time_with_many_wildcards() {
        let pat = "*a*a*a*a*a*a*a*a*a*a*a*a*b";
        let text = "a".repeat(200);
        let t0 = std::time::Instant::now();
        assert!(!glob_match(pat, &text));
        assert!(t0.elapsed() < std::time::Duration::from_millis(50));
    }

    #[test]
    fn strip_thinking_suffix_only_for_levels() {
        assert_eq!(strip_thinking_suffix("deepseek-v4:high"), "deepseek-v4");
        assert_eq!(strip_thinking_suffix("deepseek-v4"), "deepseek-v4");
        assert_eq!(
            strip_thinking_suffix("openrouter/gpt-4o:exacto"),
            "openrouter/gpt-4o:exacto"
        );
    }

    #[test]
    fn model_matches_scoped_pattern_exact_and_fuzzy() {
        let m = model(
            "clinepass",
            "cline-pass/deepseek-v4-flash",
            Some("DeepSeek V4 Flash (ClinePass)"),
        );
        assert!(model_matches_scoped_pattern(
            "clinepass/cline-pass/deepseek-v4-flash",
            &m
        ));
        assert!(model_matches_scoped_pattern("cline-pass/deepseek-v4-flash", &m));
        assert!(model_matches_scoped_pattern("DeepSeek V4 Flash (clinepass)", &m));
        assert!(model_matches_scoped_pattern("deepseek-v4", &m));
        assert!(!model_matches_scoped_pattern("qwen3.7-max", &m));
    }

    #[test]
    fn filter_scoped_models_selects_only_enabled_and_dedups() {
        let all = vec![
            model("deepseek", "deepseek-v4-flash", Some("DeepSeek V4 Flash")),
            model("opencode-go", "qwen3.7-max", Some("Qwen3.7 Max")),
            model(
                "clinepass",
                "cline-pass/qwen3.8-max",
                Some("Qwen3.8 Max (ClinePass)"),
            ),
            model(
                "clinepass",
                "cline-pass/deepseek-v4-flash",
                Some("DeepSeek V4 Flash (ClinePass)"),
            ),
        ];
        let patterns = vec![
            "clinepass/cline-pass/qwen3.8-max".to_string(),
            "clinepass/cline-pass/deepseek-v4-flash".to_string(),
        ];
        let filtered = filter_scoped_models(all.clone(), &patterns);
        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().any(|m| m.id == "cline-pass/qwen3.8-max"));
        assert!(filtered.iter().any(|m| m.id == "cline-pass/deepseek-v4-flash"));
        assert_eq!(filtered[0].id, "cline-pass/qwen3.8-max");
    }

    #[test]
    fn filter_scoped_models_empty_patterns_or_no_match_falls_back_to_all() {
        let all = vec![model("a", "x", None), model("b", "y", None)];
        assert_eq!(filter_scoped_models(all.clone(), &[]).len(), 2);
        let nohit = vec!["zzz-none".to_string()];
        assert_eq!(filter_scoped_models(all, &nohit).len(), 2);
    }
}
