use rusqlite::backup::Backup;
use rusqlite::Connection;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tauri::Manager;
use tauri_plugin_dialog::DialogExt;

/// 同目录临时文件名：`<目标名>.tmp-<pid>`。
///
/// 必须与目标**同目录**（同卷）——`rename` 跨卷会失败；带 pid 避免同机多实例互踩。
fn sibling_temp_path(target: &Path) -> PathBuf {
    let mut name = target.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".tmp-{}", std::process::id()));
    target.with_file_name(name)
}

/// 把 `conn` 导出为紧凑的独立 SQLite 副本（`VACUUM INTO`）并写入 `target`。
///
/// ## 为什么不直接 `VACUUM INTO '<target>'`
///
/// `VACUUM INTO` 要求目标**不存在或为 0 字节**：目标已存在且有内容时 SQLite 直接报
/// `file is not a database`（实测 bundled SQLite 3.49）。而保存对话框虽然默认给
/// 带时间戳的文件名，用户仍可选中一个旧备份并确认「覆盖」——此时导出会以一个与真实
/// 原因毫不相干的错误失败（用户只看到「备份导出失败: file is not a database」）。
///
/// ## 做法：先导出到同目录临时文件，成功后再改名覆盖
///
/// - 失败时**旧备份原样保留**（“先删旧再写”会在中途失败时把备份弄丢）；
/// - `rename` 在 Windows 上等价 `MOVEFILE_REPLACE_EXISTING`，具备覆盖语义；
/// - 临时文件中途失败/rename 失败均清理，不留下垃圾文件。
///
/// ## 用绑定参数而非字符串拼接
///
/// `VACUUM INTO ?1` 的参数是一个**表达式**，实测绑定参数可用，于是不再需要手工转义。
/// 原实现拼 `.replace('\'', "''").replace('\\', "\\\\")`，其中把反斜杠加倍是
/// **没有意义的**（SQLite 字符串字面量不处理 `\` 转义，也不会把它还原成单个）——
/// 实测它在 Windows 上不造成故障，只是因为系统会宽容归并重复的路径分隔符（写出的仍是
/// 预期的 `backup.db`），**并非因为转义正确**。改用参数绑定后不再依赖这个巧合，
/// 也避开了「路径含单引号时拼串是否可靠」这类只能靠人推的细节。
fn vacuum_into(conn: &Connection, target: &Path) -> Result<(), String> {
    let tmp = sibling_temp_path(target);
    // 上次异常退出可能留下同名临时文件——VACUUM INTO 同样不接受已存在的文件
    let _ = std::fs::remove_file(&tmp);

    let tmp_str = tmp.to_string_lossy();
    if let Err(e) = conn.execute("VACUUM INTO ?1", rusqlite::params![tmp_str.as_ref()]) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("err.backup_export_failed|{}", e));
    }

    std::fs::rename(&tmp, target).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("err.backup_export_failed|{}", e)
    })
}

/// 验证指定路径的文件是有效的 SQLite 数据库（检查魔数前 16 字节）
pub fn validate_sqlite_file(path: &str) -> Result<(), String> {
    let header = std::fs::read(path).map_err(|e| format!("err.backup_read_failed|{}", e))?;
    if header.len() < 16 || &header[..16] != b"SQLite format 3\0" {
        return Err("err.backup_invalid_file".to_string());
    }
    Ok(())
}

/// 将回调式 dialog 转换为 async
async fn save_file_dialog(app: &tauri::AppHandle) -> Option<tauri_plugin_dialog::FilePath> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter("Database", &["db"])
        .set_file_name(format!(
            "relwatch-backup.{}.{}.db",
            chrono::Local::now().format("%Y%m%d-%H%M%S"),
            hostname::get().map(|h| h.to_string_lossy().to_string()).unwrap_or_else(|_| "unknown".to_string())
        ))
        .save_file(move |path| {
            let _ = tx.send(path);
        });
    rx.await.unwrap_or(None)
}

/// 将回调式 open dialog 转换为 async
async fn open_file_dialog(app: &tauri::AppHandle) -> Option<tauri_plugin_dialog::FilePath> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter("Database", &["db"])
        .pick_file(move |path| {
            let _ = tx.send(path);
        });
    rx.await.unwrap_or(None)
}

#[tauri::command]

