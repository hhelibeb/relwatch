mod discovery;
#[cfg(not(windows))]
mod os_unix;
#[cfg(windows)]
mod os_windows;
mod pool;
mod process;

#[cfg(not(windows))]
use os_unix as os;
#[cfg(windows)]
use os_windows as os;

use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Serialize, Deserialize, Clone, Type, PartialEq)]
pub struct RpcAvailableModel {
    pub provider: String,
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
}

pub use discovery::{
    ensure_supported_type, filter_scoped_models, model_matches_scoped_pattern, pi_agent_dir,
    read_scoped_model_patterns, resolve_agent_binary, scoped_settings_path,
};
pub use os::kill_process_tree;
pub use pool::{AgentConfigProvider, DbConfigProvider, RpcManager};
pub use process::ProcessLease;

/// 在 `cmd.exe /C` 下会被二次解释的元字符。
///
/// 经 cmd 启动进程时，参数即使被 Rust 按 MSVC 规则加了引号也不安全：cmd 会先做
/// 自己的解析，`&`、`|`、`>`、`^`、`%VAR%` 足够拆出新命令。
/// 括号（如 `Program Files (x86)`）只在裸奔时才是语法符，引号能挡住，故不列入。
/// `!` 仅在 delayed expansion 下特殊，这里一并保守拒绝。
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn cmd_unsafe_char<'a>(parts: impl IntoIterator<Item = &'a str>) -> Option<(char, &'a str)> {
    const UNSAFE: &[char] = &['&', '|', '<', '>', '^', '"', '%', '!', '\r', '\n'];
    for part in parts {
        if let Some(c) = part.chars().find(|c| UNSAFE.contains(c)) {
            return Some((c, part));
        }
    }
    None
}

/// 二进制是否 `.exe`：Windows 上可直启、无需经 cmd 解释。
/// 用扩展名判定而非字符串后缀，避免尾随空格/大小写差异把它推到 cmd 分支。
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn is_exe_binary(binary: &str) -> bool {
    std::path::Path::new(binary.trim())
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
}

#[cfg(test)]
mod tests {
    use super::cmd_unsafe_char;

    #[test]
    fn cmd_unsafe_char_flags_metacharacters() {
        for part in ["a&b", "a|b", "a>b", "a^b", "a%b%", "a!b", "a\nb"] {
            assert!(cmd_unsafe_char([part]).is_some(), "应判为不安全: {:?}", part);
        }
        // 括号只在未加引号时才是语法符，引号能挡住，不应误伤 x86 路径
        for part in [
            "C:\\Program Files (x86)\\nodejs\\pi.cmd",
            "pi.cmd",
            "skills/demo.md",
            "anthropic/claude-sonnet-4:latest",
        ] {
            assert!(cmd_unsafe_char([part]).is_none(), "不应判为不安全: {:?}", part);
        }
    }
}
