use rusqlite::{Connection, Result};
use std::path::PathBuf;

pub fn app_data_dir() -> PathBuf {
    dirs::data_dir()
        .expect("Failed to get app data dir")
        .join("RelWatch")
}

pub fn db_path() -> PathBuf {
    app_data_dir().join("database.db")
}

/// 仅测试使用的内存库（生产走 `db_path` + 连接池）。
#[cfg(test)]
pub fn init_memory_db() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA foreign_keys=ON;")?;
    apply_schema(&conn)?;
    migrate(&conn)?;
    Ok(conn)
}

/// 仅测试使用的内存连接池。
#[cfg(test)]
pub fn init_memory_pool(
) -> Result<r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>, String> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    let name = format!(
        "file:relwatch_test_{}?mode=memory&cache=shared",
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    );
    let manager = r2d2_sqlite::SqliteConnectionManager::file(&name);
    let pool = r2d2::Pool::builder()
        .max_size(2)
        .build(manager)
        .map_err(|e| e.to_string())?;
    {
        let conn = pool.get().map_err(|e| e.to_string())?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")
            .map_err(|e| e.to_string())?;
        apply_schema(&conn).map_err(|e| e.to_string())?;
        migrate(&conn).map_err(|e| e.to_string())?;
    }
    Ok(pool)
}

pub fn init_pool(
) -> Result<r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>, String> {
    let dir = app_data_dir();
    std::fs::create_dir_all(&dir).expect("Failed to create app data dir");

    let manager = r2d2_sqlite::SqliteConnectionManager::file(db_path())
        .with_init(|conn| {
            conn.execute_batch(
                "PRAGMA journal_mode=WAL;
                 PRAGMA wal_autocheckpoint=1000;
                 PRAGMA foreign_keys=ON;",
            )
        });

    // 容量与 poll MAX_CONCURRENCY(10) 对齐并留余量，覆盖 spawn_blocking 闭包与
    // collect_pending_and_notify 并发取连接；避免 spawn 内 pool.get() 排队。
    let pool = r2d2::Pool::builder()
        .max_size(16)
        .build(manager)
        .map_err(|e| e.to_string())?;

    {
        let conn = pool.get().map_err(|e| e.to_string())?;
        apply_schema(&conn).map_err(|e| e.to_string())?;
        migrate(&conn).map_err(|e| e.to_string())?;
    }

    Ok(pool)
}

pub fn apply_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS sources (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            source_type TEXT NOT NULL,
            owner TEXT NOT NULL,
            repo TEXT NOT NULL,
            poll_interval_minutes INTEGER NOT NULL DEFAULT 30,
            enabled INTEGER NOT NULL DEFAULT 1,
            last_checked_at TEXT,
            last_check_status TEXT NOT NULL DEFAULT 'unknown',
            last_check_message TEXT,
            consecutive_failures INTEGER NOT NULL DEFAULT 0,
            last_new_count INTEGER NOT NULL DEFAULT 0,
            muted INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            UNIQUE(source_type, owner, repo)
        );

        CREATE TABLE IF NOT EXISTS releases (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            source_id INTEGER NOT NULL,
            tag_name TEXT NOT NULL,
            release_name TEXT NOT NULL,
            html_url TEXT NOT NULL,
            published_at TEXT NOT NULL,
            prerelease INTEGER NOT NULL DEFAULT 0,
            body TEXT,
            detected_at TEXT NOT NULL,
            retry_count INTEGER NOT NULL DEFAULT 0,
            UNIQUE(source_id, tag_name),
            FOREIGN KEY (source_id) REFERENCES sources(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS notification_state (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            release_id INTEGER NOT NULL UNIQUE,
            status TEXT NOT NULL DEFAULT 'pending',
            snooze_until TEXT,
            last_notified_at TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            FOREIGN KEY (release_id) REFERENCES releases(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS app_settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS logs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            level TEXT NOT NULL,
            message TEXT NOT NULL,
            created_at TEXT NOT NULL
        );

        -- 诊断统计：功能按钮点击次数（按天分桶）。
        -- 纯本地记录，不上传；随数据库备份导出/恢复；SUM(count) 即累计，GROUP BY day 看趋势。
        CREATE TABLE IF NOT EXISTS usage_stats (
            key TEXT NOT NULL,
            day TEXT NOT NULL,
            count INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (key, day)
        );

        CREATE TABLE IF NOT EXISTS agent_runs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_key TEXT NOT NULL,
            skill_path TEXT,
            entities TEXT NOT NULL DEFAULT '[]',
            instruction TEXT NOT NULL DEFAULT '',
            session_path TEXT,
            status TEXT NOT NULL DEFAULT 'pending',
            exit_code INTEGER,
            stdout TEXT,
            stderr TEXT,
            error TEXT,
            started_at TEXT,
            finished_at TEXT,
            created_at TEXT NOT NULL
        );",
    )?;
    Ok(())
}