#[specta::specta]pub async fn export_backup(app: tauri::AppHandle) -> Result<String, String> {
    let path = save_file_dialog(&app).await;
    let path = match path {
        Some(p) => p,
        None => return Err("err.backup_cancelled_export".to_string()),
    };

    // FilePath 在桌面端恒为 Path 变体；此处仍不用 unwrap——路径形态意外时
    // 报可读错误，比 panic 掉整个命令好
    let Some(target) = path.as_path() else {
        return Err("err.backup_export_failed|path is not a filesystem path".to_string());
    };
    let path_str = target.to_string_lossy().to_string();

    let state = app.state::<crate::types::AppState>();
    let conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;

    // WAL checkpoint 确保数据完整性
    conn
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .map_err(|e| format!("err.backup_wal_checkpoint_failed|{}", e))?;

    // 导出紧凑副本（含「目标已存在 / 路径转义」处理，见 vacuum_into 文档）
    vacuum_into(&conn, target)?;

    if let Ok(conn) = state.db.get() {
        crate::db::logs::write_log_key(&conn, "INFO", "backup.exported", &json!({"path": &path_str}).to_string());
    }

    Ok(path_str)
}

#[tauri::command]

#[specta::specta]pub async fn import_backup(app: tauri::AppHandle) -> Result<(), String> {
    let path = open_file_dialog(&app).await;
    let path = match path {
        Some(p) => p,
        None => return Err("err.backup_cancelled_import".to_string()),
    };

    let path_str = path.as_path().unwrap().to_string_lossy().to_string();

    // 持有 poll 锁，防止导入期间轮询线程并发修改数据库
    let _poll_guard = crate::poll::acquire_lock()
        .map_err(|_| "err.backup_import_busy".to_string())?;

    // 验证文件是有效的 SQLite 数据库
    validate_sqlite_file(&path_str)?;

    // 打开备份文件作为源连接（source）
    let src_conn = rusqlite::Connection::open(&path_str)
        .map_err(|e| format!("err.backup_open_failed|{}", e))?;

    // 从连接池获取目标连接（避免使用独立连接绕过连接池导致并发冲突）
    let state = app.state::<crate::types::AppState>();
    let mut dst_conn = state
        .db
        .get()
        .map_err(|e| format!("err.db_connect|{}", e))?;

    // 不预先 DELETE：SQLite Backup API 是页级整库覆盖拷贝，目标库的内容会被
    // 源库**完整替换**（含 schema），预先 DELETE 对结果无影响、反而会在恢复失败
    // 时造成用户数据被清空且无法回滚的数据丢失窗口。
    //
    // 使用 rusqlite backup API 将备份文件内容复制到运行中的数据库。
    // 通过池连接写入，SQLite 自身的 WAL 锁机制保证并发一致性。
    {
        let backup = Backup::new(&src_conn, &mut dst_conn)
            .map_err(|e| format!("err.backup_session_failed|{}", e))?;
        backup
            .run_to_completion(100, Duration::from_millis(250), None)
            .map_err(|e| format!("err.backup_restore_failed|{}", e))?;
    } // backup 在此处释放，dst_conn 不再被借用
    drop(dst_conn);

    // 恢复后跑迁移： imported 备份可能来自旧版本应用（缺 ai_summary/muted/
    // body_translated/rendered_message 等列），不补列会让引用这些列的查询立即报错，
    // 应用处于半瘫痪直到下次重启。此处主动补齐，避免该断裂。
    // 同时检查 master key 一致性： 备份里的加密设置（github_token/deepseek_api_key）
    // 用的是导出机器的 master key 加密，本机 master key 无法解密，自动清空避免静默失效。
    let cleared_keys: Vec<&'static str> = {
        let conn = state.db.get().map_err(|e| format!("err.db_connect|{}", e))?;
        // Backup 整库覆盖后，目标库 schema 被备份的 schema 完全替换。旧版本备份可能
        // 缺 logs 等基础表，也可能缺 ai_summary 等 ALTER 后新增的列。先 apply_schema
        // （CREATE TABLE IF NOT EXISTS 补齐缺失基础表），再 migrate（ALTER 补齐新增列），
        // 确保运行中的应用查询新列/新表不会报错。
        if let Err(e) = crate::db::init::apply_schema(&conn) {
            return Err(format!("err.backup_reinit_failed|{}", e));
        }
        if let Err(e) = crate::db::init::migrate(&conn) {
            return Err(format!("err.backup_migrate_failed|{}", e));
        }
        crate::crypto::verify_master_key_consistency(&conn)
    };
    if !cleared_keys.is_empty() {
        eprintln!(
            "WARNING: 导入的备份中以下加密设置无法用本机 master key 解密，已自动清空，请重新配置: {}",
            cleared_keys.join(", ")
        );
    }

    if let Ok(conn) = state.db.get() {
        crate::db::logs::write_log_key(&conn, "INFO", "backup.imported", &json!({"path": &path_str}).to_string());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_validate_sqlite_file_valid() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("test_valid_{}.db", std::process::id()));
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch("CREATE TABLE t (x);").unwrap();
        }
        let result = validate_sqlite_file(path.to_str().unwrap());
        assert!(result.is_ok(), "合法 SQLite 文件应通过验证: {:?}", result);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_validate_sqlite_file_invalid() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("test_invalid_{}.tmp", std::process::id()));
        {
            let mut f = std::fs::File::create(&path).unwrap();
            f.write_all(b"not a sqlite database file").unwrap();
        }
        let result = validate_sqlite_file(path.to_str().unwrap());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("err.backup_invalid_file"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_validate_sqlite_file_empty() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("test_empty_{}.tmp", std::process::id()));
        {
            std::fs::File::create(&path).unwrap();
        }
        let result = validate_sqlite_file(path.to_str().unwrap());
        assert!(result.is_err());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_validate_sqlite_file_nonexistent() {
        let result = validate_sqlite_file("/tmp/nonexistent_file_that_does_not_exist.db");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("err.backup_read_failed"));
    }

    /// 每个用例独立的临时目录（名带 pid + 标签），用完即删。
    ///
    /// 回归背景：旧用例把目标写成 `temp_dir()/test_export_<pid>.db`——
    /// ① 直接落在系统 temp 根目录且不保证清理，已累积大量残留；
    /// ② pid 会被复用，一旦撞上旧残留文件，`VACUUM INTO` 就报
    ///    `file is not a database`，使 CI 随机变红。
    fn unique_temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("relwatch-test-{}-{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 临时目录卫兵：Drop 时删除目录。
    ///
    /// 用例里**不能**只写一句 `remove_dir_all` 就完事：Windows 不允许删除仍被打开的文件
    /// 所在目录，而用例还持有 `Connection::open(备份文件)`——在 `restored` 仍存活时清理
    /// 会**静默失败**（`let _ =` 吞掉），留下残留目录（实测 `export_valid` /
    /// `export_overwrite` 各留一堆）。用卫兵即可：局部变量按声明逆序析构，卫兵声明在最前
    /// ⇒ 最后析构，届时连接已全部关闭；断言失败/panic 展开时同样会走到 Drop。
    /// 顺带把「结尾手动清理」这个容易漏写的约定收口成一种写法。
    struct TempDirGuard(PathBuf);

    impl TempDirGuard {
        fn new(tag: &str) -> Self {
            Self(unique_temp_dir(tag))
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// 建一个内存库并写入一行 t.val = 'hello'。
    fn seed_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE t (id INTEGER PRIMARY KEY, val TEXT);
             INSERT INTO t VALUES (1, 'hello');",
        )
        .unwrap();
        conn
    }

    #[test]
    fn test_export_vacuum_into_creates_valid_sqlite_file() {
        let dir = TempDirGuard::new("export_valid");
        let backup_path = dir.path().join("backup.db");
        let conn = seed_db();

        // 调生产同一份实现（不再在测试里另拄一遍 VACUUM INTO——旧写法是
        // 「测试副本」：它既测不到生产的转义/覆盖处理，自身又引入了 temp 残留 flake）
        vacuum_into(&conn, &backup_path).unwrap();

        assert!(backup_path.exists(), "应创建备份文件");
        assert!(validate_sqlite_file(backup_path.to_str().unwrap()).is_ok(),
                "导出的文件应为有效的 SQLite 数据库");

        // 数据往返
        let restored = Connection::open(&backup_path).unwrap();
        let val: String = restored
            .query_row("SELECT val FROM t WHERE id = 1", [], |row| row.get(0))
            .unwrap();
        assert_eq!(val, "hello");

        // 临时文件不得留下
        assert!(!sibling_temp_path(&backup_path).exists(), "不应留下 .tmp-<pid> 临时文件");
    }

    /// 回归：目标已存在且非空时，导出必须成功并覆盖。
    ///
    /// 修复前此处报 `file is not a database`（`VACUUM INTO` 不接受已有内容的文件），
    /// 用户看到的是一句与真实原因毫无关系的报错。
    #[test]
    fn test_export_overwrites_existing_nonempty_target() {
        let dir = TempDirGuard::new("export_overwrite");
        let backup_path = dir.path().join("backup.db");
        // 模拟「用户选中旧备份并确认覆盖」：目标已存在且有内容
        std::fs::write(&backup_path, b"stale backup content".repeat(64)).unwrap();

        let conn = seed_db();
        vacuum_into(&conn, &backup_path).unwrap();

        assert!(validate_sqlite_file(backup_path.to_str().unwrap()).is_ok(),
                "覆盖后应为合法 SQLite，而非旧内容残留");
        let restored = Connection::open(&backup_path).unwrap();
        let val: String = restored
            .query_row("SELECT val FROM t WHERE id = 1", [], |row| row.get(0))
            .unwrap();
        assert_eq!(val, "hello");
        assert!(!sibling_temp_path(&backup_path).exists());
    }

    /// 回归：路径含单引号时不再依赖字符串拼接转义（改用绑定参数）。
    #[test]
    fn test_export_handles_single_quote_in_path() {
        let root = TempDirGuard::new("export_quote");
        let dir = root.path().join("it's here");
        std::fs::create_dir_all(&dir).unwrap();
        let backup_path = dir.join("back'up.db");

        let conn = seed_db();
        vacuum_into(&conn, &backup_path).unwrap();

        assert!(validate_sqlite_file(backup_path.to_str().unwrap()).is_ok());
    }

    /// 目标目录不存在时失败得干净：返回可翻译错误键，不 panic、不留临时文件。
    #[test]
    fn test_export_failure_is_clean_and_leaves_no_temp() {
        let dir = TempDirGuard::new("export_fail");
        let missing = dir.path().join("no_such_subdir").join("backup.db");

        let conn = seed_db();
        let err = vacuum_into(&conn, &missing).unwrap_err();
        assert!(err.starts_with("err.backup_export_failed|"), "{err}");
        assert!(!missing.exists());
        assert!(!sibling_temp_path(&missing).exists(), "失败后不应留下临时文件");
    }

    #[test]
    fn test_sibling_temp_path_is_same_dir_and_suffixed() {
        let target = std::env::temp_dir().join("a").join("backup.db");
        let tmp = sibling_temp_path(&target);
        assert_eq!(tmp.parent(), target.parent(), "必须同目录（同卷）才能 rename");
        assert!(
            tmp.file_name().unwrap().to_string_lossy().starts_with("backup.db.tmp-"),
            "{:?}",
            tmp.file_name()
        );
    }

    #[test]
    fn test_import_backup_restores_data() {
        use std::time::Duration;

        let dir = std::env::temp_dir();
        let backup_path = dir.join(format!("test_restore_{}.db", std::process::id()));

        // 创建备份文件
        {
            let src = rusqlite::Connection::open(&backup_path).unwrap();
            src.execute_batch(
                "CREATE TABLE t (id INTEGER PRIMARY KEY, val TEXT);
                 INSERT INTO t VALUES (1, 'restored-data');"
            ).unwrap();
        }

        // 创建内存目标数据库（模拟运行中的 DB）
        let mut dst = rusqlite::Connection::open_in_memory().unwrap();
        dst.execute_batch(
            "CREATE TABLE t (id INTEGER PRIMARY KEY, val TEXT);"
        ).unwrap();

        // rusqlite Backup API —— import_backup 的核心操作
        {
            let src_conn = rusqlite::Connection::open(&backup_path).unwrap();
            let backup = rusqlite::backup::Backup::new(&src_conn, &mut dst).unwrap();
            backup.run_to_completion(100, Duration::from_millis(250), None).unwrap();
        }

        // 验证数据已恢复
        let val: String = dst.query_row(
            "SELECT val FROM t WHERE id = 1", [], |row| row.get(0)
        ).unwrap();
        assert_eq!(val, "restored-data");

        let _ = std::fs::remove_file(&backup_path);
    }

    #[test]
    fn test_import_clears_existing_data_before_restore() {
        use std::time::Duration;

        let dir = std::env::temp_dir();
        let backup_path = dir.join(format!("test_clear_{}.db", std::process::id()));

        // 创建备份（一条记录）
        {
            let src = rusqlite::Connection::open(&backup_path).unwrap();
            src.execute_batch(
                "CREATE TABLE t (id INTEGER PRIMARY KEY, val TEXT);
                 INSERT INTO t VALUES (1, 'fresh');"
            ).unwrap();
        }

        // 创建目标数据库，包含脏数据
        let mut dst = rusqlite::Connection::open_in_memory().unwrap();
        dst.execute_batch(
            "CREATE TABLE t (id INTEGER PRIMARY KEY, val TEXT);
             INSERT INTO t VALUES (1, 'stale');
             INSERT INTO t VALUES (2, 'extra');"
        ).unwrap();

        // 注意：生产代码 import_backup 不再预先 DELETE（Backup API 页级整库覆盖，
        // DELETE 是死代码且制造数据丢失窗口）。这里仅验证 Backup 覆盖后脏数据被替换。
        // 从备份恢复（整库覆盖）
        {
            let src_conn = rusqlite::Connection::open(&backup_path).unwrap();
            let backup = rusqlite::backup::Backup::new(&src_conn, &mut dst).unwrap();
            backup.run_to_completion(100, Duration::from_millis(250), None).unwrap();
        }

        // 验证只有备份中的记录存在
        let count: i64 = dst.query_row(
            "SELECT COUNT(*) FROM t", [], |row| row.get(0)
        ).unwrap();
        assert_eq!(count, 1, "只应存在恢复的一条记录");
        let val: String = dst.query_row(
            "SELECT val FROM t", [], |row| row.get(0)
        ).unwrap();
        assert_eq!(val, "fresh", "脏数据应被备份数据覆盖");

        let _ = std::fs::remove_file(&backup_path);
    }

    /// 问题2 回归测试：导入旧版本 schema 的备份（缺 ai_summary 等 ALTER 后新增的列）
    /// 后，Backup API 整库覆盖会把目标库 schema 退回旧版；不补列会让引用新列的
    /// 查询立即报错。此处验证 import_backup 修复后的链路：覆盖 → apply_schema → migrate → 查询新列成功。
    #[test]
    fn test_import_old_schema_backup_then_migrate_restores_columns() {
        use crate::db::init::{apply_schema, migrate};
        use std::time::Duration;

        let dir = std::env::temp_dir();
        let backup_path = dir.join(format!("test_old_schema_{}.db", std::process::id()));

        // 构造一个旧版本 schema 的备份：只有基础 releases 表，无 ai_summary/ai_importance 列
        {
            let src = rusqlite::Connection::open(&backup_path).unwrap();
            src.execute_batch(
                "CREATE TABLE releases (
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
                    UNIQUE(source_id, tag_name)
                );
                 INSERT INTO releases VALUES (1, 1, 'v1', 'R', 'u', '2024-01-01', 0, 'b', '2024-01-01', 0);
                 CREATE TABLE sources (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    source_type TEXT NOT NULL, owner TEXT NOT NULL, repo TEXT NOT NULL,
                    poll_interval_minutes INTEGER NOT NULL DEFAULT 30,
                    enabled INTEGER NOT NULL DEFAULT 1,
                    last_checked_at TEXT, last_check_status TEXT NOT NULL DEFAULT 'unknown',
                    last_check_message TEXT, consecutive_failures INTEGER NOT NULL DEFAULT 0,
                    last_new_count INTEGER NOT NULL DEFAULT 0, muted INTEGER NOT NULL DEFAULT 0,
                    created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
                    UNIQUE(source_type, owner, repo)
                );
                 INSERT INTO sources VALUES (1,'github','o','r',30,1,NULL,'unknown',NULL,0,0,0,'2024','2024');"
            ).unwrap();
        }

        // 目标库：先用完整 init 建 schema，再被旧备份覆盖（模拟运行中应用导入旧备份）
        let mut dst = crate::db::init::init_memory_db().unwrap();
        {
            let src_conn = rusqlite::Connection::open(&backup_path).unwrap();
            let backup = rusqlite::backup::Backup::new(&src_conn, &mut dst).unwrap();
            backup.run_to_completion(100, Duration::from_millis(250), None).unwrap();
        }

        // 覆盖后查询新列应失败（证明 schema 已退回旧版）
        let pre = dst.prepare("SELECT r.ai_summary FROM releases r");
        assert!(pre.is_err(), "覆盖后旧 schema 应缺 ai_summary 列");
        drop(pre);

        // 跑恢复后链路：先补基础表，再 migrate 补 ALTER 列（与 import_backup 一致）
        apply_schema(&dst).expect("apply_schema 应补齐缺失的基础表");
        migrate(&dst).expect("migrate 应成功补齐缺失列");

        // 现在引用新列的查询应成功
        let ok = dst.prepare("SELECT r.ai_summary, r.ai_importance, r.body_translated FROM releases r");
        assert!(ok.is_ok(), "migrate 后应补齐 ai_summary/ai_importance/body_translated 列");

        let _ = std::fs::remove_file(&backup_path);
    }
}
