use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

/// v2 密文前缀：标记用 master key（OS keyring 托管）加密的数据
const V2_PREFIX: &str = "v2:";

/// 全局 master key，启动时从 OS keyring 加载并缓存。
/// 首次运行时随机生成并写入 keyring。
static MASTER_KEY: OnceLock<[u8; 32]> = OnceLock::new();

/// 已尝试 v1→v2 迁移的密文缓存，防止回写失败后无限重试。
/// 回写失败的密文被记录在此，后续调用跳过迁移，重启后重置。
static MIGRATION_ATTEMPTED: Mutex<Option<HashSet<String>>> = Mutex::new(None);

// ── 跨平台 keyring 交互 ────────────────────────────────

/// 从 `keyring::Entry` 加载 master key，首次运行则随机生成并存储。
///
/// **注意**：此函数不设置 `MASTER_KEY` 全局缓存，仅处理 keyring 交互。
/// 提取为独立函数以便测试（可用 keyring mock 注入）。
/// Windows 上生产代码不使用 keyring 但测试仍需要。
#[cfg_attr(windows, allow(dead_code))]
fn load_or_generate_master_key(entry: &keyring::Entry) -> Result<[u8; 32], String> {
    match entry.get_password() {
        Ok(pw) => {
            let bytes = BASE64
                .decode(pw.as_bytes())
                .map_err(|_| {
                    "Stored master key in OS keyring is corrupted (invalid base64). \
                     Please delete the entry 'relwatch/master-key' from your OS keyring \
                     and restart the application."
                        .to_string()
                })?;
            if bytes.len() != 32 {
                return Err(
                    "Stored master key in OS keyring has wrong length. \
                     Please delete the entry 'relwatch/master-key' from your OS keyring \
                     and restart the application."
                        .to_string(),
                );
            }
            let mut key = [0u8; 32];
            key.copy_from_slice(&bytes);
            Ok(key)
        }
        Err(keyring::Error::NoEntry) => {
            // 首次运行：生成随机 32B 密钥并写入 OS keyring
            let key: [u8; 32] = rand::Rng::gen(&mut rand::thread_rng());
            let encoded = BASE64.encode(key);
            entry
                .set_password(&encoded)
                .map_err(|e| format!("Failed to write master key to OS keyring: {}", e))?;
            Ok(key)
        }
        Err(e) => {
            Err(format!(
                "Failed to read master key from OS keyring: {}. \
                 The application cannot start without OS keyring access.",
                e
            ))
        }
    }
}

// ── 平台相关的密钥加载 ──────────────────────────────

/// 非 Windows 平台：使用 keyring crate
#[cfg(not(windows))]
fn platform_load_or_generate_master_key() -> Result<[u8; 32], String> {
    let entry = keyring::Entry::new("relwatch", "master-key")
        .map_err(|e| format!("Failed to create keyring entry: {}", e))?;
    load_or_generate_master_key(&entry)
}

/// Windows 平台：直接调用 Win32 API，使用 `CRED_PERSIST_LOCAL_MACHINE`
/// 解决 keyring crate 默认使用 `CRED_PERSIST_ENTERPRISE`
/// 在非域环境中等同于仅当前会话的问题。
#[cfg(windows)]
fn platform_load_or_generate_master_key() -> Result<[u8; 32], String> {
    platform::load_or_generate()
}

#[cfg(windows)]
#[allow(clippy::upper_case_acronyms)]
mod platform {
    use base64::Engine;
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr;

    const CRED_TYPE_GENERIC: u32 = 1;
    /// `CRED_PERSIST_LOCAL_MACHINE` = 2，凭证持久化到本地机器，重启后仍有效。
    /// 相比 keyring crate 默认使用的 `CRED_PERSIST_ENTERPRISE` = 3（非域机器上等同于仅当前会话），
    /// 这是修复凭证重启丢失问题的关键。
    const CRED_PERSIST_LOCAL_MACHINE: u32 = 2;
    const ERROR_NOT_FOUND: u32 = 1168;

