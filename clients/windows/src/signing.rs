//! Device-local HMAC-SHA256 payload tags.
//!
//! Each device generates a 32-byte secret key once, encrypts it with Windows
//! DPAPI (user-scoped, see [`crate::dpapi`]), and stores it next to the
//! credentials. Every activity payload written to the local SQLite queue is
//! tagged with that key before it reaches disk.
//!
//! **This is not tamper protection the server enforces.** The key is never
//! sent to the server and the server does not verify the tag, so an edited
//! queue row is uploaded like any other; a row whose images were stripped
//! after a 413 no longer matches its tag either. Real attestation needs a
//! per-device key registered with the server, a signature over the exact
//! uploaded body and server-side verification — none of which exists yet.
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
            if let Ok(key) = crate::dpapi::unprotect(&raw, None) {
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
    crate::fsutil::write_atomically(path, &crate::dpapi::protect(key, None)?)
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
