use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use sha2::{Digest, Sha256};

fn derive_key() -> [u8; 32] {
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

pub fn encrypt(plaintext: &str) -> String {
    let key = derive_key();
    let cipher = Aes256Gcm::new_from_slice(&key).expect("valid key length");
    let nonce_bytes = rand::Rng::gen::<[u8; 12]>(&mut rand::thread_rng());
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .expect("encryption failure");
    let mut combined = Vec::with_capacity(12 + ciphertext.len());
    combined.extend_from_slice(&nonce_bytes);
    combined.extend_from_slice(&ciphertext);
    BASE64.encode(&combined)
}

pub fn decrypt(encoded: &str) -> Option<String> {
    let key = derive_key();
    let cipher = Aes256Gcm::new_from_slice(&key).ok()?;
    let combined = BASE64.decode(encoded).ok()?;
    if combined.len() < 12 {
        return None;
    }
    let (nonce_bytes, ciphertext) = combined.split_at(12);
    let nonce = Nonce::from_slice(nonce_bytes);
    let plaintext = cipher.decrypt(nonce, ciphertext).ok()?;
    String::from_utf8(plaintext).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crypto_roundtrip() {
        let plain = "sk-test-key-12345";
        let encrypted = encrypt(plain);
        assert_ne!(encrypted, plain);
        assert!(!encrypted.contains(plain));

        let decrypted = decrypt(&encrypted);
        assert_eq!(decrypted.unwrap(), plain);
    }

    #[test]
    fn test_crypto_decrypt_invalid() {
        assert!(decrypt("not-valid-base64!!").is_none());
        assert!(decrypt("").is_none());
        assert!(decrypt("YWJj").is_none());
    }

    #[test]
    fn test_crypto_deterministic_key() {
        let plain = "hello";
        let enc1 = encrypt(plain);
        let enc2 = encrypt(plain);
        assert_ne!(enc1, enc2, "nonce should make each encryption unique");

        assert_eq!(decrypt(&enc1).unwrap(), plain);
        assert_eq!(decrypt(&enc2).unwrap(), plain);
    }
}
