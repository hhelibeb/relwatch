use crate::db;
use crate::types::{AppState, LogSearchResult};
use serde_json::json;

#[tauri::command]

#[specta::specta]pub async fn search_logs(
    state: tauri::State<'_, AppState>,
    keyword: String,
    page: i64,
    page_size: i64,
    level: Option<String>,
) -> Result<LogSearchResult, String> {
    // 同步 SQLite I/O 放进 spawn_blocking，避免在 Tauri 主线程冻结 UI
    let pool = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let conn = pool.get().map_err(|e| format!("err.db_connect|{}", e))?;
        let (entries, total) = db::logs::search_logs(&conn, &keyword, level.as_deref(), page, page_size)?;
        Ok(LogSearchResult {
            entries,
            total,
            page,
            page_size,
        })
    })
    .await
    .map_err(|e| format!("err.task_failed|search_logs|{}", e))?
}

#[tauri::command]

#[specta::specta]pub async fn clear_logs(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let pool = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let conn = pool.get().map_err(|e| format!("err.db_connect|{}", e))?;
        db::logs::clear_logs(&conn)?;
        db::logs::write_log_key(&conn, "INFO", "log.cleared", "{}");
        Ok::<_, String>(())
    })
    .await
    .map_err(|e| format!("err.task_failed|clear_logs|{}", e))?
}

/// 前端未捕获异常上报（V2 全局兜底通道）。
///
/// 背景：release 版没有控制台，`app.config.errorHandler` / `unhandledrejection` 里
/// 的异常原本既无提示、也无落库，用户报障时无从查起（“点了没反应且查不到”）。
/// 前端侧 `src/api/report-error.ts` 是唯一调用方——它负责节流（同一 key 60s 内只报一条）、
/// 截断（堆栈 ≤2KB）与 toast 提示；本命令只负责落库。
///
/// **命名约束**：`message_key` 由前端传入，**不得以 err. 开头**。
/// `src/__tests__/i18n-keys.test.ts` 会扫描 Rust 生产代码中以双引号开头的 err. 前缀
/// 字面量，并要求两个字典都有翻译；若这种写法出现在此处，会被当成一个未翻译的 key
/// 而卡住 CI（本注释特意不写出该字面量，以免自投罗网）。
/// 前端现用 `ui.vue_error` / `ui.unhandled_rejection` / `ui.window_error`。
///
/// 落库前经 `db::logs::write_log_key` 的默认脱敏（V28），无需在此重复处理；
/// DB 写入失败时该函数会降级写 `logs/fallback.log`（V23）。
#[tauri::command]

#[specta::specta]pub async fn report_frontend_error(
    state: tauri::State<'_, AppState>,
    message_key: String,
    detail: String,
    info: Option<String>,
) -> Result<(), String> {
    let pool = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let conn = pool.get().map_err(|e| format!("err.db_connect|{}", e))?;
        let mut args = json!({ "error": detail });
        if let Some(info) = info.filter(|i| !i.is_empty()) {
            args["info"] = json!(info);
        }
        db::logs::write_log_key(&conn, "ERROR", &message_key, &args.to_string());
        Ok::<_, String>(())
    })
    .await
    .map_err(|e| format!("err.task_failed|report_frontend_error|{}", e))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init::init_memory_db;

    #[test]
    fn test_get_logs_returns_written_logs() {
        let conn = init_memory_db().unwrap();
        db::logs::write_log_key(&conn, "INFO", "test.message", "{\"key\":\"val\"}");
        db::logs::write_log_key(&conn, "WARN", "test.warn", "{}");

        let logs = db::logs::get_logs(&conn, 10).unwrap();
        assert_eq!(logs.len(), 2);
        assert!(logs.iter().any(|l| l.message_key.as_deref() == Some("test.message")));
        assert!(logs.iter().any(|l| l.message_key.as_deref() == Some("test.warn")));
    }

    /// `report_frontend_error` 的 args 形状与可搜索性（V2）。
    /// 命令体需要 tauri State，无法在单测里直接调用；这里复刻其 args 构造，
    /// 锁住「日志页搜错误关键词能命中」这一用户可见契约。
    #[test]
    fn test_report_frontend_error_args_are_searchable() {
        let conn = init_memory_db().unwrap();
        crate::db::settings::set_setting(&conn, crate::db::settings::KEY_LANGUAGE, "zh-CN").unwrap();

        let args = json!({ "error": "Error: boom
    at App.vue:1:1" }).to_string();
        db::logs::write_log_key(&conn, "ERROR", "ui.unhandled_rejection", &args);

        let (entries, total) = db::logs::search_logs(&conn, "boom", None, 1, 10).unwrap();
        assert_eq!(total, 1, "日志页搜 boom 应命中");
        assert_eq!(entries[0].level, "ERROR");
        assert_eq!(entries[0].message_key.as_deref(), Some("ui.unhandled_rejection"));
        assert!(entries[0].rendered_message.as_deref().unwrap().contains("boom"));
    }

    #[test]
    fn test_get_logs_respects_limit() {
        let conn = init_memory_db().unwrap();
        db::logs::write_log_key(&conn, "INFO", "msg1", "{}");
        db::logs::write_log_key(&conn, "INFO", "msg2", "{}");
        db::logs::write_log_key(&conn, "INFO", "msg3", "{}");

        // Limit 2 should only return 2 newest
        let logs = db::logs::get_logs(&conn, 2).unwrap();
        assert_eq!(logs.len(), 2);
    }

    #[test]
    fn test_clear_logs_empties_and_writes_log() {
        let conn = init_memory_db().unwrap();
        db::logs::write_log_key(&conn, "INFO", "test.message", "{}");
        assert!(!db::logs::get_logs(&conn, 10).unwrap().is_empty());

        // Simulate clear_logs internal logic
        db::logs::clear_logs(&conn).unwrap();
        db::logs::write_log_key(&conn, "INFO", "log.cleared", "{}");

        // After clear, only the "log.cleared" entry should exist
        let logs = db::logs::get_logs(&conn, 10).unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].message_key.as_deref(), Some("log.cleared"));
    }
}
