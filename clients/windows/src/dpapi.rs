//! Windows DPAPI (`CryptProtectData`), shared by the saved sign-in and the
//! device signing key.
//!
//! DPAPI keys the ciphertext to the current **user account**: other users of
//! the machine cannot read it, and a copy taken elsewhere does not decrypt.
//! Any program running as the same user can call `CryptUnprotectData`,
//! though, so callers pass an application-specific `entropy`: a blob is then
//! only opened by code that knows it (a generic "dump every DPAPI blob" tool
//! does not). It is no secret — this source is public — but it means a blob
//! of this app is not opened by a blind sweep, and a blob of another app is
//! never mistaken for one of ours.

use anyhow::Result;

/// Encrypt `plaintext` for the current Windows user, bound to `entropy`
/// (`None`: no entropy, as blobs saved by older builds were).
pub fn protect(plaintext: &[u8], entropy: Option<&[u8]>) -> Result<Vec<u8>> {
    imp::protect(plaintext, entropy)
}

/// Decrypt a blob [`protect`]ed with the same `entropy`.
pub fn unprotect(ciphertext: &[u8], entropy: Option<&[u8]>) -> Result<Vec<u8>> {
    imp::unprotect(ciphertext, entropy)
}

#[cfg(windows)]
mod imp {
    use anyhow::{anyhow, Result};
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    fn blob_of(bytes: &[u8]) -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB { cbData: bytes.len() as u32, pbData: bytes.as_ptr() as *mut u8 }
    }

    /// Copy a DPAPI output blob into a Vec and release the OS allocation.
    unsafe fn take_blob(blob: &CRYPT_INTEGER_BLOB) -> Vec<u8> {
        if blob.pbData.is_null() {
            return Vec::new();
        }
        let out = unsafe { std::slice::from_raw_parts(blob.pbData, blob.cbData as usize) }.to_vec();
        let _ = unsafe { LocalFree(HLOCAL(blob.pbData as *mut core::ffi::c_void)) };
        out
    }

    pub fn protect(plaintext: &[u8], entropy: Option<&[u8]>) -> Result<Vec<u8>> {
        let input = blob_of(plaintext);
        let entropy = entropy.map(blob_of);
        let mut output = CRYPT_INTEGER_BLOB::default();
        unsafe {
            CryptProtectData(
                &input,
                None,
                entropy.as_ref().map(|e| e as *const CRYPT_INTEGER_BLOB),
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
            .map_err(|e| anyhow!("CryptProtectData failed: {e}"))?;
            Ok(take_blob(&output))
        }
    }

    pub fn unprotect(ciphertext: &[u8], entropy: Option<&[u8]>) -> Result<Vec<u8>> {
        let input = blob_of(ciphertext);
        let entropy = entropy.map(blob_of);
        let mut output = CRYPT_INTEGER_BLOB::default();
        unsafe {
            CryptUnprotectData(
                &input,
                None,
                entropy.as_ref().map(|e| e as *const CRYPT_INTEGER_BLOB),
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
            .map_err(|e| anyhow!("CryptUnprotectData failed: {e}"))?;
            Ok(take_blob(&output))
        }
    }
}

// Non-Windows builds (CI lint, dev on other platforms) keep the API but do not
// pretend to encrypt — the agent only ships on Windows. The entropy is still
// checked, so the fallback paths of the callers behave the same.
#[cfg(not(windows))]
mod imp {
    use anyhow::{anyhow, Result};

    pub fn protect(plaintext: &[u8], entropy: Option<&[u8]>) -> Result<Vec<u8>> {
        let mut out = entropy.unwrap_or_default().to_vec();
        out.push(0);
        out.extend_from_slice(plaintext);
        Ok(out)
    }

    pub fn unprotect(ciphertext: &[u8], entropy: Option<&[u8]>) -> Result<Vec<u8>> {
        let entropy = entropy.unwrap_or_default();
        ciphertext
            .strip_prefix(entropy)
            .and_then(|rest| rest.strip_prefix(&[0]))
            .map(<[u8]>::to_vec)
            .ok_or_else(|| anyhow!("wrong entropy"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blob_round_trips_and_hides_its_plaintext() {
        let secret = br#"{"username":"you@example.com","refresh_token":"s3cr3t"}"#;
        let encrypted = protect(secret, Some(b"app")).expect("CryptProtectData should succeed");
        if cfg!(windows) {
            assert!(!encrypted.windows(6).any(|w| w == b"s3cr3t"), "the ciphertext must not contain the plaintext");
        }
        assert_eq!(unprotect(&encrypted, Some(b"app")).unwrap(), secret);
    }

    #[test]
    fn the_entropy_must_match() {
        let with = protect(b"payload", Some(b"app")).unwrap();
        assert!(unprotect(&with, None).is_err(), "no entropy");
        assert!(unprotect(&with, Some(b"other")).is_err(), "another app's entropy");
        let without = protect(b"payload", None).unwrap();
        assert!(unprotect(&without, Some(b"app")).is_err());
        assert_eq!(unprotect(&without, None).unwrap(), b"payload");
    }

    #[test]
    fn garbage_is_refused() {
        assert!(unprotect(b"not a dpapi blob", Some(b"app")).is_err());
    }
}