    #[repr(C)]
    struct FILETIME {
        dw_low_date_time: u32,
        dw_high_date_time: u32,
    }

    #[repr(C)]
    struct CREDENTIALW {
        flags: u32,
        typ: u32,
        target_name: *mut u16,
        comment: *mut u16,
        last_written: FILETIME,
        credential_blob_size: u32,
        credential_blob: *mut u8,
        persist: u32,
        attribute_count: u32,
        attributes: *mut core::ffi::c_void,
        target_alias: *mut u16,
        user_name: *mut u16,
    }

    #[link(name = "advapi32")]
    extern "system" {
        fn CredReadW(
            target_name: *const u16,
            credential_type: u32,
            flags: u32,
            credential: *mut *mut CREDENTIALW,
        ) -> i32;

        fn CredWriteW(credential: *const CREDENTIALW, flags: u32) -> i32;

        fn CredFree(buffer: *mut core::ffi::c_void) -> i32;

        fn GetLastError() -> u32;
    }

    /// 生成目标名称：使用统一前缀，与 keyring crate 的服务名一致
    const TARGET_NAME: &str = "RelWatch_MasterKey";

    pub fn load_or_generate() -> Result<[u8; 32], String> {
        match read_credential(TARGET_NAME) {
            Ok(Some(encoded)) => {
                let bytes = super::BASE64
                    .decode(encoded.as_bytes())
                    .map_err(|_| {
                        "Stored master key in Windows Credential Manager is corrupted \
                         (invalid base64). Please delete the 'RelWatch_MasterKey' entry \
                         from your Windows Credential Manager and restart."
                            .to_string()
                    })?;
                if bytes.len() != 32 {
                    return Err(
                        "Stored master key in Windows Credential Manager has wrong length. \
                         Please delete the 'RelWatch_MasterKey' entry from your \
                         Windows Credential Manager and restart."
                            .to_string(),
                    );
                }
                let mut key = [0u8; 32];
                key.copy_from_slice(&bytes);
                Ok(key)
            }
            Ok(None) => {
                // 首次运行：生成随机 32B 密钥，使用 CRED_PERSIST_LOCAL_MACHINE 写入
                let key: [u8; 32] = rand::Rng::gen(&mut rand::thread_rng());
                let encoded = super::BASE64.encode(key);
                write_credential(TARGET_NAME, encoded.as_bytes())?;
                Ok(key)
            }
            Err(e) => Err(e),
        }
    }

    fn to_wide(s: &str) -> Vec<u16> {
        OsStr::new(s).encode_wide().chain(Some(0)).collect()
    }

    fn read_credential(target: &str) -> Result<Option<String>, String> {
        let target_wide = to_wide(target);
        unsafe {
            let mut p_cred: *mut CREDENTIALW = ptr::null_mut();
            let ret = CredReadW(
                target_wide.as_ptr(),
                CRED_TYPE_GENERIC,
                0,
                &mut p_cred,
            );
            if ret != 0 {
                // 成功读取
                let blob = std::slice::from_raw_parts(
                    (*p_cred).credential_blob,
                    (*p_cred).credential_blob_size as usize,
                );
                let value =
                    String::from_utf8(blob.to_vec()).map_err(|_| {
                        "Windows credential data is not valid UTF-8".to_string()
                    })?;
                CredFree(p_cred as *mut core::ffi::c_void);
                Ok(Some(value))
            } else {
                let err = GetLastError();
                if err == ERROR_NOT_FOUND {
                    Ok(None)
                } else {
                    Err(format!(
                        "Failed to read Windows credential (error code: {})",
                        err
                    ))
                }
            }
        }
    }