pub fn migrate(conn: &Connection) -> Result<()> {
    let has_summary: bool = conn
        .prepare("SELECT 1 FROM pragma_table_info('releases') WHERE name='ai_summary'")
        .and_then(|mut s| s.exists([]))
        .unwrap_or(false);
    if !has_summary {
        conn.execute_batch(
            "ALTER TABLE releases ADD COLUMN ai_summary TEXT;
             ALTER TABLE releases ADD COLUMN ai_importance TEXT;",
        )?;
    }
    let has_msg_key: bool = conn
        .prepare("SELECT 1 FROM pragma_table_info('logs') WHERE name='message_key'")
        .and_then(|mut s| s.exists([]))
        .unwrap_or(false);
    if !has_msg_key {
        conn.execute_batch(
            "ALTER TABLE logs ADD COLUMN message_key TEXT;
             ALTER TABLE logs ADD COLUMN message_args TEXT;",
        )?;
    }
    let has_source_health: bool = conn
        .prepare("SELECT 1 FROM pragma_table_info('sources') WHERE name='last_checked_at'")
        .and_then(|mut s| s.exists([]))
        .unwrap_or(false);
    if !has_source_health {
        conn.execute_batch(
            "ALTER TABLE sources ADD COLUMN last_checked_at TEXT;
             ALTER TABLE sources ADD COLUMN last_check_status TEXT NOT NULL DEFAULT 'unknown';
             ALTER TABLE sources ADD COLUMN last_check_message TEXT;
             ALTER TABLE sources ADD COLUMN consecutive_failures INTEGER NOT NULL DEFAULT 0;
             ALTER TABLE sources ADD COLUMN last_new_count INTEGER NOT NULL DEFAULT 0;",
        )?;
    }
    let has_desc: bool = conn
        .prepare("SELECT 1 FROM pragma_table_info('sources') WHERE name='description'")
        .and_then(|mut s| s.exists([]))
        .unwrap_or(false);
    if !has_desc {
        conn.execute_batch("ALTER TABLE sources ADD COLUMN description TEXT")?;
    }

    // ── Migration 5: retry_count on releases ──
    let has_retry_count: bool = conn
        .prepare("SELECT 1 FROM pragma_table_info('releases') WHERE name='retry_count'")
        .and_then(|mut s| s.exists([]))
        .unwrap_or(false);
    if !has_retry_count {
        conn.execute_batch(
            "ALTER TABLE releases ADD COLUMN retry_count INTEGER NOT NULL DEFAULT 0;"
        )?;
    }

    // ── Migration 6: last_notified_at on notification_state ──
    let has_last_notified: bool = conn
        .prepare("SELECT 1 FROM pragma_table_info('notification_state') WHERE name='last_notified_at'")
        .and_then(|mut s| s.exists([]))
        .unwrap_or(false);
    if !has_last_notified {
        conn.execute_batch(
            "ALTER TABLE notification_state ADD COLUMN last_notified_at TEXT;"
        )?;
    }

    // ── Migration 7: muted on sources ──
    let has_muted: bool = conn
        .prepare("SELECT 1 FROM pragma_table_info('sources') WHERE name='muted'")
        .and_then(|mut s| s.exists([]))
        .unwrap_or(false);
    if !has_muted {
        conn.execute_batch(
            "ALTER TABLE sources ADD COLUMN muted INTEGER NOT NULL DEFAULT 0;"
        )?;
    }

    // ── Migration 8: rendered_message on logs ──
    let has_rendered: bool = conn
        .prepare("SELECT 1 FROM pragma_table_info('logs') WHERE name='rendered_message'")
        .and_then(|mut s| s.exists([]))
        .unwrap_or(false);
    if !has_rendered {
        // 创建新列
        conn.execute_batch(
            "ALTER TABLE logs ADD COLUMN rendered_message TEXT;"
        )?;

        // 一次性清理：之前错误写入的 raw template（含未替换的 {key} 占位符）
        // 仅在首次添加列时执行，避免重复 NULL → backfill → 再次 NULL 的循环
        let _ = conn.execute(
            "UPDATE logs SET rendered_message = NULL WHERE rendered_message LIKE '%{%}%'",
            [],
        );

        // 一次性回填已有日志的 rendered_message
        match super::logs::backfill_rendered_messages(conn) {
            Ok(n) if n > 0 => {
                log::info!("已回填 {} 条日志的 rendered_message", n);
            }
            Ok(_) => {}
            Err(e) => {
                log::error!("回填 rendered_message 失败: {}", e);
            }
        }
    }

    // ── Migration 9: body_translated + translate_retry_count on releases ──
    // 用于 AI 翻译 release note 全文功能。
    let has_body_translated: bool = conn
        .prepare("SELECT 1 FROM pragma_table_info('releases') WHERE name='body_translated'")
        .and_then(|mut s| s.exists([]))
        .unwrap_or(false);
    if !has_body_translated {
        conn.execute_batch(
            "ALTER TABLE releases ADD COLUMN body_translated TEXT;
             ALTER TABLE releases ADD COLUMN translate_retry_count INTEGER NOT NULL DEFAULT 0;",
        )?;
    }

    // ── Migration 10: extra_metadata on releases ──
    // 用于 HuggingFace 模型元数据（pipeline_tag/downloads/likes/gated/tags 等），
    // body 列改为存储模型 README（人类可读内容）。
    let has_extra_metadata: bool = conn
        .prepare("SELECT 1 FROM pragma_table_info('releases') WHERE name='extra_metadata'")
        .and_then(|mut s| s.exists([]))
        .unwrap_or(false);
    if !has_extra_metadata {
        conn.execute_batch(
            "ALTER TABLE releases ADD COLUMN extra_metadata TEXT;",
        )?;
    }

    // ── Migration 11: config on sources ──
    // 源级附加配置（JSON），目前用于 YouTube 订阅内容类型（视频/直播/帖子）。
    let has_source_config: bool = conn
        .prepare("SELECT 1 FROM pragma_table_info('sources') WHERE name='config'")
        .and_then(|mut s| s.exists([]))
        .unwrap_or(false);
    if !has_source_config {
        conn.execute_batch(
            "ALTER TABLE sources ADD COLUMN config TEXT;",
        )?;
    }

    // ── Migration 14: Agent 全局化重构 ──
    // 逐源绑定模型（source_agent_bindings + agent_runs 含 binding_id）废弃，
    // 改为全局 Agent 配置（app_settings key）+ 工作区会话提交记录（agent_runs 重建）。
    // 该模型随分支引入且未发布，旧表数据直接丢弃重建，无迁移成本。
    let has_bindings_table: bool = conn
        .prepare("SELECT 1 FROM sqlite_master WHERE type='table' AND name='source_agent_bindings'")
        .and_then(|mut s| s.exists([]))
        .unwrap_or(false);
    if has_bindings_table {
        conn.execute_batch("DROP TABLE source_agent_bindings;")?;
    }
    let has_old_runs: bool = conn
        .prepare("SELECT 1 FROM pragma_table_info('agent_runs') WHERE name='binding_id'")
        .and_then(|mut s| s.exists([]))
        .unwrap_or(false);
    if has_old_runs {
        conn.execute_batch("DROP TABLE agent_runs;")?;
        conn.execute_batch(
            "CREATE TABLE agent_runs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_key TEXT NOT NULL,
                skill_path TEXT,
                entities TEXT NOT NULL DEFAULT '[]',
                instruction TEXT NOT NULL DEFAULT '',
                session_path TEXT,
                status TEXT NOT NULL DEFAULT 'pending',
                exit_code INTEGER,
                stdout TEXT,
                stderr TEXT,
                error TEXT,
                started_at TEXT,
                finished_at TEXT,
                created_at TEXT NOT NULL
            );",
        )?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 验证 migrate() 可重复调用（幂等性）
    #[test]
    fn test_migrate_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        apply_schema(&conn).unwrap();
        migrate(&conn).unwrap();
        // 第二次调用不应报错
        migrate(&conn).unwrap();
    }

    /// 验证所有 migration 添加的列在初始化后均存在
    #[test]
    fn test_all_migration_columns_exist() {
        let conn = init_memory_db().unwrap();

        // Migration 1: releases.ai_summary, releases.ai_importance
        assert!(has_column(&conn, "releases", "ai_summary"));
        assert!(has_column(&conn, "releases", "ai_importance"));

        // Migration 2: logs.message_key, logs.message_args
        assert!(has_column(&conn, "logs", "message_key"));
        assert!(has_column(&conn, "logs", "message_args"));

        // Migration 3: sources.last_checked_at, last_check_status, etc.
        assert!(has_column(&conn, "sources", "last_checked_at"));
        assert!(has_column(&conn, "sources", "last_check_status"));
        assert!(has_column(&conn, "sources", "last_check_message"));
        assert!(has_column(&conn, "sources", "consecutive_failures"));
        assert!(has_column(&conn, "sources", "last_new_count"));

        // Migration 4: sources.description
        assert!(has_column(&conn, "sources", "description"));

        // Migration 5: releases.retry_count
        assert!(has_column(&conn, "releases", "retry_count"));

        // Migration 6: notification_state.last_notified_at
        assert!(has_column(&conn, "notification_state", "last_notified_at"));

        // Migration 7: sources.muted
        assert!(has_column(&conn, "sources", "muted"));

        // Migration 8: logs.rendered_message
        assert!(has_column(&conn, "logs", "rendered_message"));

        // Migration 9: releases.body_translated, releases.translate_retry_count
        assert!(has_column(&conn, "releases", "body_translated"));
        assert!(has_column(&conn, "releases", "translate_retry_count"));

        // Migration 10: releases.extra_metadata
        assert!(has_column(&conn, "releases", "extra_metadata"));

        // Migration 11: sources.config
        assert!(has_column(&conn, "sources", "config"));

        // Migration 14: Agent 全局化 —— 旧绑定表已删除，agent_runs 为工作区提交记录
        let has_bindings_table: bool = conn
            .prepare("SELECT 1 FROM sqlite_master WHERE type='table' AND name='source_agent_bindings'")
            .and_then(|mut s| s.exists([]))
            .unwrap_or(false);
        assert!(!has_bindings_table, "source_agent_bindings 表应已删除");
        assert!(has_column(&conn, "agent_runs", "session_key"));
        assert!(has_column(&conn, "agent_runs", "entities"));
        assert!(has_column(&conn, "agent_runs", "instruction"));
        assert!(!has_column(&conn, "agent_runs", "binding_id"));

        // usage_stats 表（Migration 12：诊断统计）
        let has_table: bool = conn
            .prepare("SELECT 1 FROM sqlite_master WHERE type='table' AND name='usage_stats'")
            .and_then(|mut s| s.exists([]))
            .unwrap_or(false);
        assert!(has_table, "usage_stats 表应存在");
        assert!(has_column(&conn, "usage_stats", "key"));
        assert!(has_column(&conn, "usage_stats", "day"));
        assert!(has_column(&conn, "usage_stats", "count"));
    }

    fn has_column(conn: &Connection, table: &str, column: &str) -> bool {
        conn
            .prepare(&format!("SELECT 1 FROM pragma_table_info('{}') WHERE name='{}'", table, column))
            .and_then(|mut s| s.exists([]))
            .unwrap_or(false)
    }

    /// 验证 init_memory_pool 正常创建可用的连接池
    #[test]
    fn test_init_memory_pool_usable() {
        let pool = init_memory_pool().unwrap();
        let conn = pool.get().unwrap();
        // 能在生成的连接上执行查询
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM sources", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    /// 验证 Migration 14：旧绑定模型库 → 新工作区模型，旧表被清理重建
    #[test]
    fn test_migration_14_drops_old_binding_schema() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        apply_schema(&conn).unwrap();
        // 模拟旧版库结构：binding 表 + 旧 agent_runs（含 binding_id）
        conn.execute_batch(
            "CREATE TABLE source_agent_bindings (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                source_id INTEGER NOT NULL UNIQUE,
                agent_type TEXT NOT NULL DEFAULT 'pi',
                skill_paths TEXT NOT NULL DEFAULT '[]',
                trigger_mode TEXT NOT NULL DEFAULT 'manual',
                delay_seconds INTEGER NOT NULL DEFAULT 0,
                timeout_seconds INTEGER NOT NULL DEFAULT 300,
                working_dir TEXT,
                extra_args TEXT,
                enabled INTEGER NOT NULL DEFAULT 1,
                save_session INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            DROP TABLE agent_runs;
            CREATE TABLE agent_runs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                binding_id INTEGER NOT NULL,
                source_id INTEGER NOT NULL,
                release_id INTEGER,
                skill_path TEXT,
                session_path TEXT,
                trigger TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending',
                exit_code INTEGER,
                stdout TEXT,
                stderr TEXT,
                error TEXT,
                started_at TEXT,
                finished_at TEXT,
                created_at TEXT NOT NULL
            );",
        )
        .unwrap();

        migrate(&conn).unwrap();

        let has_bindings: bool = conn
            .prepare("SELECT 1 FROM sqlite_master WHERE type='table' AND name='source_agent_bindings'")
            .and_then(|mut s| s.exists([]))
            .unwrap_or(false);
        assert!(!has_bindings);
        assert!(has_column(&conn, "agent_runs", "session_key"));
        assert!(has_column(&conn, "agent_runs", "entities"));
        assert!(!has_column(&conn, "agent_runs", "binding_id"));
    }
}
