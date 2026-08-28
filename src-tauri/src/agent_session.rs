//! pi 会话 JSONL 解析 —— Agent 工作区聊天渲染的数据源。
//!
//! 工作区采用 RPC 常驻进程模型（`pi --mode rpc`，见 agent_rpc.rs），每次提交向
//! 常驻进程发 `prompt` 命令，pi 把完整对话（user / assistant / toolResult /
//! bashExecution 等）逐行 append 到会话文件（`pi --session <file>` 语义）。
//! 本模块把该 JSONL 解析为前端可渲染的结构化消息流（树结构取当前 leaf 路径）。
//! 格式依据 pi 官方文档 `docs/session-format.md`（v3：id/parentId 树形）。

use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::Path;

/// 一条消息中的内容块（前端按 kind 渲染）。
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AgentChatBlock {
    /// 普通文本（用户提问 / 助手回复 / 自定义消息）。
    Text { text: String },
    /// 思考过程（assistant thinking 块，默认折叠展示）。
    Thinking { text: String },
    /// 工具调用（assistant toolCall 块，默认折叠展示参数）。
    ToolCall { id: String, name: String, args: String },
    /// 工具结果（toolResult 消息，默认折叠展示输出）。
    ToolResult {
        id: String,
        tool_name: String,
        text: String,
        is_error: bool,
    },
    /// pi 的 bash 执行消息（命令 + 输出 + 退出码）。
    Bash {
        command: String,
        output: String,
        exit_code: Option<i64>,
        truncated: bool,
    },
}

/// 一条聊天消息（时间正序，树取当前 leaf 路径）。
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct AgentChatMessage {
    /// user | assistant | tool | bash | custom
    pub role: String,
    pub blocks: Vec<AgentChatBlock>,
    /// ISO 时间戳（entry 级别，非 message 内嵌）。
    pub timestamp: String,
    /// 生成该消息的模型（仅 assistant）。
    pub model: Option<String>,
    /// 与本次提交 run 的直连 id（仅 user 消息；由 list_agent_messages 按序对位填充，
    /// 前端据此把失败备注 / 重试入口精确挂到对应气泡，替代 60 秒时间窗猜测）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<i64>,
}

/// 解析会话文件。文件不存在 / 为空 → 空列表。
pub fn parse_session_file(path: &Path) -> Result<Vec<AgentChatMessage>, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("err.agent.read_session|{}", e))?;
    parse_session_jsonl(&content)
}

/// 解析会话 JSONL 文本（公开以便单测）。损坏/半行容忍：解析失败的行跳过，
/// 下轮轮询读到完整行后自然补齐（pi 整行 append，读取瞬间可能截半）。
pub fn parse_session_jsonl(content: &str) -> Result<Vec<AgentChatMessage>, String> {
    // id → (parentId, 消息)；保留文件顺序
    let mut entries: Vec<(String, Option<String>, AgentChatMessage)> = Vec::new();
    let mut last_id: Option<String> = None;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let value: serde_json::Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue, // 半行写入 / 未知扩展行，容忍
        };
        if value.get("type").and_then(|t| t.as_str()) != Some("message") {
            continue;
        }
        let id = value
            .get("id")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        if id.is_empty() {
            continue;
        }
        let parent = value
            .get("parentId")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string());
        let timestamp = value
            .get("timestamp")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let msg = value.get("message").cloned().unwrap_or(serde_json::Value::Null);
        last_id = Some(id.clone());
        entries.push((id, parent, convert_message(&msg, &timestamp)));
    }

    // 从最后一条消息沿 parentId 回溯到根，反转得到当前 leaf 路径。
    // 损坏文件可能出现 parentId 环（自引用/回指祖先）：visited 集合防死循环，
    // 深度上限兜底（正常会话数千条消息封顶，同步命令不可被损坏文件拖死）。
    let mut chain: Vec<String> = Vec::new();
    let mut cur = last_id;
    let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();
    let max_depth = entries.len().max(1) * 2 + 1000; // 正常树深度 ≤ 消息数，留余量
    while let Some(c) = cur {
        if !visited.insert(c.clone()) {
            break; // 检测到环
        }
        if chain.len() >= max_depth {
            break;
        }
        chain.push(c.clone());
        cur = entries
            .iter()
            .find(|(id, _, _)| *id == c)
            .and_then(|(_, parent, _)| parent.clone());
    }
    chain.reverse();

    let mut out = Vec::with_capacity(chain.len());
    for id in chain {
        if let Some((_, _, m)) = entries.iter().find(|(eid, _, _)| *eid == id) {
            out.push(m.clone());
        }
    }
    Ok(out)
}