    fn write_credential(target: &str, data: &[u8]) -> Result<(), String> {
        let target_wide = to_wide(target);
        let comment_wide = to_wide("RelWatch master key");

        let credential = CREDENTIALW {
            flags: 0,
            typ: CRED_TYPE_GENERIC,
            target_name: target_wide.as_ptr() as *mut u16,
            comment: comment_wide.as_ptr() as *mut u16,
            last_written: FILETIME {
                dw_low_date_time: 0,
                dw_high_date_time: 0,
            },
            credential_blob_size: data.len() as u32,
            credential_blob: data.as_ptr() as *mut u8,
            persist: CRED_PERSIST_LOCAL_MACHINE, // ← 关键修复
            attribute_count: 0,
            attributes: ptr::null_mut(),
            target_alias: ptr::null_mut(),
            user_name: ptr::null_mut(),
        };

        unsafe {
            let ret = CredWriteW(&credential, 0);
            if ret == 0 {
                let err = GetLastError();
                Err(format!(
                    "Failed to write master key to Windows Credential Manager (error code: {})",
                    err
                ))
            } else {
                Ok(())
            }
        }
    }
}

/// 从 OS keyring 加载 master key，首次运行则随机生成并存储。
///
/// **必须**在 Tauri 启动前（`lib.rs::run()` 中）调用一次。
/// 之后所有 `encrypt`/`decrypt` 操作都使用此缓存的密钥。
pub fn initialize_master_key() -> Result<(), String> {
    let key = platform_load_or_generate_master_key()?;
    MASTER_KEY
        .set(key)
        .map_err(|_| "Master key already initialized".to_string())?;
    Ok(())
}

/// 验证 master key 能否解密 DB 中已有的 v2 密文。
///
/// 应在 DB 初始化后、应用正常启动前调用。
/// 如果 DB 中有 `v2:` 格式的密文但无法用当前 master key 解密，
/// 说明 master key 已丢失（如 Windows 上 keyring 凭据重启后丢失），
/// 此时会自动清空对应的设置项，避免程序无法启动。
/// 返回被清空的 key 名称列表（如 `deepseek_api_key`、`github_token`）。
pub fn verify_master_key_consistency(conn: &rusqlite::Connection) -> Vec<&'static str> {
    let keys_to_check = [
        crate::db::settings::KEY_DEEPSEEK_API_KEY,
        crate::db::settings::KEY_GITHUB_TOKEN,
    ];
    let mut cleared = Vec::new();
    for &key_name in &keys_to_check {
        if let Ok(Some(val)) = crate::db::settings::get_setting(conn, key_name) {
            if val.starts_with(V2_PREFIX) && decrypt_inner(&val).is_none() {
                // 无法解密，清空该设置项
                let _ = crate::db::settings::set_setting(conn, key_name, "");
                cleared.push(key_name);
                eprintln!(
                    "WARNING: '{}' 使用 v2 加密但无法解密（master key 不匹配），已自动清空。\
                     请重新在设置中配置该值。",
                    key_name
                );
            }
        }
    }
    cleared
}

/// decrypt 内部实现（不依赖 MASTER_KEY static）。
/// 供 verify_master_key_consistency 及公开 decrypt 共用。
fn decrypt_inner(encoded: &str) -> Option<String> {
    if let Some(stripped) = encoded.strip_prefix(V2_PREFIX) {
        return decrypt_with_key(stripped, get_master_key());
    }
    let v1_key = v1_derive_key();
    decrypt_with_key(encoded, &v1_key)
}

/// 导出 v1 派生密钥（hostname 派生），供其他模块测试构造 v1 密文用。仅测试用。
#[cfg(test)]
pub fn test_v1_derive_key() -> [u8; 32] {
    v1_derive_key()
}

/// 设置测试用 master key（跳过 keyring）。仅测试用。
#[cfg(test)]
pub fn set_test_master_key() {
    let key = [0x42u8; 32];
    if MASTER_KEY.set(key).is_err() {
        eprintln!("WARNING: set_test_master_key 被多次调用（并行测试中后续调用被忽略）");
    }
}

