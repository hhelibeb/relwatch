//! 凭据读取统一管道：读取 → 解密 → v1→v2 迁移回写。
//!
//! 原先 poll.rs（3 份）、commands/source.rs（3 份）、deepseek.rs（1 份）各自内联
//! 同一段「get_setting → 判空 → decrypt_with_migration → 回写」逻辑，仅 key 不同。
//! 收敛为本模块唯一实现，调用方只需传入 settings key 常量。

use rusqlite::Connection;

use crate::crypto;
use crate::db;

/// 读取并解密指定设置键中的凭据（含 v1→v2 加密迁移回写）。
///
/// - 未设置或读取失败 → `None`
/// - 空字符串 → `None`（未配置）
/// - v1 密文 → 解密并返回迁移后的 v2 密文（回写失败仅告警，不阻塞）
pub fn read_credential(conn: &Connection, key: &str) -> Option<String> {
    let encrypted = db::settings::get_setting(conn, key).ok()??;
    if encrypted.is_empty() {
        return None;
    }
    let (plain, new_v2) = crypto::decrypt_with_migration(&encrypted)?;
    if let Some(new_val) = new_v2 {
        if let Err(e) = db::settings::set_setting(conn, key, &new_val) {
            log::warn!("迁移 v1→v2 凭据 {} 回写失败: {}", key, e);
        }
    }
    Some(plain)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes_gcm::aead::{Aead, KeyInit};
    use base64::Engine;
    use crate::db::init::init_memory_db;
    use crate::db::settings::{KEY_GITHUB_TOKEN, KEY_YOUTUBE_API_KEY};

    #[test]
    fn test_read_credential_no_key_returns_none() {
        let conn = init_memory_db().unwrap();
        assert!(read_credential(&conn, KEY_GITHUB_TOKEN).is_none(), "未设置凭据时应返回 None");
    }

    #[test]
    fn test_read_credential_empty_string_returns_none() {
        let conn = init_memory_db().unwrap();
        crate::crypto::set_test_master_key();
        db::settings::set_setting(&conn, KEY_GITHUB_TOKEN, "").unwrap();
        assert!(read_credential(&conn, KEY_GITHUB_TOKEN).is_none(), "空字符串凭据时应返回 None");
    }

    #[test]
    fn test_read_credential_valid_returns_decrypted() {
        let conn = init_memory_db().unwrap();
        crate::crypto::set_test_master_key();
        let encrypted = crate::crypto::encrypt("ghp_test_token");
        db::settings::set_setting(&conn, KEY_GITHUB_TOKEN, &encrypted).unwrap();
        let result = read_credential(&conn, KEY_GITHUB_TOKEN);
        assert_eq!(result.as_deref(), Some("ghp_test_token"), "应解密返回原始凭据");
    }

    #[test]
    fn test_read_credential_v1_migrates_and_writeback() {
        let conn = init_memory_db().unwrap();
        crate::crypto::set_test_master_key();

        // 构造 v1 密文（hostname 派生密钥加密，无 v2: 前缀）
        let plain = "ghp_legacy_token";
        let v1_encrypted = {
            let key = crate::crypto::test_v1_derive_key();
            let cipher = aes_gcm::Aes256Gcm::new_from_slice(&key).unwrap();
            let nonce_bytes = rand::Rng::gen::<[u8; 12]>(&mut rand::thread_rng());
            let nonce = aes_gcm::Nonce::from_slice(&nonce_bytes);
            let ciphertext = cipher.encrypt(nonce, plain.as_bytes()).expect("encryption failure");
            let mut combined = Vec::with_capacity(12 + ciphertext.len());
            combined.extend_from_slice(&nonce_bytes);
            combined.extend_from_slice(&ciphertext);
            base64::engine::general_purpose::STANDARD.encode(&combined)
        };
        db::settings::set_setting(&conn, KEY_GITHUB_TOKEN, &v1_encrypted).unwrap();

        // 读取触发迁移，返回明文
        let result = read_credential(&conn, KEY_GITHUB_TOKEN);
        assert_eq!(result.as_deref(), Some(plain));

        // 回写后 DB 中应为 v2 密文，可直接解密
        let stored = db::settings::get_setting(&conn, KEY_GITHUB_TOKEN).unwrap().unwrap();
        assert!(stored.starts_with("v2:"), "迁移后应写回 v2 密文");
        assert_eq!(crate::crypto::decrypt(&stored).as_deref(), Some(plain));
    }

    #[test]
    fn test_read_credential_different_keys_independent() {
        let conn = init_memory_db().unwrap();
        crate::crypto::set_test_master_key();
        db::settings::set_setting(&conn, KEY_YOUTUBE_API_KEY, &crate::crypto::encrypt("yt-key")).unwrap();
        assert!(read_credential(&conn, KEY_GITHUB_TOKEN).is_none());
        assert_eq!(read_credential(&conn, KEY_YOUTUBE_API_KEY).as_deref(), Some("yt-key"));
    }
}
