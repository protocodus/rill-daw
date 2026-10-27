//! Settings-file encryption is protection against casual inspection only: the
//! application contains this key. It is not a substitute for an OS keychain.

use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit, Payload},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use std::fmt;
use zeroize::Zeroize;

const SETTINGS_KEY: [u8; 32] = [
    0x72, 0x69, 0x6c, 0x6c, 0x8d, 0x26, 0x3f, 0x91, 0xbc, 0x02, 0xe7, 0x4a, 0x59, 0xd0, 0xa1, 0x6c,
    0x15, 0xf3, 0x48, 0x9b, 0x67, 0x2e, 0xc0, 0x85, 0xda, 0x31, 0x7f, 0x04, 0xb6, 0x53, 0x98, 0xed,
];
const ASSOCIATED_DATA: &[u8] = b"rill-provider-credential-v1";
const MAX_KEY_BYTES: usize = 4096;

/// Deliberately neither serializable nor printable. Settings store EncryptedSecret.
#[derive(Clone)]
pub struct SecretString(String);

impl SecretString {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretString([REDACTED])")
    }
}

impl Drop for SecretString {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EncryptedSecret {
    pub version: u8,
    pub nonce: String,
    pub ciphertext: String,
}

impl fmt::Debug for EncryptedSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EncryptedSecret")
            .field("version", &self.version)
            .field("data", &"[REDACTED]")
            .finish()
    }
}

pub fn encrypt_api_key(value: &str) -> Result<EncryptedSecret, String> {
    if value.is_empty() || value.len() > MAX_KEY_BYTES {
        return Err("Enter a nonempty API key of at most 4096 bytes".into());
    }
    let mut nonce = [0u8; 12];
    getrandom::fill(&mut nonce).map_err(|_| "Could not obtain secure randomness")?;
    let cipher = Aes256Gcm::new_from_slice(&SETTINGS_KEY).expect("AES key has fixed length");
    let ciphertext = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: value.as_bytes(),
                aad: ASSOCIATED_DATA,
            },
        )
        .map_err(|_| "Could not encrypt API key")?;
    Ok(EncryptedSecret {
        version: 1,
        nonce: STANDARD.encode(nonce),
        ciphertext: STANDARD.encode(ciphertext),
    })
}

pub fn decrypt_api_key(secret: &EncryptedSecret) -> Result<SecretString, String> {
    if secret.version != 1 || secret.nonce.len() != 16 || secret.ciphertext.len() > 5500 {
        return Err("Unsupported or invalid encrypted API key; enter the key again".into());
    }
    let nonce = STANDARD
        .decode(&secret.nonce)
        .map_err(|_| "Invalid encrypted API key")?;
    if nonce.len() != 12 {
        return Err("Invalid encrypted API key".into());
    }
    let ciphertext = STANDARD
        .decode(&secret.ciphertext)
        .map_err(|_| "Invalid encrypted API key")?;
    let cipher = Aes256Gcm::new_from_slice(&SETTINGS_KEY).expect("AES key has fixed length");
    let bytes = cipher
        .decrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: &ciphertext,
                aad: ASSOCIATED_DATA,
            },
        )
        .map_err(|_| "Could not decrypt API key; enter the key again")?;
    let value = String::from_utf8(bytes).map_err(|_| "Invalid decrypted API key")?;
    if value.is_empty() || value.len() > MAX_KEY_BYTES {
        return Err("Invalid decrypted API key".into());
    }
    Ok(SecretString::new(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authenticated_randomized_roundtrip_and_redacted_debug() {
        let first = encrypt_api_key("test-provider-secret").unwrap();
        let second = encrypt_api_key("test-provider-secret").unwrap();
        assert_ne!(first, second);
        assert_eq!(
            decrypt_api_key(&first).unwrap().expose_secret(),
            "test-provider-secret"
        );
        assert!(
            !serde_json::to_string(&first)
                .unwrap()
                .contains("test-provider-secret")
        );
        assert!(
            !format!("{:?}", decrypt_api_key(&first).unwrap()).contains("test-provider-secret")
        );
        let mut tampered = first;
        tampered.ciphertext.replace_range(0..1, "!");
        assert!(decrypt_api_key(&tampered).is_err());
    }

    #[test]
    fn rejects_wrong_version_and_modified_ciphertext() {
        let mut secret = encrypt_api_key("key").unwrap();
        let mut bytes = STANDARD.decode(&secret.ciphertext).unwrap();
        bytes[0] ^= 1;
        secret.ciphertext = STANDARD.encode(bytes);
        assert!(decrypt_api_key(&secret).is_err());
        secret.version = 2;
        assert!(decrypt_api_key(&secret).is_err());
    }
}