fn convert_message(msg: &serde_json::Value, timestamp: &str) -> AgentChatMessage {
    let role = msg
        .get("role")
        .and_then(|r| r.as_str())
        .unwrap_or("")
        .to_string();
    let model = msg
        .get("model")
        .and_then(|m| m.as_str())
        .map(|s| s.to_string());
    let blocks = match role.as_str() {
        "user" => convert_user_blocks(msg),
        "assistant" => convert_assistant_blocks(msg),
        "toolResult" => convert_tool_result_blocks(msg),
        "bashExecution" => convert_bash_block(msg),
        _ => convert_generic_blocks(msg),
    };
    AgentChatMessage {
        role,
        blocks,
        timestamp: timestamp.to_string(),
        model,
        run_id: None,
    }
}

/// user 消息：content 为字符串或块数组。
fn convert_user_blocks(msg: &serde_json::Value) -> Vec<AgentChatBlock> {
    let content = &msg["content"];
    match content {
        serde_json::Value::String(s) => vec![AgentChatBlock::Text { text: s.clone() }],
        serde_json::Value::Array(items) => items
            .iter()
            .filter_map(|b| match b.get("type").and_then(|t| t.as_str()) {
                Some("text") => b
                    .get("text")
                    .and_then(|t| t.as_str())
                    .map(|t| AgentChatBlock::Text { text: t.to_string() }),
                _ => None, // image 等暂不渲染
            })
            .collect(),
        _ => vec![],
    }
}

/// assistant 消息：content 为 text / thinking / toolCall 块数组。
fn convert_assistant_blocks(msg: &serde_json::Value) -> Vec<AgentChatBlock> {
    let content = &msg["content"];
    let items = match content {
        serde_json::Value::Array(items) => items,
        _ => return vec![],
    };
    let mut blocks = Vec::new();
    for b in items {
        match b.get("type").and_then(|t| t.as_str()) {
            Some("text") => {
                if let Some(t) = b.get("text").and_then(|t| t.as_str()) {
                    blocks.push(AgentChatBlock::Text { text: t.to_string() });
                }
            }
            Some("thinking") => {
                if let Some(t) = b.get("thinking").and_then(|t| t.as_str()) {
                    blocks.push(AgentChatBlock::Thinking { text: t.to_string() });
                }
            }
            Some("toolCall") => {
                let id = b.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string();
                let name = b.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string();
                let args = b
                    .get("arguments")
                    .map(|a| serde_json::to_string(a).unwrap_or_default())
                    .unwrap_or_default();
                blocks.push(AgentChatBlock::ToolCall { id, name, args });
            }
            _ => {}
        }
    }
    blocks
}

/// toolResult 消息：content 文本块拼接为一条结果。
fn convert_tool_result_blocks(msg: &serde_json::Value) -> Vec<AgentChatBlock> {
    let id = msg
        .get("toolCallId")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let tool_name = msg
        .get("toolName")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let text = collect_text(&msg["content"]);
    let is_error = msg
        .get("isError")
        .and_then(|x| x.as_bool())
        .unwrap_or(false);
    vec![AgentChatBlock::ToolResult {
        id,
        tool_name,
        text,
        is_error,
    }]
}

/// bashExecution 消息（pi 的 shell 工具执行记录）。
fn convert_bash_block(msg: &serde_json::Value) -> Vec<AgentChatBlock> {
    let command = msg
        .get("command")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let output = msg
        .get("output")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let exit_code = msg.get("exitCode").and_then(|x| x.as_i64());
    let truncated = msg
        .get("truncated")
        .and_then(|x| x.as_bool())
        .unwrap_or(false);
    vec![AgentChatBlock::Bash {
        command,
        output,
        exit_code,
        truncated,
    }]
}

