pub(crate) struct ProcessScope;

impl ProcessScope {
    pub(crate) fn attach(_pid: u32) -> Option<Self> {
        None
    }
}

pub(crate) fn configure_spawn_command(binary: &str) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(binary);
    // 独立进程组首领，使 kill_process_tree 的负 pid 能整树击杀且不误伤主进程组。
    cmd.process_group(0);
    cmd.kill_on_drop(true);
    cmd
}

pub(crate) fn configure_probe_command(_cmd: &mut std::process::Command) {}

pub fn kill_process_tree(pid: u32) {
    let is_pgid_leader = std::process::Command::new("ps")
        .args(["-o", "pgid=", "-p", &pid.to_string()])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse::<u32>().ok())
        == Some(pid);
    let target = if is_pgid_leader {
        format!("-{}", pid)
    } else {
        pid.to_string()
    };
    let _ = std::process::Command::new("kill")
        .args(["-9", &target])
        .output();
}
