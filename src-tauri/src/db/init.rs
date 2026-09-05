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
        // 启动清理：上次进程遗留的 pending/running run 置 cancelled（防永久悬挂）
        cleanup_stale_agent_runs(&conn).map_err(|e| e.to_string())?;
    }

    Ok(pool)
}

/// agent_runs 建表列定义（apply_schema 与重建型 Migration 共用，避免两份字面 SQL 脱节）。
/// status 带 CHECK 约束（防御非法状态值）；注意旧库（未走重建的）agent_runs 表没有该
/// CHECK，仅新建库与重建库生效——Migration 16 负责把存量库补齐。
///
/// `unknown` 是「结果未知」终态：终态事件丢失（`err.agent.end_lost`）或应用重启时
/// 未落终态的 run。语义上区别于 failed：任务**可能已经执行完成**，只是结果没能记录。
const AGENT_RUNS_COLUMNS: &str = "(
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_key TEXT NOT NULL,
            skill_path TEXT,
            entities TEXT NOT NULL DEFAULT '[]',
            instruction TEXT NOT NULL DEFAULT '',
            model TEXT,
            session_path TEXT,
            status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'running', 'success', 'failed', 'timeout', 'cancelled', 'unknown')),
            exit_code INTEGER,
            stdout TEXT,
            stderr TEXT,
            error TEXT,
            started_at TEXT,
            finished_at TEXT,
            created_at TEXT NOT NULL,
            files TEXT
        )";

/// agent_runs 的全部列名（按建表顺序），重建型 Migration 据此做列交集拷贝。
const AGENT_RUNS_COLUMN_NAMES: &[&str] = &[
    "id",
    "session_key",
    "skill_path",
    "entities",
    "instruction",
    "model",
    "session_path",
    "status",
    "exit_code",
    "stdout",
    "stderr",
    "error",
    "started_at",
    "finished_at",
    "created_at",
    "files",
];

/// agent_runs 会话键索引（列表/会话路径查询均按 session_key）。
const AGENT_RUNS_SESSION_INDEX: &str =
    "CREATE INDEX IF NOT EXISTS idx_agent_runs_session ON agent_runs(session_key);";

pub fn apply_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(&(
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

        -- AI（DeepSeek 兼容 API）token 用量：每次成功的 chat/completions 调用一行。
        -- 覆盖摘要 / 翻译 / 语言检测 / 连接测试；source_id 为 NULL 表示无源调用
        -- （连接测试），release_id 为 NULL 时同样可能（连接测试）。
        -- 成本预留设计：cache_hit / cache_miss 分列（DeepSeek 缓存命中单价约
        -- 为未命中的 1/10）+ model + day 已把金额核算的全部维度采齐，将来做
        -- 费用展示由前端按单价表现算，历史数据无需迁移。
        -- day 为本地日期（与 usage_stats 同口径，GROUP BY day 即日历聚合）。
        CREATE TABLE IF NOT EXISTS ai_usage (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            release_id INTEGER,
            source_id INTEGER,
            action TEXT NOT NULL,
            model TEXT,
            prompt_tokens INTEGER NOT NULL DEFAULT 0,
            completion_tokens INTEGER NOT NULL DEFAULT 0,
            cache_hit_tokens INTEGER NOT NULL DEFAULT 0,
            cache_miss_tokens INTEGER NOT NULL DEFAULT 0,
            estimated INTEGER NOT NULL DEFAULT 0,
            duration_ms INTEGER NOT NULL DEFAULT 0,
            day TEXT NOT NULL,
            created_at TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_ai_usage_day ON ai_usage(day);
        CREATE INDEX IF NOT EXISTS idx_ai_usage_source ON ai_usage(source_id);

        CREATE TABLE IF NOT EXISTS agent_runs "
            .to_string()
            + AGENT_RUNS_COLUMNS
            + ";\n\n        "
            + AGENT_RUNS_SESSION_INDEX),
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
        // 与 apply_schema 共用同一份列定义（含 CHECK 约束）；表重建后索引丢失需重建
        conn.execute_batch(
            &format!(
                "CREATE TABLE agent_runs {};\n{}",
                AGENT_RUNS_COLUMNS, AGENT_RUNS_SESSION_INDEX
            ),
        )?;
    }

    // ── Migration 15: 工作区提交可选模型（agent_runs.model）──
    // 提交时显式选择的 pi 模型（`{"provider":..,"model_id":..}` JSON），
    // None = 跟随 pi 当前/默认模型。老库无此列时补列（新库已含于 AGENT_RUNS_COLUMNS）。
    let has_run_model: bool = conn
        .prepare("SELECT 1 FROM pragma_table_info('agent_runs') WHERE name='model'")
        .and_then(|mut s| s.exists([]))
        .unwrap_or(false);
    if !has_run_model {
        conn.execute_batch("ALTER TABLE agent_runs ADD COLUMN model TEXT;")?;
    }

    // ── Migration 16: agent_runs 新增 `files` 列 + `unknown` 状态 ──
    // `files`：本次提交附带的本地文件绝对路径（JSON 数组，评审「本地文件/图片附件」）。
    // `unknown`：结果未知终态（终态事件丢失 / 应用重启时未落终态），与 failed 区分——
    // 它**可能已经执行完成**，误标 failed 会让用户以为没跑而重复提交。
    // 两者都无法用 ALTER TABLE 表达（CHECK 约束固化在表 DDL 里），故整表重建；
    // 重建走「新建 → 列交集拷贝 → 换名」，存量数据不丢。
    if agent_runs_needs_rebuild(conn)? {
        rebuild_agent_runs(conn)?;
    }

    Ok(())
}

