fn main() {
  // 禁用 tauri-build 的默认 app manifest（common-controls v6），
  // 改由下方统一通过链接参数嵌入，确保 bin 与测试目标行为一致。
  tauri_build::try_build(
    tauri_build::Attributes::new()
      .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest()),
  )
  .expect("failed to run tauri-build");

  // workaround：tauri-build 默认仅给 bin 目标嵌入 Windows manifest（.res 方式），
  // 测试 exe 一旦链接 tauri 的 WinRT/GUI 代码路径（如 tauri-specta 收集全部命令），
  // 缺少 common-controls v6 激活上下文会在加载时 STATUS_ENTRYPOINT_NOT_FOUND (0xc0000139)。
  // 参考 tauri 官方示例：https://github.com/tauri-apps/tauri/pull/4383#issuecomment-1212221864
  let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
  let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
  if target_os == "windows" && target_env == "msvc" {
    embed_manifest();
  }
}

fn embed_manifest() {
  let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("windows-app-manifest.xml");
  println!("cargo:rerun-if-changed={}", manifest.display());
  println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
  println!(
    "cargo:rustc-link-arg=/MANIFESTINPUT:{}",
    manifest.to_str().unwrap()
  );
}
