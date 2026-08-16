//! pi 会话 JSONL 解析 —— Agent 工作区聊天渲染的数据源。
//!
//! 工作区每次提交都以 `pi -p --session <file>` 运行，pi 会把完整对话
//! （user / assistant / toolResult / bashExecution 等）逐行 append 到会话文件。
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
        let content = vec![
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
        let content = vec![
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
}
