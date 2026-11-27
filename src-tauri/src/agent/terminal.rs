use crate::db::agent::{self, AgentConfig};

pub const MAX_ATTACHED_FILES: usize = 10;
pub const MAX_ATTACHED_FILE_BYTES: u64 = 20 * 1024 * 1024;

pub fn agent_process_level_changed(a: &AgentConfig, b: &AgentConfig) -> bool {
    a.agent_type != b.agent_type
        || a.binary != b.binary
        || a.model != b.model
        || a.working_dir != b.working_dir
        || a.skills != b.skills
}

pub fn validate_attached_files(files: Option<&Vec<String>>) -> Result<Vec<String>, String> {
    let Some(list) = files else {
        return Ok(Vec::new());
    };
    let mut out: Vec<String> = Vec::new();
    for raw in list {
        let p = raw.trim();
        if p.is_empty() || out.iter().any(|x| x == p) {
            continue;
        }
        let meta = std::fs::metadata(p).map_err(|_| "err.agent.file_missing".to_string())?;
        if !meta.is_file() {
            return Err("err.agent.file_not_file".to_string());
        }
        if meta.len() > MAX_ATTACHED_FILE_BYTES {
            return Err("err.agent.file_too_large".to_string());
        }
        out.push(p.to_string());
        if out.len() > MAX_ATTACHED_FILES {
            return Err(format!("err.agent.file_too_many|{}", MAX_ATTACHED_FILES));
        }
    }
    Ok(out)
}

pub fn resolve_session_binary(
    conn: &rusqlite::Connection,
) -> Result<(String, Option<String>), String> {
    let config = agent::load_agent_config(conn)?;
    crate::agent_rpc::ensure_supported_type(&config)?;
    let binary = crate::agent_rpc::resolve_agent_binary(&config)?;
    Ok((binary, config.working_dir))
}

pub fn format_session_command(binary: &str, path: &str, working_dir: Option<&str>) -> String {
    let wd = working_dir.map(str::trim).filter(|s| !s.is_empty());
    let base = format!("\"{}\" --session \"{}\"", binary, path);
    match wd {
        Some(w) if cfg!(windows) => format!("cd /d \"{}\" && {}", w, base),
        Some(w) => format!("cd \"{}\" && {}", w, base),
        None => base,
    }
}

pub fn spawn_terminal(
    binary: &str,
    session_path: &str,
    working_dir: Option<&str>,
) -> Result<(), String> {
    #[cfg(windows)]
    {
        let mut cmd = std::process::Command::new("cmd");
        cmd.arg("/C").arg("start").arg("");
        if let Some(wd) = working_dir {
            cmd.arg("/D").arg(wd);
        }
        cmd.arg(binary).arg("--session").arg(session_path);
        cmd.spawn().map_err(|e| format!("err.agent.spawn|{}", e))?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let terminals = [
            ("x-terminal-emulator", vec!["-e"]),
            ("gnome-terminal", vec!["--"]),
            ("konsole", vec!["-e"]),
            ("xfce4-terminal", vec!["-e"]),
        ];
        for (name, args) in terminals {
            if std::process::Command::new("which")
                .arg(name)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
            {
                let mut cmd = std::process::Command::new(name);
                cmd.args(&args);
                cmd.arg(binary).arg("--session").arg(session_path);
                if let Some(wd) = working_dir {
                    cmd.current_dir(wd);
                }
                cmd.spawn().map_err(|e| format!("err.agent.spawn|{}", e))?;
                return Ok(());
            }
        }
        Err("err.agent.no_terminal".to_string())
    }
}
