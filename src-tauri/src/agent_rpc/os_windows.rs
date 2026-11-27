use std::os::windows::process::CommandExt;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Windows JobObject 生命周期域：句柄关闭时内核终止作业内全部后代进程，消除工具子进程孤儿窗口。
pub(crate) struct ProcessScope {
    job: windows::Win32::Foundation::HANDLE,
}

impl ProcessScope {
    pub(crate) fn attach(pid: u32) -> Option<Self> {
        use windows::Win32::Foundation::{CloseHandle, HANDLE};
        use windows::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };
        use windows::Win32::System::Threading::{
            OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
        };

        unsafe {
            let job = match CreateJobObjectW(None, None) {
                Ok(h) => h,
                Err(e) => {
                    log::warn!("agent JobObject 创建失败，进程生命周期域降级: {}", e);
                    return None;
                }
            };
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION =
                std::mem::zeroed::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let ok = SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
            .is_ok();
            if !ok {
                log::warn!("agent JobObject 限制设置失败，进程生命周期域降级");
                let _ = CloseHandle(job);
                return None;
            }
            let proc = match OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, false, pid) {
                Ok(h) => h,
                Err(e) => {
                    log::warn!("agent JobObject 打开进程失败，进程生命周期域降级: {}", e);
                    let _ = CloseHandle(job);
                    return None;
                }
            };
            let assigned = AssignProcessToJobObject(job, proc).is_ok();
            let _ = CloseHandle(proc);
            if !assigned {
                log::warn!("agent JobObject 纳管进程失败，进程生命周期域降级");
                let _ = CloseHandle(job);
                return None;
            }
            let _: HANDLE = job;
            Some(ProcessScope { job })
        }
    }
}

impl Drop for ProcessScope {
    fn drop(&mut self) {
        use windows::Win32::Foundation::CloseHandle;
        unsafe {
            let _ = CloseHandle(self.job);
        }
    }
}

// JobObject 句柄仅随 RpcProcess 持有与释放，不跨线程共享内部可变状态。
unsafe impl Send for ProcessScope {}
unsafe impl Sync for ProcessScope {}

pub(crate) fn configure_spawn_command(binary: &str) -> tokio::process::Command {
    // npm 全局安装的 pi.cmd 是批处理，CreateProcess 无法直接执行，需经 cmd /C 解释。
    let mut cmd = tokio::process::Command::new("cmd");
    cmd.arg("/C").arg(binary);
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd.kill_on_drop(true);
    cmd
}

pub(crate) fn configure_probe_command(cmd: &mut std::process::Command) {
    cmd.creation_flags(CREATE_NO_WINDOW);
}

pub fn kill_process_tree(pid: u32) {
    let mut cmd = std::process::Command::new("taskkill");
    cmd.args(["/F", "/T", "/PID", &pid.to_string()]);
    cmd.creation_flags(CREATE_NO_WINDOW);
    let _ = cmd.output();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    #[test]
    fn process_scope_terminates_child_on_drop() {
        let mut child = Command::new("cmd")
            .args(["/C", "pause"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn cmd /C pause");
        let pid = child.id();

        let scope = ProcessScope::attach(pid).expect("JobObject 纳管应成功");
        drop(scope);

        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = child.wait();
            let _ = tx.send(());
        });
        assert!(
            rx.recv_timeout(std::time::Duration::from_secs(5)).is_ok(),
            "JobObject 句柄关闭后，作业内的子进程应被内核终止"
        );
    }
}
