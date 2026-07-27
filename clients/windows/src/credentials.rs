//! Persisted sign-in, encrypted at rest with Windows DPAPI.
//!
//! DPAPI (`CryptProtectData`) keys the ciphertext to the current **user account**,
//! so the file is unreadable by other users on the machine and unusable if copied
//! elsewhere. This is the same mechanism Windows itself uses for stored browser
//! credentials — the password never touches disk (or the environment) in clear text.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credentials {
    /// Workspace (tenant) name; empty for a host account.
    #[serde(default)]
    pub workspace: String,
    pub username: String,
    pub password: String,
    /// Project the user last chose, so tracking resumes where it left off.
    #[serde(default)]
    pub project_id: Option<String>,
}

fn credentials_path() -> Result<std::path::PathBuf> {
    let dir = dirs::config_dir()
        .ok_or_else(|| anyhow!("could not resolve the user config directory"))?
        .join("DosiTracker");
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join("credentials.bin"))
}

/// Load and decrypt the saved sign-in, if any. A corrupt/foreign blob is treated
/// as "no credentials" rather than a hard error, so the app falls back to login.
pub fn load() -> Option<Credentials> {
    let path = credentials_path().ok()?;
    let encrypted = std::fs::read(&path).ok()?;
    let plaintext = dpapi::unprotect(&encrypted).ok()?;
    serde_json::from_slice(&plaintext).ok()
}

/// Encrypt and persist the sign-in for this Windows user.
pub fn save(credentials: &Credentials) -> Result<()> {
    let path = credentials_path()?;
    let plaintext = serde_json::to_vec(credentials)?;
    let encrypted = dpapi::protect(&plaintext)?;
    std::fs::write(&path, encrypted)?;
    Ok(())
}

/// Forget the saved sign-in (sign out).
pub fn clear() -> Result<()> {
    let path = credentials_path()?;
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

#[cfg(windows)]
mod dpapi {
    use anyhow::{anyhow, Result};
    use windows::Win32::Foundation::LocalFree;
    use windows::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPT_INTEGER_BLOB,
    };

    /// Copy a DPAPI output blob into a Vec and release the OS allocation.
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

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn dpapi_roundtrips_credentials() {
        let secret = br#"{"username":"you@example.com","password":"s3cr3t"}"#;
        let encrypted = dpapi::protect(secret).expect("CryptProtectData should succeed");

        // The ciphertext must not contain the plaintext password.
        assert!(!encrypted.windows(6).any(|w| w == b"s3cr3t"));
        assert_ne!(encrypted.as_slice(), secret.as_slice());

        let decrypted = dpapi::unprotect(&encrypted).expect("CryptUnprotectData should succeed");
        assert_eq!(decrypted, secret);
    }

    #[test]
    fn dpapi_rejects_garbage() {
        assert!(dpapi::unprotect(b"not a dpapi blob").is_err());
    }

    #[test]
    fn credentials_serialize_with_optional_project() {
        let json = br#"{"username":"a","password":"b"}"#; // legacy blob: no workspace field
        let parsed: Credentials = serde_json::from_slice(json).expect("project_id is optional");
        assert_eq!(parsed.username, "a");
        assert!(parsed.project_id.is_none());
    }
}

// Non-Windows builds (CI lint, dev on other platforms) keep the API but do not
// pretend to encrypt — the agent only ships on Windows.
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