fn get_master_key() -> &'static [u8; 32] {
    MASTER_KEY
        .get()
        .expect("Master key not initialized. Call initialize_master_key() or set_test_master_key() first.")
}

/// v1 向后兼容：用 hostname 派生密钥（旧版加密方式）
fn v1_derive_key() -> [u8; 32] {
    let host = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "unknown".to_string());
    let seed = format!("RelWatch::v1::{}", host);
    let hash = Sha256::digest(seed.as_bytes());
    let mut key = [0u8; 32];
    key.copy_from_slice(&hash);
    key
}

/// 底层 AES-256-GCM 加密，输出 v2 格式
fn encrypt_with_key(plaintext: &str, key: &[u8; 32]) -> String {
    let cipher = Aes256Gcm::new_from_slice(key).expect("valid key length");
    let nonce_bytes = rand::Rng::gen::<[u8; 12]>(&mut rand::thread_rng());
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .expect("encryption failure");
    let mut combined = Vec::with_capacity(12 + ciphertext.len());
    combined.extend_from_slice(&nonce_bytes);
    combined.extend_from_slice(&ciphertext);
    format!("{}{}", V2_PREFIX, BASE64.encode(&combined))
}

/// 底层 AES-256-GCM 解密（无前缀检查，调⽤方需先 strip 前缀）
fn decrypt_with_key(encoded: &str, key: &[u8; 32]) -> Option<String> {
    let cipher = Aes256Gcm::new_from_slice(key).ok()?;
    let combined = BASE64.decode(encoded).ok()?;
    if combined.len() < 12 {
        return None;
    }
    let (nonce_bytes, ciphertext) = combined.split_at(12);
    let nonce = Nonce::from_slice(nonce_bytes);
    let plaintext = cipher.decrypt(nonce, ciphertext).ok()?;
    String::from_utf8(plaintext).ok()
}

/// 用 master key 加密，输出 `v2:` + base64(nonce ‖ ciphertext)。
pub fn encrypt(plaintext: &str) -> String {
    encrypt_with_key(plaintext, get_master_key())
}

/// 解密，自动兼容 v2（master key）和 v1（hostname 派生密钥）。
pub fn decrypt(encoded: &str) -> Option<String> {
    decrypt_inner(encoded)
}

