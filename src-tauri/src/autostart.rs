//! 跨平台自动启动管理模块
//!
//! - Windows: 通过注册表 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` 实现
//! - Linux: 通过 `~/.config/autostart/relwatch.desktop` 文件实现
//!
//! 无论哪个平台，启动命令都附加 `--autostart` 参数，
//! 便于应用在启动时检测并自动最小化到托盘。

use std::path::PathBuf;

/// 获取当前可执行文件路径
fn get_exe_path() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("relwatch"))
}

/// 获取带有 --autostart 参数的命令行字符串
fn get_autostart_command() -> String {
    format_autostart_command(&get_exe_path().to_string_lossy())
}

/// 根据可执行文件路径格式化 autostart 命令行。
/// 如果路径包含空格，自动用双引号包裹。
fn format_autostart_command(exe_path: &str) -> String {
    if exe_path.contains(' ') {
        format!("\"{}\" --autostart", exe_path)
    } else {
        format!("{} --autostart", exe_path)
    }
}

/// 启用开机自启动
pub fn enable() -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        enable_windows()?;
    }

    #[cfg(target_os = "linux")]
    {
        enable_linux()?;
    }

    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        let _ = get_autostart_command(); // 抑制未使用警告
        log::warn!("当前平台不支持开机自启动");
    }

    Ok(())
}

/// 禁用开机自启动
pub fn disable() -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        disable_windows()?;
    }

    #[cfg(target_os = "linux")]
    {
        disable_linux()?;
    }

    Ok(())
}

// ── Windows ────────────────────────────────────────────

#[cfg(target_os = "windows")]
fn enable_windows() -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    let cmd = get_autostart_command();
    let key = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

    let status = std::process::Command::new("reg")
        .args([
            "add",
            key,
            "/v",
            "RelWatch",
            "/t",
            "REG_SZ",
            "/d",
            &cmd,
            "/f",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(|e| format!("无法执行注册表操作: {}", e))?;

    if !status.success() {
        return Err("设置开机自启动失败（注册表写入失败）".to_string());
    }

    Ok(())
}

#[cfg(target_os = "windows")]
fn disable_windows() -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    let key = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

    let status = std::process::Command::new("reg")
        .args(["delete", key, "/v", "RelWatch", "/f"])
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(|e| format!("无法执行注册表操作: {}", e))?;

    // 即使键不存在也返回成功（已经处于未启动状态）
    if !status.success() {
        log::warn!("取消开机自启动时注册表操作返回非零状态（可能已不存在）");
    }

    Ok(())
}

// ── Linux ──────────────────────────────────────────────

#[cfg(target_os = "linux")]
fn autostart_dir() -> Result<PathBuf, String> {
    // 优先使用 XDG_CONFIG_HOME，否则 fallback 到 ~/.config
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        Ok(PathBuf::from(xdg).join("autostart"))
    } else if let Ok(home) = std::env::var("HOME") {
        Ok(PathBuf::from(home).join(".config/autostart"))
    } else {
        Err("无法确定 autostart 目录位置（缺少 HOME 环境变量）".to_string())
    }
}

#[cfg(target_os = "linux")]
fn enable_linux() -> Result<(), String> {
    let dir = autostart_dir()?;
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("无法创建 autostart 目录 {}: {}", dir.display(), e))?;

    let cmd = get_autostart_command();
    let desktop_content = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=RelWatch\n\
         Exec={}\n\
         X-GNOME-Autostart-enabled=true\n\
         Terminal=false\n",
        cmd
    );

    let desktop_path = dir.join("relwatch.desktop");
    std::fs::write(&desktop_path, &desktop_content)
        .map_err(|e| format!("无法写入自启动文件 {}: {}", desktop_path.display(), e))?;

    Ok(())
}

#[cfg(target_os = "linux")]
fn disable_linux() -> Result<(), String> {
    let dir = autostart_dir()?;
    let desktop_path = dir.join("relwatch.desktop");

    if desktop_path.exists() {
        std::fs::remove_file(&desktop_path)
            .map_err(|e| format!("无法移除自启动文件 {}: {}", desktop_path.display(), e))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_autostart_command_no_spaces() {
        let cmd = format_autostart_command("relwatch");
        assert_eq!(cmd, "relwatch --autostart");
    }

    #[test]
    fn test_format_autostart_command_with_spaces() {
        let cmd = format_autostart_command("C:\\Program Files\\RelWatch\\relwatch.exe");
        assert_eq!(cmd, "\"C:\\Program Files\\RelWatch\\relwatch.exe\" --autostart");
    }

    #[test]
    fn test_format_autostart_command_empty_path() {
        let cmd = format_autostart_command("");
        assert_eq!(cmd, " --autostart");
    }

    #[test]
    fn test_format_autostart_command_trailing_space() {
        let cmd = format_autostart_command("/usr/local/bin/relwatch ");
        assert_eq!(cmd, "\"/usr/local/bin/relwatch \" --autostart");
    }
}