/// 其他消息（custom / compactionSummary / branchSummary 等）：文本化展示。
fn convert_generic_blocks(msg: &serde_json::Value) -> Vec<AgentChatBlock> {
    match &msg["content"] {
        serde_json::Value::String(s) => vec![AgentChatBlock::Text { text: s.clone() }],
        serde_json::Value::Array(_) => {
            let text = collect_text(&msg["content"]);
            if text.is_empty() {
                vec![]
            } else {
                vec![AgentChatBlock::Text { text }]
            }
        }
        _ => vec![],
    }
}

// ---- 会话标题重建（磁盘发现：localStorage 索引丢失后找回「数据到意义的映射」）----

/// 标题取首条用户指令的字符上限（与前端 `SessionMeta.title` 的 40 字截断对齐）。
pub const SESSION_TITLE_MAX_CHARS: usize = 40;

/// 只读取会话文件头部这么多字节来重建标题。
///
/// 首条 user 消息位于文件开头（紧随 `type:session` 行之后），无需读全文件；
/// 会话文件可能累积到数 MB，逐会话全量读取会让会话列表启动变慢。
const TITLE_SCAN_BYTES: u64 = 64 * 1024;

/// 从会话文件重建标题（取首条 user 消息中的用户真实指令）。
///
/// 会话索引（localStorage）落在 WebView2 缓存目录树中，清缓存即失联；而会话文件
/// 在 Roaming 数据目录里完好无损。本函数是「磁盘发现」的语义来源：文件即索引，
/// 标题从内容重建，不依赖任何外部索引。
///
/// 文件不存在 / 无法读取 / 无 user 消息 → None（调用方用占位标题兜底）。
pub fn session_title_from_file(path: &Path) -> Option<String> {
    use std::io::Read;
    let file = std::fs::File::open(path).ok()?;
    let mut buf = Vec::new();
    // 只取头部：读取失败或提前 EOF 时按已读到的内容继续（首条 user 消息通常在前几行）
    let _ = (&file).take(TITLE_SCAN_BYTES).read_to_end(&mut buf);
    let text = String::from_utf8_lossy(&buf);
    // 截断到最后一个换行，丢弃可能截半的末行
    let head = match text.rfind('\n') {
        Some(i) => &text[..i],
        None => text.as_ref(),
    };
    session_title_from_jsonl(head)
}

/// 从会话 JSONL 文本取首条 user 消息作为标题（公开以便单测）。
pub fn session_title_from_jsonl(content: &str) -> Option<String> {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let value: serde_json::Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue, // 半行写入 / 未知扩展行，容忍
        };
        if value.get("type").and_then(|t| t.as_str()) != Some("message") {
            continue;
        }
        let msg = value.get("message")?;
        if msg.get("role").and_then(|r| r.as_str()) != Some("user") {
            continue;
        }
        let raw = user_message_text(msg);
        let instruction = extract_user_instruction(&strip_skill_block(&raw));
        let title: String = instruction.chars().take(SESSION_TITLE_MAX_CHARS).collect();
        if !title.trim().is_empty() {
            return Some(title);
        }
        // 首条 user 消息无文本（纯附件等）→ 继续找下一条
    }
    None
}

/// 剥离 pi 展开 Skill 时注入的 `<skill name=…>…</skill>` 全文块。
///
/// 与前端 `stripSkillBlock`（AgentWorkspace.vue）同语义：不剥离的话，用了 Skill 的
/// 会话标题会变成 Skill 正文的前 40 字，而不是用户的真实指令。
fn strip_skill_block(text: &str) -> String {
    let start = match text.find("<skill") {
        Some(i) => i,
        None => return text.to_string(),
    };
    // 开标签结束位置；未闭合（异常内容）时原样返回
    let tag_end = match text[start..].find('>') {
        Some(i) => start + i + 1,
        None => return text.to_string(),
    };
    let rest = &text[tag_end..];
    match rest.find("</skill>") {
        Some(i) => {
            let after = rest[i + "</skill>".len()..].trim_start();
            format!("{}{}", &text[..start], after)
        }
        None => text.to_string(),
    }
}