/// 解密并支持 v1→v2 自动迁移。
///
/// 返回 `(解密后的明文, 迁移后的新 v2 密文)`。
/// 若 `new_encoded` 为 `Some`，调用方可将其写回 DB 完成迁移。
/// 若输入已是 v2 格式，`new_encoded` 为 `None`。
pub fn decrypt_with_migration(encoded: &str) -> Option<(String, Option<String>)> {
    if let Some(stripped) = encoded.strip_prefix(V2_PREFIX) {
        // 已是 v2，无需迁移
        let plain = decrypt_with_key(stripped, get_master_key())?;
        return Some((plain, None));
    }

    // v1 → 尝试 fallback 解密
    let v1_key = v1_derive_key();
    let plain = decrypt_with_key(encoded, &v1_key)?;

    // 检查是否已尝试过迁移该密文（防止回写失败后无限重试）
    let mut cache = MIGRATION_ATTEMPTED.lock().unwrap();
    let attempted = cache.get_or_insert_with(HashSet::new);
    if attempted.contains(encoded) {
        return Some((plain, None));
    }

    // 首次尝试迁移：用 master key 重加密并记录标记
    let v2_encoded = encrypt_with_key(&plain, get_master_key());
    attempted.insert(encoded.to_string());
    Some((plain, Some(v2_encoded)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init() {
        set_test_master_key();
    }

    #[test]
    fn test_crypto_v2_roundtrip() {
        init();
        let plain = "sk-test-key-12345";
        let encrypted = encrypt(plain);
        assert!(encrypted.starts_with(V2_PREFIX), "v2 ciphertext should have prefix");
        assert_ne!(encrypted, plain);
        assert!(!encrypted.contains(plain));

        let decrypted = decrypt(&encrypted);
        assert_eq!(decrypted.unwrap(), plain);
    }

    #[test]
    fn test_crypto_v1_fallback() {
        init();
        let plain = "ghp_legacy_token";
        // 模拟 v1 格式（hostname 派生）
        let v1_key = v1_derive_key();
        let v1_encrypted = {
            let cipher = Aes256Gcm::new_from_slice(&v1_key).unwrap();
            let nonce_bytes = rand::Rng::gen::<[u8; 12]>(&mut rand::thread_rng());
            let nonce = Nonce::from_slice(&nonce_bytes);
            let ciphertext = cipher
                .encrypt(nonce, plain.as_bytes())
                .expect("encryption failure");
            let mut combined = Vec::with_capacity(12 + ciphertext.len());
            combined.extend_from_slice(&nonce_bytes);
            combined.extend_from_slice(&ciphertext);
            BASE64.encode(&combined)
        };
        assert!(!v1_encrypted.starts_with(V2_PREFIX), "v1 should not have prefix");

        // 通过 decrypt（v1 fallback）应能解出
        let decrypted = decrypt(&v1_encrypted);
        assert_eq!(decrypted.unwrap(), plain);
    }

    #[test]
    fn test_decrypt_with_migration_v2_returns_none() {
        init();
        let plain = "my-token";
        let v2_encrypted = encrypt(plain);

        let (decrypted, migration) = decrypt_with_migration(&v2_encrypted).unwrap();
        assert_eq!(decrypted, plain);
        assert!(migration.is_none(), "v2 input should not trigger migration");
    }

    #[test]
    fn test_decrypt_with_migration_v1_returns_new_encoded() {
        init();
        let plain = "ghp_legacy_migrate";
        // 生成 v1 密文
        let v1_key = v1_derive_key();
        let v1_encrypted = {
            let cipher = Aes256Gcm::new_from_slice(&v1_key).unwrap();
            let nonce_bytes = rand::Rng::gen::<[u8; 12]>(&mut rand::thread_rng());
            let nonce = Nonce::from_slice(&nonce_bytes);
            let ciphertext = cipher
                .encrypt(nonce, plain.as_bytes())
                .expect("encryption failure");
            let mut combined = Vec::with_capacity(12 + ciphertext.len());
            combined.extend_from_slice(&nonce_bytes);
            combined.extend_from_slice(&ciphertext);
            BASE64.encode(&combined)
        };

        let (decrypted, migration) = decrypt_with_migration(&v1_encrypted).unwrap();
        assert_eq!(decrypted, plain);
        assert!(migration.is_some(), "v1 input should trigger migration");

        // 迁移后的 v2 密文应能通过标准 decrypt 解密
        let v2_encrypted = migration.unwrap();
        assert!(v2_encrypted.starts_with(V2_PREFIX));
        let re_decrypted = decrypt(&v2_encrypted);
        assert_eq!(re_decrypted.unwrap(), plain);
    }

    #[test]
    fn test_crypto_decrypt_invalid() {
        init();
        assert!(decrypt("not-valid-base64!!").is_none());
        assert!(decrypt("").is_none());
        assert!(decrypt("YWJj").is_none());
        assert!(decrypt("v2:").is_none());
        assert!(decrypt("v2:not-valid-base64").is_none());
    }

    #[test]
    fn test_crypto_deterministic_key() {
        init();
        let plain = "hello";
        let enc1 = encrypt(plain);
        let enc2 = encrypt(plain);
        assert_ne!(enc1, enc2, "nonce should make each encryption unique");

        assert_eq!(decrypt(&enc1).unwrap(), plain);
        assert_eq!(decrypt(&enc2).unwrap(), plain);
    }

    #[test]
    fn test_decrypt_with_migration_idempotent_after_failed_writeback() {
        init();
        let plain = "ghp_retry_loop_prevention";
        // 生成 v1 密文
        let v1_key = v1_derive_key();
        let v1_encrypted = {
            let cipher = Aes256Gcm::new_from_slice(&v1_key).unwrap();
            let nonce_bytes = rand::Rng::gen::<[u8; 12]>(&mut rand::thread_rng());
            let nonce = Nonce::from_slice(&nonce_bytes);
            let ciphertext = cipher
                .encrypt(nonce, plain.as_bytes())
                .expect("encryption failure");
            let mut combined = Vec::with_capacity(12 + ciphertext.len());
            combined.extend_from_slice(&nonce_bytes);
            combined.extend_from_slice(&ciphertext);
            BASE64.encode(&combined)
        };

        // 第一次调用：应触发迁移（模拟回写失败场景）
        let (decrypted1, migration1) = decrypt_with_migration(&v1_encrypted).unwrap();
        assert_eq!(decrypted1, plain);
        assert!(migration1.is_some(), "first call should trigger migration");

        // 第二次调用：应跳过迁移（缓存命中，模拟无限循环防护）
        let (decrypted2, migration2) = decrypt_with_migration(&v1_encrypted).unwrap();
        assert_eq!(decrypted2, plain);
        assert!(migration2.is_none(), "second call should skip migration due to cache");
    }

    // ── Keyring 交互测试（使用 keyring mock） ────────────────
    //
    // 这些测试依赖 keyring 全局 state（set_default_credential_builder），
    // 必须串行执行，通过 KEYRING_MUTEX 保证。

    use std::sync::Mutex as StdMutex;
    static KEYRING_MUTEX: StdMutex<()> = StdMutex::new(());

    /// 在测试中用 keyring mock 初始化一片干净的测试环境。
    /// 返回新创建的 keyring::Entry（用独立的 service 名避免干扰）。
    fn mock_entry(name: &str) -> keyring::Entry {
        keyring::set_default_credential_builder(keyring::mock::default_credential_builder());
        keyring::Entry::new("crypto_test", name).expect("mock entry should succeed")
    }

    #[test]
    fn test_load_or_generate_first_run() {
        // 模拟首次运行：keyring 中无条目 → 生成新 key
        let _lock = KEYRING_MUTEX.lock().unwrap();

        let entry = mock_entry("first_run");
        let key = load_or_generate_master_key(&entry).expect("should generate key on first run");

        assert_eq!(key.len(), 32, "generated key should be 32 bytes");
        // 验证 key 已写入 keyring
        let stored = entry.get_password().expect("should have stored password");
        let decoded = BASE64.decode(stored.as_bytes()).expect("stored value should be valid base64");
        assert_eq!(decoded.len(), 32, "stored key should be 32 bytes");
        assert_eq!(decoded.as_slice(), &key[..], "stored key should match returned key");
    }

    #[test]
    fn test_load_or_generate_existing_key() {
        // 模拟已有 key：keyring 中有有效密钥 → 正确加载
        let _lock = KEYRING_MUTEX.lock().unwrap();

        let entry = mock_entry("existing_key");
        let expected = [0xABu8; 32];
        entry
            .set_password(&BASE64.encode(expected))
            .expect("should set password");

        let key = load_or_generate_master_key(&entry).expect("should load existing key");
        assert_eq!(key, expected, "should return the stored key");
    }

    #[test]
    fn test_load_or_generate_corrupted_base64() {
        // 模拟 keyring 中数据损坏（无效 base64）→ 报错
        let _lock = KEYRING_MUTEX.lock().unwrap();

        let entry = mock_entry("corrupted_b64");
        entry
            .set_password("!!!invalid-base64!!!")
            .expect("should set password");

        let result = load_or_generate_master_key(&entry);
        assert!(result.is_err(), "corrupted base64 should error");
        assert!(
            result.unwrap_err().contains("corrupted"),
            "error should mention corruption"
        );
    }

    #[test]
    fn test_load_or_generate_wrong_length() {
        // 模拟 keyring 中 key 长度错误 → 报错
        let _lock = KEYRING_MUTEX.lock().unwrap();

        let entry = mock_entry("wrong_len");
        let short_key = [0x42u8; 16]; // 16 bytes != 32
        entry
            .set_password(&BASE64.encode(short_key))
            .expect("should set password");

        let result = load_or_generate_master_key(&entry);
        assert!(result.is_err(), "wrong length should error");
        assert!(
            result.unwrap_err().contains("wrong length"),
            "error should mention wrong length"
        );
    }

    #[test]
    fn test_load_or_generate_keyring_error() {
        // 模拟 keyring API 错误 → 传播错误
        let _lock = KEYRING_MUTEX.lock().unwrap();

        let entry = mock_entry("api_error");
        // 通过 mock 设置一个错误
        let mock: &keyring::mock::MockCredential = entry
            .get_credential()
            .downcast_ref()
            .expect("should downcast to MockCredential");
        mock.set_error(keyring::Error::NoStorageAccess(
            "simulated storage error".into(),
        ));

        let result = load_or_generate_master_key(&entry);
        assert!(result.is_err(), "keyring error should propagate");
    }

    // ── 密钥一致性检查测试 ────────────────────────────────

    #[test]
    fn test_verify_master_key_consistency_no_v2_data() {
        // DB 中没有 v2 密文 → 返回空
        init();
        let conn = crate::db::init::init_memory_db().expect("in-memory db");

        let cleared = verify_master_key_consistency(&conn);
        assert!(cleared.is_empty(), "no v2 data should return empty");
    }

    #[test]
    fn test_verify_master_key_consistency_with_v2_matching() {
        // DB 中有 v2 密文且 master key 匹配 → 返回空
        init();
        let conn = crate::db::init::init_memory_db().expect("in-memory db");

        let plain = "sk-my-deepseek-key";
        let encrypted = encrypt(plain); // 用当前 master key 加密
        assert!(encrypted.starts_with(V2_PREFIX));

        crate::db::settings::set_setting(
            &conn,
            crate::db::settings::KEY_DEEPSEEK_API_KEY,
            &encrypted,
        )
        .expect("should set setting");

        let cleared = verify_master_key_consistency(&conn);
        assert!(cleared.is_empty(), "matching key should return empty");

        // 验证数据未被清空且能解密
        let stored =
            crate::db::settings::get_setting(&conn, crate::db::settings::KEY_DEEPSEEK_API_KEY)
                .expect("should read")
                .expect("should have value");
        let decrypted = decrypt(&stored).expect("should decrypt");
        assert_eq!(decrypted, plain);
    }

    #[test]
    fn test_verify_master_key_consistency_mismatch_clears() {
        // DB 中有 v2 密文但 master key 不匹配 → 自动清空
        init();
        let conn = crate::db::init::init_memory_db().expect("in-memory db");

        // 用不同的 key 加密数据
        let wrong_key = [0x99u8; 32];
        let plain = "some-secret-value";
        let encrypted = encrypt_with_key(plain, &wrong_key);
        assert!(encrypted.starts_with(V2_PREFIX));

        crate::db::settings::set_setting(
            &conn,
            crate::db::settings::KEY_DEEPSEEK_API_KEY,
            &encrypted,
        )
        .expect("should set setting");

        // 调用一致性检查 → 应清空数据
        let cleared = verify_master_key_consistency(&conn);
        assert!(!cleared.is_empty(), "should clear mismatched key");
        assert!(
            cleared.contains(&crate::db::settings::KEY_DEEPSEEK_API_KEY),
            "should report deepseek_api_key as cleared"
        );

        // 验证 DB 中已被清空
        let stored = crate::db::settings::get_setting(
            &conn,
            crate::db::settings::KEY_DEEPSEEK_API_KEY,
        )
        .expect("should read")
        .unwrap_or_default();
        assert_eq!(stored, "", "mismatched data should be cleared");
    }
}