/// agent_runs 是否需要重建（缺 `files` 列，或 status CHECK 未含 `unknown`）。
///
/// CHECK 约束不进 `pragma_table_info`，只能从 `sqlite_master.sql` 的建表 DDL 里
/// 做字面检测；DDL 读不到（表不存在）时按无需重建处理（apply_schema 后续会建新表）。
fn agent_runs_needs_rebuild(conn: &Connection) -> Result<bool> {
    let ddl: Option<String> = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='agent_runs'",
            [],
            |row| row.get(0),
        )
        .ok();
    let Some(ddl) = ddl else {
        return Ok(false);
    };
    Ok(!ddl.contains("'unknown'") || !table_has_column(conn, "agent_runs", "files"))
}

/// 表是否含指定列（`pragma_table_info` 探测；迁移期表结构不定，故不缓存）。
fn table_has_column(conn: &Connection, table: &str, column: &str) -> bool {
    conn.prepare(&format!(
        "SELECT 1 FROM pragma_table_info('{table}') WHERE name='{column}'"
    ))
    .and_then(|mut s| s.exists([]))
    .unwrap_or(false)
}

/// 重建 agent_runs（保留数据）：新表 → 列交集拷贝 → 删旧表 → 改名 → 重建索引。
///
/// 拷贝只取「新旧表都存在的列」：新增列（如 files）留 NULL，删除的列自然丢弃，
/// 因此对任意历史版本的表结构都安全。
fn rebuild_agent_runs(conn: &Connection) -> Result<()> {
    let shared: Vec<&str> = AGENT_RUNS_COLUMN_NAMES
        .iter()
        .copied()
        .filter(|c| table_has_column(conn, "agent_runs", c))
        .collect();
    if !shared.contains(&"id") {
        return Ok(()); // 结构异常（无 id 列）：放弃重建，交由上层报错
    }
    let cols = shared.join(", ");
    // unchecked_transaction：迁移期无法拿到 &mut Connection（migrate 与 init_pool 都只
    // 持有 &Connection），而此处是启动期单线程路径，独占性由调用点保证。
    // 用它把「建表 + 拷贝 + 换名 + 建索引」包成一个原子操作：中途失败不会留下
    // 半重建的 agent_runs（旧表已删、新表未就位的中间态）。
    let tx = conn.unchecked_transaction()?;
    tx.execute_batch(&format!("CREATE TABLE agent_runs_new {}", AGENT_RUNS_COLUMNS))?;
    tx.execute_batch(&format!(
        "INSERT INTO agent_runs_new ({cols}) SELECT {cols} FROM agent_runs;"
    ))?;
    tx.execute_batch("DROP TABLE agent_runs;")?;
    tx.execute_batch("ALTER TABLE agent_runs_new RENAME TO agent_runs;")?;
    tx.execute_batch(AGENT_RUNS_SESSION_INDEX)?;
    tx.commit()
}

