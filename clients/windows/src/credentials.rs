//! Persisted sign-in, encrypted at rest with Windows DPAPI.
//!
//! DPAPI (`CryptProtectData`) keys the ciphertext to the current **user account**,
//! so the file is unreadable by other users on the machine and unusable if copied
//! elsewhere. This is the same mechanism Windows itself uses for stored browser
//! credentials — the password never touches disk (or the environment) in clear text.
//!
//! Besides the credentials, the sign-in carries what must outlive a restart
//! for that account: the project last chosen, the project list last resolved
//! (so tracking can start offline), a pause, and the task note.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use chrono::{DateTime, TimeDelta, Utc};
use serde::{Deserialize, Serialize};

use crate::model::Project;

/// How long the saved project list may be used to track without reaching
/// the server (an offline start, or switching project offline).
pub const PROJECT_CACHE_MAX_AGE: TimeDelta = TimeDelta::days(7);

/// A saved list newer than this (the device clock went backwards) is not
/// trusted either.
const PROJECT_CACHE_MAX_FUTURE: TimeDelta = TimeDelta::minutes(10);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credentials {
    /// The workspace tracked in (`X-Workspace-Id`). A sign-in saved by an
    /// older build has none and must sign in again.
    #[serde(default)]
    pub workspace_id: Option<String>,
    /// Its name, for display.
    #[serde(default)]
    pub workspace_name: String,
    /// The email (or user name) typed at sign-in.
    pub username: String,
    pub password: String,
    /// Renews the access token without the password.
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// Project the user last chose, so tracking resumes where it left off.
    #[serde(default)]
    pub project_id: Option<String>,
    /// The server address typed at sign-in (normalized); `None` means the
    /// configured one.
    #[serde(default)]
    pub server: Option<String>,
    /// The projects (with their capture policy and interval) the server last
    /// listed for this account.
    #[serde(default)]
    pub projects: Option<ProjectCache>,
    /// Tracking was paused when the app last ran: it starts paused again.
    #[serde(default)]
    pub pause: Option<SavedPause>,
    /// "What are you working on?"
    #[serde(default)]
    pub task_note: Option<String>,
}

impl Credentials {
    /// A fresh sign-in, with nothing remembered yet.
    pub fn new(username: String, password: String, server: Option<String>) -> Self {
        Self {
            workspace_id: None,
            workspace_name: String::new(),
            username,
            password,
            refresh_token: None,
            project_id: None,
            server,
            projects: None,
            pause: None,
            task_note: None,
        }
    }

    /// The user, as the offline queue tags its rows.
    pub fn owner_key(&self) -> String {
        crate::storage::owner_key(&self.username)
    }

    /// Is this the same user (compared as the server does)?
    pub fn same_account(&self, username: &str) -> bool {
        self.owner_key() == crate::storage::owner_key(username)
    }

    /// The saved project list, if it may still be used at `now`.
    pub fn usable_projects(&self, now: DateTime<Utc>) -> Option<&[Project]> {
        self.projects
            .as_ref()
            .filter(|cache| cache.usable_at(now))
            .map(|cache| cache.projects.as_slice())
    }
}

/// The project list as last resolved by the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectCache {
    pub resolved_at: DateTime<Utc>,
    pub projects: Vec<Project>,
}

impl ProjectCache {
    /// Usable for at most [`PROJECT_CACHE_MAX_AGE`] after it was resolved.
    pub fn usable_at(&self, now: DateTime<Utc>) -> bool {
        let age = now - self.resolved_at;
        age <= PROJECT_CACHE_MAX_AGE && age >= -PROJECT_CACHE_MAX_FUTURE
    }
}

/// A pause as saved: since when, and when a timed break ends by itself
/// (`None`: until the user resumes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedPause {
    pub since: DateTime<Utc>,
    #[serde(default)]
    pub until: Option<DateTime<Utc>>,
}

/// Where the sign-in is kept: `%APPDATA%\DosiTracker\credentials.bin`.
pub fn default_path() -> Result<PathBuf> {
    let dir = dirs::config_dir()
        .ok_or_else(|| anyhow!("could not resolve the user config directory"))?
        .join("DosiTracker");
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join("credentials.bin"))
}

/// Load and decrypt the saved sign-in, if any. A corrupt/foreign blob is treated
/// as "no credentials" rather than a hard error, so the app falls back to login.
pub fn load() -> Option<Credentials> {
    load_from(&default_path().ok()?)
}

/// [`load`] from `path`.
pub fn load_from(path: &Path) -> Option<Credentials> {
    let encrypted = std::fs::read(path).ok()?;
    let plaintext = dpapi::unprotect(&encrypted).ok()?;
    serde_json::from_slice(&plaintext).ok()
}

