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

/// 从 OS keyring 加载 master key，首次运行则随机生成并存储。
///
/// **必须**在 Tauri 启动前（`lib.rs::run()` 中）调用一次。
/// 之后所有 `encrypt`/`decrypt` 操作都使用此缓存的密钥。
pub fn initialize_master_key() -> Result<(), String> {
    let entry = keyring::Entry::new("relwatch", "master-key")
        .map_err(|e| format!("Failed to create keyring entry: {}", e))?;

    let key = match entry.get_password() {
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
            key
        }
        Err(keyring::Error::NoEntry) => {
            // 首次运行：生成随机 32B 密钥并写入 OS keyring
            let key: [u8; 32] = rand::Rng::gen(&mut rand::thread_rng());
            let encoded = BASE64.encode(key);
            entry
                .set_password(&encoded)
                .map_err(|e| format!("Failed to write master key to OS keyring: {}", e))?;
            key
        }
        Err(e) => {
            return Err(format!(
                "Failed to read master key from OS keyring: {}. \
                 The application cannot start without OS keyring access.",
                e
            ));
        }
    };

    MASTER_KEY
        .set(key)
        .map_err(|_| "Master key already initialized".to_string())?;
    Ok(())
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
    if let Some(stripped) = encoded.strip_prefix(V2_PREFIX) {
        // v2：用 master key
        return decrypt_with_key(stripped, get_master_key());
    }
    // v1 fallback：用 hostname 派生密钥
    let v1_key = v1_derive_key();
    decrypt_with_key(encoded, &v1_key)
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
}