/// 启动清理：把上次进程遗留的 pending / running run 批量置终态。
/// 调度器随进程消亡，这些 run 不会有终态写入，不清理则永远挂着。
/// 在应用启动（init_pool）时调用一次。
///
/// **两类遗留 run 的真实语义不同，分开归类（评审 3.4）**：
/// - **pending（无 started_at）**：从未被调度执行过 —— 确定没跑，置 `cancelled`。
/// - **running（有 started_at）**：已在 pi 里跑起来了，进程被强杀时**可能已经跑完**，
///   只是终态没来得及落库 —— 置 `unknown`（结果未知），而非「已取消」。
///
/// 此前一律置 cancelled 且文案为「未完成的提交已取消」，用户读到「已取消」会以为
/// 任务没执行、从而重跑（重复烧词元、重复副作用）；`unknown` + 对应文案把不确定性
/// 如实交还给用户，由他确认后再决定是否重跑。
pub fn cleanup_stale_agent_runs(conn: &Connection) -> Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    // pending（未启动）→ cancelled
    conn.execute(
        "UPDATE agent_runs
         SET status = 'cancelled', error = 'err.agent.startup_cleanup_pending', finished_at = ?1
         WHERE status IN ('pending', 'running') AND (started_at IS NULL OR started_at = '')",
        [&now],
    )?;
    // running（已启动，结果未知）→ unknown
    conn.execute(
        "UPDATE agent_runs
         SET status = 'unknown', error = 'err.agent.startup_cleanup_running', finished_at = ?1
         WHERE status IN ('pending', 'running') AND started_at IS NOT NULL AND started_at != ''",
        [&now],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::agent::NewRun;

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
        // Migration 15: agent_runs.model（提交可选模型）
        assert!(has_column(&conn, "agent_runs", "model"));
        assert!(!has_column(&conn, "agent_runs", "binding_id"));
        // Migration 16: agent_runs.files（本地文件附件）+ status 支持 'unknown'
        assert!(has_column(&conn, "agent_runs", "files"));
        let ddl: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type='table' AND name='agent_runs'",
                [],
                |row| row.get(0),
            )
            .expect("agent_runs DDL");
        assert!(ddl.contains("'unknown'"), "status CHECK 应含 'unknown'");

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

    /// 验证 Migration 16：旧版 agent_runs（无 files 列、status 无 'unknown'）
    /// 被重建为新结构，且**存量数据保留**。
    #[test]
    fn test_migration_16_rebuilds_agent_runs_keeping_rows() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        apply_schema(&conn).unwrap();
        // 旧结构：无 files 列、CHECK 不含 'unknown'（模拟 1.12.0 及更早）
        conn.execute_batch(
            "DROP TABLE agent_runs;
             CREATE TABLE agent_runs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_key TEXT NOT NULL,
                skill_path TEXT,
                entities TEXT NOT NULL DEFAULT '[]',
                instruction TEXT NOT NULL DEFAULT '',
                model TEXT,
                session_path TEXT,
                status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'running', 'success', 'failed', 'timeout', 'cancelled')),
                exit_code INTEGER,
                stdout TEXT,
                stderr TEXT,
                error TEXT,
                started_at TEXT,
                finished_at TEXT,
                created_at TEXT NOT NULL
            );
            INSERT INTO agent_runs (session_key, instruction, status, created_at)
            VALUES ('ws-old', '历史提交', 'success', '2025-01-01T00:00:00Z');",
        )
        .unwrap();

        migrate(&conn).unwrap();

        // 新结构：files 列 + status 含 'unknown'
        assert!(has_column(&conn, "agent_runs", "files"));
        let ddl: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type='table' AND name='agent_runs'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(ddl.contains("'unknown'"));
        // 存量数据保留
        let (instruction, files): (String, Option<String>) = conn
            .query_row(
                "SELECT instruction, files FROM agent_runs WHERE session_key='ws-old'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(instruction, "历史提交");
        assert!(files.is_none(), "新增列在旧行上应为 NULL");
        // 新状态可写入（CHECK 已放开）
        conn.execute(
            "UPDATE agent_runs SET status='unknown' WHERE session_key='ws-old'",
            [],
        )
        .expect("unknown 状态应可写入");
    }

    /// Migration 16 幂等：已含 files 与 unknown 的库不触发重建（数据不受影响）。
    #[test]
    fn test_migration_16_idempotent() {
        let conn = init_memory_db().unwrap();
        conn.execute(
            "INSERT INTO agent_runs (session_key, instruction, status, created_at)
             VALUES ('ws-1', '指令', 'success', '2025-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        migrate(&conn).unwrap();
        migrate(&conn).unwrap();
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM agent_runs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1, "重复迁移不应丢数据也不应重复插入");
    }

    /// 启动清理按语义分流：未启动 → cancelled，已启动 → unknown。
    #[test]
    fn test_cleanup_stale_agent_runs_splits_by_started_at() {
        let conn = init_memory_db().unwrap();
        let pending = crate::db::agent::create_run(&conn, &NewRun { session_key: "ws-p", skill_path: None, entities: &[], instruction: "排队中", model: None, session_path: None, files: None })
        .unwrap();
        let started = crate::db::agent::create_run(&conn, &NewRun { session_key: "ws-r", skill_path: None, entities: &[], instruction: "执行中", model: None, session_path: None, files: None })
        .unwrap();
        crate::db::agent::mark_run_started(&conn, started).unwrap();

        cleanup_stale_agent_runs(&conn).unwrap();

        let (p_status, p_err): (String, Option<String>) = conn
            .query_row(
                "SELECT status, error FROM agent_runs WHERE id=?1",
                [pending],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        // 从未启动：确定没跑 → cancelled
        assert_eq!(p_status, "cancelled");
        assert_eq!(p_err.as_deref(), Some("err.agent.startup_cleanup_pending"));

        let (s_status, s_err): (String, Option<String>) = conn
            .query_row(
                "SELECT status, error FROM agent_runs WHERE id=?1",
                [started],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        // 已启动：可能已跑完 → unknown（而非「已取消」，避免用户误以为没执行而重跑）
        assert_eq!(s_status, "unknown");
        assert_eq!(s_err.as_deref(), Some("err.agent.startup_cleanup_running"));

        // 两者都带终态时间，不再悬挂
        let hanging: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM agent_runs WHERE status IN ('pending','running')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(hanging, 0);
    }
}