/// Encrypt and persist the sign-in for this Windows user at `path`.
///
/// Write-then-rename: the sign-in is saved on every pause, project change
/// and project refresh, and a crash halfway through a plain write would
/// leave a blob that no longer decrypts — signing the user out.
pub fn save_at(path: &Path, credentials: &Credentials) -> Result<()> {
    let plaintext = serde_json::to_vec(credentials)?;
    let encrypted = dpapi::protect(&plaintext)?;
    let mut temp = path.as_os_str().to_os_string();
    temp.push(".tmp");
    let temp = PathBuf::from(temp);
    std::fs::write(&temp, encrypted)?;
    std::fs::rename(&temp, path)?;
    Ok(())
}

/// Forget the sign-in saved at `path` (sign out).
pub fn clear_at(path: &Path) -> Result<()> {
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
        let json = br#"{"workspace":"acme","username":"a","password":"b"}"#; // saved by an older build
        let parsed: Credentials = serde_json::from_slice(json).expect("project_id is optional");
        assert_eq!(parsed.username, "a");
        assert!(parsed.workspace_id.is_none() && parsed.refresh_token.is_none(), "must sign in again");
        assert!(parsed.project_id.is_none());
        // Nothing the newer builds remember is required either.
        assert!(parsed.server.is_none() && parsed.projects.is_none() && parsed.pause.is_none() && parsed.task_note.is_none());
    }

    fn project(id: &str) -> Project {
        serde_json::from_value(serde_json::json!({
            "id": id, "title": format!("Project {id}"), "intervalMinutes": 10,
            "allowScreenshot": true, "allowWebcam": false, "allowKeyboard": true, "allowMouse": true,
            "allowActiveWindow": true, "allowRunningPrograms": true, "idleThresholdMinutes": 15
        }))
        .unwrap()
    }

    #[test]
    fn everything_remembered_survives_an_encrypted_round_trip() {
        let path = std::env::temp_dir().join(format!("dosi-credentials-{}.bin", uuid::Uuid::new_v4()));
        let at = DateTime::parse_from_rfc3339("2026-10-05T09:00:00Z").unwrap().with_timezone(&Utc);
        let mut saved = Credentials::new("alice@acme.com".into(), "pw".into(), Some("https://tracker.acme.com".into()));
        saved.workspace_id = Some("w1".into());
        saved.workspace_name = "Acme".into();
        saved.refresh_token = Some("r3fr3sh".into());
        saved.project_id = Some("p2".into());
        saved.projects = Some(ProjectCache { resolved_at: at, projects: vec![project("p1"), project("p2")] });
        saved.pause = Some(SavedPause { since: at, until: Some(at + TimeDelta::minutes(15)) });
        saved.task_note = Some("Invoice export".into());
        save_at(&path, &saved).unwrap();

        let raw = std::fs::read(&path).unwrap();
        assert!(!raw.windows(7).any(|w| w == b"r3fr3sh"), "the refresh token is encrypted too");
        let back = load_from(&path).expect("decrypts");
        assert_eq!((back.workspace_id.as_deref(), back.workspace_name.as_str()), (Some("w1"), "Acme"));
        assert_eq!(back.refresh_token.as_deref(), Some("r3fr3sh"));
        assert_eq!(back.server.as_deref(), Some("https://tracker.acme.com"));
        assert_eq!(back.project_id.as_deref(), Some("p2"));
        let cache = back.projects.as_ref().unwrap();
        assert_eq!(cache.resolved_at, at);
        assert_eq!(cache.projects[1].idle_threshold_minutes, 15);
        assert_eq!(back.pause, saved.pause);
        assert_eq!(back.task_note.as_deref(), Some("Invoice export"));
        assert!(!path.with_extension("bin.tmp").exists(), "the temporary file was renamed into place");

        // Saving again replaces the file.
        saved.pause = None;
        save_at(&path, &saved).unwrap();
        assert!(load_from(&path).unwrap().pause.is_none());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn the_project_list_is_usable_offline_for_seven_days() {
        let at = DateTime::parse_from_rfc3339("2026-10-05T09:00:00Z").unwrap().with_timezone(&Utc);
        let mut saved = Credentials::new("a".into(), "b".into(), None);
        assert!(saved.usable_projects(at).is_none(), "nothing saved yet");
        saved.projects = Some(ProjectCache { resolved_at: at, projects: vec![project("p1")] });
        assert_eq!(saved.usable_projects(at + TimeDelta::days(6)).map(<[Project]>::len), Some(1));
        assert!(saved.usable_projects(at + TimeDelta::days(7)).is_some());
        assert!(saved.usable_projects(at + TimeDelta::days(7) + TimeDelta::seconds(1)).is_none(), "too old");
        assert!(saved.usable_projects(at - TimeDelta::hours(1)).is_none(), "resolved in the future: the clock is off");
    }

    #[test]
    fn accounts_compare_like_the_server() {
        let saved = Credentials::new("Alice@Acme.com".into(), "pw".into(), None);
        assert!(saved.same_account(" alice@acme.com "));
        assert!(!saved.same_account("bob@acme.com"));
        assert_eq!(saved.owner_key(), crate::storage::owner_key("alice@acme.com"));
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
