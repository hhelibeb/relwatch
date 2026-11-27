mod discovery;
#[cfg(not(windows))]
mod os_unix;
#[cfg(windows)]
mod os_windows;
mod pool;
mod process;

#[cfg(not(windows))]
use os_unix as os;
#[cfg(windows)]
use os_windows as os;

use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Serialize, Deserialize, Clone, Type, PartialEq)]
pub struct RpcAvailableModel {
    pub provider: String,
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
}

pub use discovery::{
    ensure_supported_type, filter_scoped_models, model_matches_scoped_pattern, pi_agent_dir,
    read_scoped_model_patterns, resolve_agent_binary, scoped_settings_path,
};
pub use os::kill_process_tree;
pub use pool::{AgentConfigProvider, DbConfigProvider, RpcManager};
pub use process::ProcessLease;
