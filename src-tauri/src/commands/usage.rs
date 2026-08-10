use crate::db;
use crate::db::usage::UsageStatRow;
use crate::types::AppState;

/// 批量记录功能按钮点击统计。
/// 内部校验诊断开关（enable_usage_stats）：关闭时静默丢弃，不做任何写入。
#[tauri::command]
pub fn record_usage(
    state: tauri::State<AppState>,
    events: Vec<(String, u32)>,
) -> Result<(), String> {
    let conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;
    if !db::settings::get_setting_bool(&conn, db::settings::KEY_ENABLE_USAGE_STATS, true)? {
        return Ok(());
    }
    db::usage::record_usage(&conn, &events)
}

/// 查询使用统计（按累计次数降序）。仅开发者工具调用，不进入任何用户 UI。
/// `days` 提供时仅返回最近 N 天的数据（趋势窗口）。
#[tauri::command]
pub fn get_usage_stats(
    state: tauri::State<AppState>,
    days: Option<u32>,
) -> Result<Vec<UsageStatRow>, String> {
    let conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;
    db::usage::get_usage_stats(&conn, days)
}

/// 清空全部使用统计。
#[tauri::command]
pub fn clear_usage_stats(state: tauri::State<AppState>) -> Result<(), String> {
    let conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;
    db::usage::clear_usage_stats(&conn)
}
