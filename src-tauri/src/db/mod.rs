pub mod init;
pub mod pool;

pub use pool::SqliteConnectionManager;
pub mod sources;
pub mod settings;
pub mod releases;
pub mod logs;
pub mod usage;
pub mod ai_usage;
pub mod save;
pub mod agent;

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
