//! r2d2 的 SQLite 连接管理器（内联实现，替代 `r2d2_sqlite`）。
//!
//! 不用 `r2d2_sqlite` 是因为它把 rusqlite 锁在 ^0.40，而 RUSTSEC-2026-0459 要求
//! libsqlite3-sys >= 0.39（只有 rusqlite >= 0.41 才满足）。行为与 r2d2_sqlite 对齐：
//! `connect` 建连后执行 init 回调，`is_valid` 用 `SELECT 1` 探活，`has_broken` 恒 false。

use rusqlite::{Connection, OpenFlags};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

type InitFn = dyn Fn(&mut Connection) -> rusqlite::Result<()> + Send + Sync + 'static;

enum Source {
    File(PathBuf),
    Memory(String),
}

pub struct SqliteConnectionManager {
    source: Source,
    flags: OpenFlags,
    init: Option<Box<InitFn>>,
    /// 共享缓存内存库在最后一条连接关闭时会被销毁，若池内连接全部归还后被回收，
    /// 下次 get() 会拿到一个空库。持有一条锚连接让库的生命周期跟随 manager。
    anchor: Mutex<Option<Connection>>,
}

impl SqliteConnectionManager {
    pub fn file(path: impl Into<PathBuf>) -> Self {
        Self {
            source: Source::File(path.into()),
            flags: OpenFlags::default(),
            init: None,
            anchor: Mutex::new(None),
        }
    }

    /// 共享缓存的内存库：池内所有连接指向同一个库（仅供测试使用）。
    pub fn memory() -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        Self {
            source: Source::Memory(format!(
                "relwatch_mem_{}",
                NEXT_ID.fetch_add(1, Ordering::Relaxed)
            )),
            flags: OpenFlags::default(),
            init: None,
            anchor: Mutex::new(None),
        }
    }

    pub fn with_init<F>(self, init: F) -> Self
    where
        F: Fn(&mut Connection) -> rusqlite::Result<()> + Send + Sync + 'static,
    {
        Self {
            init: Some(Box::new(init)),
            ..self
        }
    }

    fn open(&self) -> rusqlite::Result<Connection> {
        match &self.source {
            Source::File(path) => Connection::open_with_flags(path, self.flags),
            Source::Memory(id) => Connection::open_with_flags(
                format!("file:{id}?mode=memory&cache=shared"),
                self.flags,
            ),
        }
    }
}

impl r2d2::ManageConnection for SqliteConnectionManager {
    type Connection = Connection;
    type Error = rusqlite::Error;

    fn connect(&self) -> rusqlite::Result<Connection> {
        if let Source::Memory(_) = self.source {
            let mut anchor = self.anchor.lock().unwrap();
            if anchor.is_none() {
                *anchor = Some(self.open()?);
            }
        }
        let mut conn = self.open()?;
        if let Some(init) = &self.init {
            init(&mut conn)?;
        }
        Ok(conn)
    }

    fn is_valid(&self, conn: &mut Connection) -> rusqlite::Result<()> {
        conn.query_row("SELECT 1", [], |row| row.get::<_, i64>(0))
            .map(|_| ())
    }

    fn has_broken(&self, _conn: &mut Connection) -> bool {
        false
    }
}
