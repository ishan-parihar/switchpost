// ─── Token Encryption ─────────────────────────────────────────
// AES-256-GCM encryption/decryption for access tokens and refresh
// tokens stored in the database. Uses a key derived from a config-
// provided hex string (TOKEN_ENCRYPTION_KEY, 64 hex chars = 32 bytes).

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use rand::RngCore;

/// Decode a 64-character hex string into a 32-byte AES-256 key.
pub fn decode_hex_key(hex_key: &str) -> Result<[u8; 32], String> {
    let bytes = hex::decode(hex_key).map_err(|e| format!("Invalid encryption key hex: {e}"))?;
    if bytes.len() != 32 {
        return Err(format!(
            "Encryption key must be 64 hex chars (got {} chars: {} bytes)",
            hex_key.len(),
            bytes.len()
        ));
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(&bytes);
    Ok(key)
}

/// Encrypt a plaintext string with AES-256-GCM.
/// Returns hex-encoded (nonce || ciphertext).
pub fn encrypt_string(plaintext: &str, key: &[u8; 32]) -> Result<String, String> {
    let cipher =
        Aes256Gcm::new_from_slice(key).map_err(|e| format!("Failed to create cipher: {e}"))?;

    // Generate random 96-bit nonce
    let mut nonce_bytes = [0u8; 12];
    rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|e| format!("Encryption failed: {e}"))?;

    // Encode as: nonce (12 bytes) || ciphertext (variable)
    let mut combined = nonce_bytes.to_vec();
    combined.extend(ciphertext);
    Ok(hex::encode(combined))
}

/// Decrypt a hex-encoded (nonce || ciphertext) string with AES-256-GCM.
pub fn decrypt_string(encrypted_hex: &str, key: &[u8; 32]) -> Result<String, String> {
    let data = hex::decode(encrypted_hex).map_err(|e| format!("Invalid hex encoding: {e}"))?;

    if data.len() < 12 {
        return Err("Encrypted data too short (must be at least 12 bytes)".into());
    }

    let (nonce_bytes, ciphertext) = data.split_at(12);
    let nonce = Nonce::from_slice(nonce_bytes);

    let cipher =
        Aes256Gcm::new_from_slice(key).map_err(|e| format!("Failed to create cipher: {e}"))?;

    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| format!("Decryption failed (wrong key or corrupted data): {e}"))?;

    String::from_utf8(plaintext).map_err(|e| format!("Decrypted data is not valid UTF-8: {e}"))
}

/// Maybe-decrypt an integration access token. If `key` is `Some`,
/// attempts decryption and falls back to the raw value on failure
/// (which usually means the token was stored before encryption was
/// enabled). If `key` is `None`, returns the raw value unchanged.
///
/// This is the single source of truth for the "decrypt-if-encrypted"
/// pattern used by ~30 MCP tools, the REST API analytics handlers,
/// and the scheduler. Use this instead of inlining the
/// `state.token_key.as_ref().and_then(...).unwrap_or(...)` chain.
pub fn maybe_decrypt_token(token: &str, key: Option<&[u8; 32]>) -> String {
    match key {
        Some(k) => decrypt_string(token, k).unwrap_or_else(|_| token.to_string()),
        None => token.to_string(),
    }
}

/// Does this stored value LOOK like something this module produced?
///
/// `encrypt_string` writes hex(nonce || ciphertext): an even-length hex
/// string of at least 12 + 16 = 28 bytes, so 56 hex characters.
///
/// This is the cheap half of "is this row already encrypted?". It answers
/// the shape question WITHOUT the key, so a row that was encrypted with a
/// DIFFERENT key still looks encrypted and is never re-encrypted (which
/// would double-encrypt it and destroy it). Pair it with
/// [`decrypt_string`] for the exact answer when the key is known.
pub fn looks_encrypted(value: &str) -> bool {
    if value.len() < 56 || value.len() % 2 != 0 {
        return false;
    }
    value.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Is this stored value encrypted WITH THIS KEY?
///
/// True only when it decrypts cleanly. A row that fails this check but
/// passes [`looks_encrypted`] was written with another key (or is
/// corrupt) and must be left alone.
pub fn is_encrypted_with(value: &str, key: &[u8; 32]) -> bool {
    decrypt_string(value, key).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> [u8; 32] {
        let mut k = [0u8; 32];
        k.copy_from_slice(b"01234567890123456789012345678901"); // 32 bytes
        k
    }

    #[test]
    fn test_round_trip() {
        let key = test_key();
        let original = "my-super-secret-access-token-12345";
        let encrypted = encrypt_string(original, &key).expect("encrypt");
        let decrypted = decrypt_string(&encrypted, &key).expect("decrypt");
        assert_eq!(original, decrypted);
    }

    #[test]
    fn test_differs_each_time() {
        let key = test_key();
        let plaintext = "same-token";
        let e1 = encrypt_string(plaintext, &key).expect("e1");
        let e2 = encrypt_string(plaintext, &key).expect("e2");
        // Random nonce each time → different ciphertext
        assert_ne!(e1, e2, "encryptions should differ due to random nonce");
    }

    #[test]
    fn test_wrong_key_fails() {
        let key1 = test_key();
        let mut key2 = key1;
        key2[0] ^= 1; // flip a bit
        let encrypted = encrypt_string("secret", &key1).expect("encrypt");
        let result = decrypt_string(&encrypted, &key2);
        assert!(result.is_err(), "wrong key should fail decryption");
    }

    #[test]
    fn test_invalid_hex() {
        let key = test_key();
        let result = decrypt_string("not-hex", &key);
        assert!(result.is_err());
    }

    #[test]
    fn test_too_short_data() {
        let key = test_key();
        let result = decrypt_string("aabb", &key); // 2 bytes, less than 12-byte nonce
        assert!(result.is_err());
    }

    #[test]
    fn test_decode_hex_key_valid() {
        let hex_key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let key = decode_hex_key(hex_key).expect("valid key");
        assert_eq!(key.len(), 32);
    }

    #[test]
    fn test_decode_hex_key_wrong_length() {
        let result = decode_hex_key("aabb");
        assert!(result.is_err(), "should reject short keys");
    }

    #[test]
    fn test_decode_hex_key_invalid_hex() {
        let result =
            decode_hex_key("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz");
        assert!(result.is_err(), "should reject non-hex chars");
    }
}
