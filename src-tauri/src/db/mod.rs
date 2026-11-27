pub mod init;
pub mod pool;

pub use crate::error::{DbError, DbResult};
pub use pool::SqliteConnectionManager;
pub type DbPool = r2d2::Pool<SqliteConnectionManager>;
pub mod sources;
pub mod settings;
pub mod releases;
pub mod logs;
pub mod usage;
pub mod ai_usage;
pub mod save;
pub mod agent;

pub async fn run_blocking<T, F>(pool: &DbPool, f: F) -> crate::error::AppResult<T>
where
    T: Send + 'static,
    F: FnOnce(&rusqlite::Connection) -> crate::error::AppResult<T> + Send + 'static,
{
    let pool = pool.clone();
    tokio::task::spawn_blocking(move || {
        let conn = pool.get().map_err(crate::error::DbError::Connect)?;
        f(&conn)
    })
    .await
    .map_err(|e| crate::error::AppError::Db(crate::error::DbError::TaskJoin(e.to_string())))?
}

pub async fn run_blocking_mut<T, F>(pool: &DbPool, f: F) -> crate::error::AppResult<T>
where
    T: Send + 'static,
    F: FnOnce(&mut rusqlite::Connection) -> crate::error::AppResult<T> + Send + 'static,
{
    let pool = pool.clone();
    tokio::task::spawn_blocking(move || {
        let mut conn = pool.get().map_err(crate::error::DbError::Connect)?;
        f(&mut conn)
    })
    .await
    .map_err(|e| crate::error::AppError::Db(crate::error::DbError::TaskJoin(e.to_string())))?
}

pub(crate) const BATCH_CHUNK_SIZE: usize = 500;

pub(crate) fn dedup_ids(ids: &[i64]) -> Vec<i64> {
    let mut seen = std::collections::HashSet::with_capacity(ids.len());
    ids.iter().copied().filter(|id| seen.insert(*id)).collect()
}

pub(crate) fn in_placeholders(count: usize, start_index: usize) -> String {
    (0..count)
        .map(|i| format!("?{}", i + start_index))
        .collect::<Vec<_>>()
        .join(", ")
}
