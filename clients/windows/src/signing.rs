//! Device-local HMAC-SHA256 payload signing.
//!
//! Each device generates a 32-byte secret key once, encrypts it with Windows
//! DPAPI (user-scoped, same as the credentials file), and stores it next to
//! the credentials. Every activity payload written to the local SQLite queue is
//! signed with that key before it reaches disk. The server verifies the
//! signature on upload using the key it received at heartbeat registration, so
//! a row whose `sig` column has been edited (or whose payload was modified after
//! signing) is rejected before it can affect billing or reports.
//!
//! `sign_payload` and `verify_payload` are HMAC-SHA256 over the canonical UTF-8
//! JSON bytes of the activity as serialised by `serde_json`. Any field edit
//! (start time, counts, etc.) changes those bytes and therefore invalidates the
//! MAC, even if the JSON is re-minified to look identical.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use base64::Engine as _;
use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Where the device signing key is kept: `%APPDATA%\DosiTracker\signing.key`.
pub fn default_key_path() -> Result<PathBuf> {
    let dir = dirs::config_dir()
        .ok_or_else(|| anyhow!("could not resolve the user config directory"))?
        .join("DosiTracker");
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join("signing.key"))
}

/// Load the device signing key, generating and persisting it if this is the
/// first run. The raw key bytes are DPAPI-encrypted at rest.
pub fn load_or_create_key() -> Result<Vec<u8>> {
    let path = default_key_path()?;
    load_or_create_key_at(&path)
}

pub fn load_or_create_key_at(path: &Path) -> Result<Vec<u8>> {
    if path.exists() {
        if let Ok(raw) = std::fs::read(path) {
            if let Ok(key) = dpapi::unprotect(&raw) {
                if key.len() == 32 {
                    return Ok(key);
                }
            }
        }
        // Corrupt or wrong-size blob — regenerate.
        tracing::warn!("device signing key is corrupt; regenerating");
    }
    let key = generate_key();
    save_key_at(path, &key)?;
    Ok(key)
}

fn generate_key() -> Vec<u8> {
    use rand::RngCore;
    let mut key = vec![0u8; 32];
    rand::thread_rng().fill_bytes(&mut key);
    key
}

fn save_key_at(path: &Path, key: &[u8]) -> Result<()> {
    let encrypted = dpapi::protect(key)?;
    let mut temp = path.as_os_str().to_os_string();
    temp.push(".tmp");
    let temp = PathBuf::from(temp);
    std::fs::write(&temp, &encrypted)?;
    std::fs::rename(&temp, path)?;
    Ok(())
}

/// Sign `payload_json` (the canonical bytes written to the SQLite queue) and
/// return a base64url-encoded HMAC-SHA256 tag.
pub fn sign_payload(key: &[u8], payload_json: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key size");
    mac.update(payload_json);
    let tag = mac.finalize().into_bytes();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(tag)
}

#[cfg_attr(not(test), expect(dead_code, reason = "used by the backend; included here for reference tests"))]
/// Return `true` if `sig` is the valid HMAC-SHA256 tag for `payload_json`.
pub fn verify_payload(key: &[u8], payload_json: &[u8], sig: &str) -> bool {
    let Ok(tag) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(sig) else {
        return false;
    };
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key size");
    mac.update(payload_json);
    mac.verify_slice(&tag).is_ok()
}

// ─── DPAPI shim (same pattern as credentials.rs) ────────────────────────────

#[cfg(windows)]
mod dpapi {
    use anyhow::{anyhow, Result};
    use windows::Win32::Foundation::LocalFree;
    use windows::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPT_INTEGER_BLOB,
    };

    unsafe fn take_blob(blob: &CRYPT_INTEGER_BLOB) -> Vec<u8> {
        let out = if blob.pbData.is_null() {
            Vec::new()
        } else {
            unsafe { std::slice::from_raw_parts(blob.pbData, blob.cbData as usize) }.to_vec()
        };
        if !blob.pbData.is_null() {
            let _ = unsafe {
                LocalFree(windows::Win32::Foundation::HLOCAL(
                    blob.pbData as *mut core::ffi::c_void,
                ))
            };
        }
        out
    }

    pub fn protect(plaintext: &[u8]) -> Result<Vec<u8>> {
        let input = CRYPT_INTEGER_BLOB {
            cbData: plaintext.len() as u32,
            pbData: plaintext.as_ptr() as *mut u8,
        };
        let mut output = CRYPT_INTEGER_BLOB::default();
        unsafe {
            CryptProtectData(
                &input,
                None,
                None,
                None,
                None,
                windows::Win32::Security::Cryptography::CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
            .map_err(|e| anyhow!("CryptProtectData failed: {e}"))?;
            Ok(take_blob(&output))
        }
    }

    pub fn unprotect(ciphertext: &[u8]) -> Result<Vec<u8>> {
        let input = CRYPT_INTEGER_BLOB {
            cbData: ciphertext.len() as u32,
            pbData: ciphertext.as_ptr() as *mut u8,
        };
        let mut output = CRYPT_INTEGER_BLOB::default();
        unsafe {
            CryptUnprotectData(
                &input,
                None,
                None,
                None,
                None,
                windows::Win32::Security::Cryptography::CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
            .map_err(|e| anyhow!("CryptUnprotectData failed: {e}"))?;
            Ok(take_blob(&output))
        }
    }
}

#[cfg(not(windows))]
mod dpapi {
    use anyhow::Result;
    pub fn protect(plaintext: &[u8]) -> Result<Vec<u8>> {
        Ok(plaintext.to_vec())
    }
    pub fn unprotect(ciphertext: &[u8]) -> Result<Vec<u8>> {
        Ok(ciphertext.to_vec())
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> Vec<u8> {
        (0u8..32).collect()
    }

    #[test]
    fn sign_and_verify_roundtrip() {
        let key = test_key();
        let payload = br#"{"clientActivityId":"abc","projectId":"p1"}"#;
        let sig = sign_payload(&key, payload);
        assert!(verify_payload(&key, payload, &sig));
    }

    #[test]
    fn different_key_fails() {
        let key = test_key();
        let other_key: Vec<u8> = (1u8..33).collect();
        let payload = br#"{"clientActivityId":"abc"}"#;
        let sig = sign_payload(&key, payload);
        assert!(!verify_payload(&other_key, payload, &sig));
    }

    #[test]
    fn modified_payload_fails() {
        let key = test_key();
        let payload = br#"{"clientActivityId":"abc","mouseClicks":5}"#;
        let sig = sign_payload(&key, payload);
        let tampered = br#"{"clientActivityId":"abc","mouseClicks":999}"#;
        assert!(!verify_payload(&key, tampered, &sig));
    }

    #[test]
    fn garbage_sig_fails() {
        let key = test_key();
        let payload = br#"{"clientActivityId":"abc"}"#;
        assert!(!verify_payload(&key, payload, "not-a-valid-sig"));
        assert!(!verify_payload(&key, payload, ""));
    }

    #[test]
    fn key_roundtrips_through_dpapi() {
        let path = std::env::temp_dir()
            .join(format!("dosi-signing-{}.key", uuid::Uuid::new_v4()));
        let key = load_or_create_key_at(&path).expect("first call generates and saves");
        assert_eq!(key.len(), 32);
        let key2 = load_or_create_key_at(&path).expect("second call loads the same key");
        assert_eq!(key, key2, "same key returned on reload");
        let _ = std::fs::remove_file(&path);
    }
}
