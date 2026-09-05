use crate::db;
use crate::db::usage::UsageStatRow;
use crate::db::ai_usage::AiUsageStats;
use crate::types::AppState;

/// 命令体内层逻辑：开关关闭时静默丢弃，不做任何写入（抽为函数便于测试）。
fn record_usage_inner(conn: &rusqlite::Connection, events: &[(String, u32)]) -> Result<(), String> {
    if !db::settings::get_setting_bool(conn, db::settings::KEY_ENABLE_USAGE_STATS, true)? {
        return Ok(());
    }
    db::usage::record_usage(conn, events)
}

/// 批量记录功能按钮点击统计。
/// 内部校验诊断开关（enable_usage_stats）：关闭时静默丢弃，不做任何写入。
#[tauri::command]

#[specta::specta]pub fn record_usage(
    state: tauri::State<AppState>,
    events: Vec<(String, u32)>,
) -> Result<(), String> {
    let conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;
    record_usage_inner(&conn, &events)
}

/// 查询使用统计（按累计次数降序）。仅开发者工具调用，不进入任何用户 UI。
/// `days` 提供时仅返回最近 N 天的数据（趋势窗口）。
#[tauri::command]

#[specta::specta]pub fn get_usage_stats(
    state: tauri::State<AppState>,
    days: Option<u32>,
) -> Result<Vec<UsageStatRow>, String> {
    let conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;
    db::usage::get_usage_stats(&conn, days)
}

/// 清空全部使用统计。
#[tauri::command]

#[specta::specta]pub fn clear_usage_stats(state: tauri::State<AppState>) -> Result<(), String> {
    let conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;
    db::usage::clear_usage_stats(&conn)
}

/// 查询 AI（摘要/翻译/语言检测/连接测试）token 用量聚合：
/// 一次返回逐日（热力图）、按监控源（表格）、按操作类型（饼图）三组。
/// `source_id` 为 None 时统计全部源；`days` 为 None 时统计全部历史。
#[tauri::command]

#[specta::specta]pub fn get_ai_usage_stats(
    state: tauri::State<AppState>,
    source_id: Option<i64>,
    days: Option<u32>,
) -> Result<AiUsageStats, String> {
    let conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;
    db::ai_usage::get_ai_usage_stats(&conn, source_id, days)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init::init_memory_db;

    /// 命令语义测试：开关关闭时静默丢弃（不写库、不报错）。
    #[test]
    fn test_record_usage_respects_enable_usage_stats_switch() {
        let conn = init_memory_db().unwrap();
        // 默认开启（get_setting_bool 默认值 true）：写入成功
        record_usage_inner(&conn, &[("source.add".into(), 1)]).unwrap();
        assert_eq!(db::usage::get_usage_stats(&conn, None).unwrap().len(), 1);

        // 关闭开关：静默丢弃
        db::settings::set_setting(&conn, db::settings::KEY_ENABLE_USAGE_STATS, "false").unwrap();
        record_usage_inner(&conn, &[("source.add".into(), 5)]).unwrap();
        let rows = db::usage::get_usage_stats(&conn, None).unwrap();
        assert_eq!(rows[0].total_count, 1, "开关关闭后不应再写入");

        // 重新开启：恢复写入
        db::settings::set_setting(&conn, db::settings::KEY_ENABLE_USAGE_STATS, "true").unwrap();
        record_usage_inner(&conn, &[("source.add".into(), 2)]).unwrap();
        let rows = db::usage::get_usage_stats(&conn, None).unwrap();
        assert_eq!(rows[0].total_count, 3);
    }

    /// 命令语义测试：空事件列表直接成功，不产生任何写入。
    #[test]
    fn test_record_usage_empty_events_noop() {
        let conn = init_memory_db().unwrap();
        record_usage_inner(&conn, &[]).unwrap();
        assert!(db::usage::get_usage_stats(&conn, None).unwrap().is_empty());
    }

    /// 命令链路集成：record → get（含 days 窗口）→ clear 全流程。
    #[test]
    fn test_usage_command_flow_roundtrip() {
        let conn = init_memory_db().unwrap();
        record_usage_inner(&conn, &[("source.check".into(), 2)]).unwrap();
        record_usage_inner(&conn, &[("release.open".into(), 1)]).unwrap();

        let all = db::usage::get_usage_stats(&conn, None).unwrap();
        let keys: Vec<&str> = all.iter().map(|r| r.key.as_str()).collect();
        assert_eq!(keys, vec!["source.check", "release.open"]);

        db::usage::clear_usage_stats(&conn).unwrap();
        assert!(db::usage::get_usage_stats(&conn, None).unwrap().is_empty());
    }
}