/// user 消息的正文文本（content 为字符串或块数组，与 `convert_user_blocks` 同口径）。
fn user_message_text(msg: &serde_json::Value) -> String {
    match &msg["content"] {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(_) => collect_text(&msg["content"]),
        _ => String::new(),
    }
}

/// 取首轮模板 `<用户指令>…</用户指令>` 内的用户真实指令。
///
/// 首轮提交的 user 消息是完整任务模板（订阅说明 / 实体上下文 / 不可信数据声明），
/// 直接截前 40 字会得到模板脚手架；无标签（旧格式 / 多轮精简消息）时整段返回。
fn extract_user_instruction(text: &str) -> String {
    const OPEN: &str = "<用户指令>";
    const CLOSE: &str = "</用户指令>";
    if let Some(s) = text.find(OPEN) {
        let from = s + OPEN.len();
        if let Some(rel) = text[from..].find(CLOSE) {
            return text[from..from + rel].trim().to_string();
        }
    }
    text.trim().to_string()
}

/// 拼接 content 数组中的所有文本块。
fn collect_text(content: &serde_json::Value) -> String {
    let items = match content {
        serde_json::Value::Array(items) => items,
        _ => return String::new(),
    };
    let mut out = String::new();
    for b in items {
        if b.get("type").and_then(|t| t.as_str()) == Some("text") {
            if let Some(t) = b.get("text").and_then(|t| t.as_str()) {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(t);
            }
        }
    }
    out
}

// ---- 会话水位统计（上下文水位可见性）----

/// 会话文件的水位摘要（前端展示「消息 N 条 · 约 X tokens」）。
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct AgentSessionUsage {
    /// 消息总条数（含 user / assistant / tool / bash / custom）。
    pub message_count: i64,
    /// 会话内全部文本字符数（text / thinking / toolResult / bash 输出）。
    pub total_chars: i64,
    /// 会话文件字节数（磁盘占用）。
    pub file_bytes: i64,
}

/// 统计会话文件的水位：读文件一次，行级累计消息数与文本字符数。
///
/// 粗略估算：token ≈ 字符数 / 2（中英混合），前端据此展示「约 X tokens」。
/// 坏行容忍（与 parse_session_jsonl 一致）；文件不存在 / 读取失败 → None。
pub fn session_usage(path: &Path) -> Option<AgentSessionUsage> {
    let content = std::fs::read_to_string(path).ok()?;
    let file_bytes = std::fs::metadata(path).map(|m| m.len() as i64).unwrap_or(0);
    let mut message_count: i64 = 0;
    let mut total_chars: i64 = 0;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let value: serde_json::Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if value.get("type").and_then(|t| t.as_str()) != Some("message") {
            continue;
        }
        message_count += 1;
        if let Some(msg) = value.get("message") {
            accumulate_text(&msg["content"], &mut total_chars);
            if let Some(out) = msg.get("output").and_then(|o| o.as_str()) {
                total_chars += out.chars().count() as i64;
            }
        }
    }
    Some(AgentSessionUsage {
        message_count,
        total_chars,
        file_bytes,
    })
}

/// 累计 text / thinking 块的字符数（字符串或块数组两种形态都处理）。
fn accumulate_text(value: &serde_json::Value, count: &mut i64) {
    match value {
        serde_json::Value::String(s) => *count += s.chars().count() as i64,
        serde_json::Value::Array(items) => {
            for b in items {
                if let Some(t) = b.get("text").and_then(|x| x.as_str()) {
                    *count += t.chars().count() as i64;
                }
                if let Some(t) = b.get("thinking").and_then(|x| x.as_str()) {
                    *count += t.chars().count() as i64;
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造一条 message entry 的 JSON 行（直接拼字符串便于测试）。
    fn entry(id: &str, parent: Option<&str>, message: serde_json::Value) -> String {
        let parent = match parent {
            Some(p) => format!("\"parentId\":\"{}\",", p),
            None => "\"parentId\":null,".to_string(),
        };
        format!(
            "{{\"type\":\"message\",\"id\":\"{}\",{}\"timestamp\":\"2025-01-01T00:00:00.000Z\",\"message\":{}}}",
            id,
            parent,
            message
        )
    }

    fn user_msg(text: &str) -> serde_json::Value {
        serde_json::json!({ "role": "user", "content": text })
    }

    fn assistant_msg(blocks: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "role": "assistant",
            "content": blocks,
            "provider": "anthropic",
            "model": "claude-test",
            "stopReason": "stop"
        })
    }

    #[test]
    fn parses_linear_conversation() {
        let content = [
            r#"{"type":"session","version":3,"id":"u1","timestamp":"2025-01-01T00:00:00.000Z","cwd":"/work"}"#.to_string(),
            entry("a", None, user_msg("总结这个版本")),
            entry(
                "b",
                Some("a"),
                assistant_msg(serde_json::json!([
                    { "type": "thinking", "thinking": "先看实体" },
                    { "type": "text", "text": "好的，这是总结：" },
                    { "type": "toolCall", "id": "call_1", "name": "bash", "arguments": { "cmd": "ls" } }
                ])),
            ),
            entry(
                "c",
                Some("b"),
                serde_json::json!({
                    "role": "toolResult", "toolCallId": "call_1", "toolName": "bash",
                    "content": [{ "type": "text", "text": "src\n" }], "isError": false
                }),
            ),
            entry(
                "d",
                Some("c"),
                serde_json::json!({
                    "role": "bashExecution", "command": "ls", "output": "src\n",
                    "exitCode": 0, "cancelled": false, "truncated": false
                }),
            ),
        ]
        .join("\n");

        let messages = parse_session_jsonl(&content).unwrap();
        assert_eq!(messages.len(), 4);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].blocks, vec![AgentChatBlock::Text { text: "总结这个版本".into() }]);
        // assistant：thinking + text + toolCall 三个块
        assert_eq!(messages[1].role, "assistant");
        assert_eq!(messages[1].model.as_deref(), Some("claude-test"));
        assert_eq!(messages[1].blocks.len(), 3);
        assert!(matches!(&messages[1].blocks[0], AgentChatBlock::Thinking { text } if text == "先看实体"));
        assert!(matches!(&messages[1].blocks[2], AgentChatBlock::ToolCall { name, id, .. } if name == "bash" && id == "call_1"));
        // toolResult
        assert!(matches!(&messages[2].blocks[0], AgentChatBlock::ToolResult { tool_name, text, is_error, .. }
            if tool_name == "bash" && text == "src\n" && !is_error));
        // bashExecution
        assert!(matches!(&messages[3].blocks[0], AgentChatBlock::Bash { command, exit_code, .. }
            if command == "ls" && *exit_code == Some(0)));
    }

    #[test]
    fn branch_takes_leaf_path() {
        // a → b1（主分支）与 a → b2（分支），文件末尾是 b2 → c
        let content = [
            entry("a", None, user_msg("问题一")),
            entry("b1", Some("a"), assistant_msg(serde_json::json!([{ "type": "text", "text": "回答一" }]))),
            entry("b2", Some("a"), assistant_msg(serde_json::json!([{ "type": "text", "text": "回答二" }]))),
            entry("c", Some("b2"), user_msg("追问二")),
        ]
        .join("\n");
        let messages = parse_session_jsonl(&content).unwrap();
        // leaf 路径：a → b2 → c（b1 不在路径上）
        let texts: Vec<String> = messages
            .iter()
            .flat_map(|m| m.blocks.iter())
            .filter_map(|b| match b {
                AgentChatBlock::Text { text } => Some(text.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(texts, vec!["问题一".to_string(), "回答二".to_string(), "追问二".to_string()]);
    }

    #[test]
    fn tolerates_corrupt_lines() {
        let content = format!(
            "{}\n{}\n{}",
            entry("a", None, user_msg("你好")),
            r#"{"type":"message","id":"b","parentId":"a","timestamp":"2025-01-01T00:00:01.000Z","message":{"role":"assistant","content":"半"#,
            entry("c", Some("a"), assistant_msg(serde_json::json!([{ "type": "text", "text": "完整回复" }]))),
        );
        let messages = parse_session_jsonl(&content).unwrap();
        assert_eq!(messages.len(), 2);
        assert!(matches!(&messages[1].blocks[0], AgentChatBlock::Text { text } if text == "完整回复"));
    }

    #[test]
    fn empty_and_missing_handled() {
        assert!(parse_session_jsonl("").unwrap().is_empty());
        assert!(parse_session_jsonl("not json\n").unwrap().is_empty());
        assert!(parse_session_jsonl("{\"type\":\"custom\",\"id\":\"x\"}\n").unwrap().is_empty());
    }

    #[test]
    fn title_from_first_user_message() {
        let content = [
            r#"{"type":"session","version":3,"id":"u1","timestamp":"2025-01-01T00:00:00.000Z","cwd":"/work"}"#.to_string(),
            entry("a", None, user_msg("总结这个版本的变更")),
            entry("b", Some("a"), assistant_msg(serde_json::json!([{ "type": "text", "text": "好的" }]))),
        ]
        .join("\n");
        assert_eq!(session_title_from_jsonl(&content).as_deref(), Some("总结这个版本的变更"));
    }

    #[test]
    fn title_strips_skill_block_and_template() {
        // 用了 Skill 的会话：首条 user 消息被 pi 展开为 skill 全文 + 任务模板
        let skill_text = format!(
            "<skill name=\"review\">\n{}\n</skill>\n<subscription>订阅说明</subscription>\n<用户指令>\n  {}\n</用户指令>",
            "x".repeat(300),
            "帮我评审这次发版"
        );
        let content = [
            r#"{"type":"session","version":3}"#.to_string(),
            entry("a", None, serde_json::json!({ "role": "user", "content": skill_text })),
        ]
        .join("\n");
        assert_eq!(session_title_from_jsonl(&content).as_deref(), Some("帮我评审这次发版"));
    }

    #[test]
    fn title_truncates_to_40_chars() {
        let long = "指".repeat(60);
        let content = entry("a", None, user_msg(&long));
        let title = session_title_from_jsonl(&content).expect("title");
        assert_eq!(title.chars().count(), SESSION_TITLE_MAX_CHARS);
    }

    #[test]
    fn title_none_when_no_user_message() {
        let content = [
            entry("a", None, assistant_msg(serde_json::json!([{ "type": "text", "text": "hi" }]))),
        ]
        .join("\n");
        assert_eq!(session_title_from_jsonl(&content), None);
        assert_eq!(session_title_from_jsonl(""), None);
        assert_eq!(session_title_from_jsonl("not json"), None);
    }

    #[test]
    fn title_from_file_reads_only_head() {
        let dir = std::env::temp_dir().join(format!("relwatch-title-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("ws-test.jsonl");
        let content = format!(
            "{}\n{}\n",
            entry("a", None, user_msg("来自文件的标题")),
            entry("b", Some("a"), assistant_msg(serde_json::json!([{ "type": "text", "text": "回复" }]))),
        );
        std::fs::write(&path, &content).unwrap();
        assert_eq!(session_title_from_file(&path).as_deref(), Some("来自文件的标题"));
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn session_usage_counts_messages_and_chars() {
        let content = [
            entry("a", None, user_msg("第一轮问题")),
            entry("b", Some("a"), assistant_msg(serde_json::json!([
                { "type": "thinking", "thinking": "思考中" },
                { "type": "text", "text": "这是回答" }
            ]))),
            entry(
                "c",
                Some("b"),
                serde_json::json!({
                    "role": "bashExecution", "command": "ls", "output": "file1\nfile2\n",
                    "exitCode": 0, "cancelled": false, "truncated": false
                }),
            ),
        ]
        .join("\n");
        let dir = std::env::temp_dir().join(format!("relwatch-usage-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("ws-usage.jsonl");
        std::fs::write(&path, &content).unwrap();

        let usage = session_usage(&path).expect("usage");
        assert_eq!(usage.message_count, 3);
        // 第一轮问题(5) + 思考中(3) + 这是回答(4) + file1\nfile2\n(12)
        assert_eq!(usage.total_chars, 5 + 3 + 4 + 12);
        assert!(usage.file_bytes > 0);
        // 文件不存在 → None
        assert!(session_usage(&dir.join("ws-none.jsonl")).is_none());
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn message_run_id_defaults_to_none() {
        let content = entry("a", None, user_msg("你好"));
        let messages = parse_session_jsonl(&content).unwrap();
        assert_eq!(messages[0].run_id, None);
    }
}
