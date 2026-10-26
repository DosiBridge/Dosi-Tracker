//! Persisted sign-in, encrypted at rest with Windows DPAPI.
//!
//! DPAPI (`CryptProtectData`) keys the ciphertext to the current **user
//! account**, so the file is unreadable by other users on the machine and
//! unusable if copied elsewhere, and this app's own entropy
//! ([`ENTROPY`]) means a generic sweep of the user's DPAPI blobs does not open
//! it either (see [`crate::dpapi`]).
//!
//! The password is **never** saved: only the refresh token the server granted
//! for it. A refused refresh token means "sign-in expired" and the user types
//! the password again (CONTRACT, agent sign-in). Sign-ins saved by older
//! builds carried the password; it is read once (to exchange it for a refresh
//! token) and the file is rewritten without it at start.
//!
//! Besides the token, the sign-in carries what must outlive a restart for
//! that account: the project last chosen, the project list last resolved (so
//! tracking can start offline), a pause, and the task note.

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
    /// Only read, from a sign-in saved by an older build that kept the
    /// password: it is used once to obtain a refresh token, and never written
    /// back (the field is not serialized).
    #[serde(default, rename = "password", skip_serializing)]
    pub legacy_password: String,
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
    /// Loaded from a file in an older format (no entropy, or with the
    /// password): it is saved again at once, in the current one.
    #[serde(skip)]
    pub outdated_format: bool,
}

impl Credentials {
    /// A fresh sign-in, with nothing remembered yet.
    pub fn new(username: String, server: Option<String>) -> Self {
        Self {
            workspace_id: None,
            workspace_name: String::new(),
            username,
            legacy_password: String::new(),
            refresh_token: None,
            project_id: None,
            server,
            projects: None,
            pause: None,
            task_note: None,
            outdated_format: false,
        }
    }

    /// The account, as the offline queue tags its rows: this user on
    /// `server` (the address in effect for this sign-in).
    pub fn owner_key(&self, server: &str) -> String {
        crate::storage::owner_key(server, &self.username)
    }

    /// Is this the same user (compared as the server does)? Whether it is
    /// the same *account* also depends on the server.
    pub fn same_user(&self, username: &str) -> bool {
        self.username.trim().to_lowercase() == username.trim().to_lowercase()
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

/// The application entropy the sign-in is encrypted with (no secret: it
/// keeps the blob from being opened by code that does not know it).
const ENTROPY: &[u8] = b"DosiTracker/credentials/v2";

/// Load and decrypt the saved sign-in, if any. A corrupt/foreign blob is treated
/// as "no credentials" rather than a hard error, so the app falls back to login.
/// Never writes: an outdated file is saved again by the worker.
pub fn load() -> Option<Credentials> {
    load_from(&default_path().ok()?)
}

/// [`load`] from `path`.
pub fn load_from(path: &Path) -> Option<Credentials> {
    let encrypted = std::fs::read(path).ok()?;
    let (plaintext, outdated) = match crate::dpapi::unprotect(&encrypted, Some(ENTROPY)) {
        Ok(plaintext) => (plaintext, false),
        // Saved by an older build, without the entropy.
        Err(_) => (crate::dpapi::unprotect(&encrypted, None).ok()?, true),
    };
    let mut credentials: Credentials = serde_json::from_slice(&plaintext).ok()?;
    credentials.outdated_format = outdated || !credentials.legacy_password.is_empty();
    Some(credentials)
}

/// Encrypt and persist the sign-in for this Windows user at `path` (never
/// with a password: [`Credentials::legacy_password`] is not serialized).
///
/// Write-then-rename: the sign-in is saved on every pause, project change,
/// project refresh and token rotation, and a crash halfway through a plain
/// write would leave a blob that no longer decrypts — signing the user out.
pub fn save_at(path: &Path, credentials: &Credentials) -> Result<()> {
    let plaintext = serde_json::to_vec(credentials)?;
    let encrypted = crate::dpapi::protect(&plaintext, Some(ENTROPY))?;
    crate::fsutil::write_atomically(path, &encrypted)
}

/// Forget the sign-in saved at `path` (sign out).
pub fn clear_at(path: &Path) -> Result<()> {
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    fn temp_path() -> PathBuf {
        std::env::temp_dir().join(format!("dosi-credentials-{}.bin", uuid::Uuid::new_v4()))
    }

    #[test]
    fn the_saved_sign_in_never_holds_a_password_and_needs_the_app_entropy() {
        let path = temp_path();
        let mut saved = Credentials::new("alice@acme.com".into(), None);
        saved.legacy_password = "s3cr3t-pw".into();
        saved.refresh_token = Some("r3fr3sh".into());
        save_at(&path, &saved).unwrap();

        let raw = std::fs::read(&path).unwrap();
        let plaintext = crate::dpapi::unprotect(&raw, Some(ENTROPY)).expect("encrypted with the app entropy");
        let json = String::from_utf8(plaintext).unwrap();
        assert!(!json.contains("s3cr3t-pw") && !json.contains("password"), "{json}");
        assert!(crate::dpapi::unprotect(&raw, None).is_err(), "a blind DPAPI sweep (no entropy) does not open it");
        let back = load_from(&path).unwrap();
        assert_eq!((back.legacy_password.as_str(), back.outdated_format), ("", false));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_sign_in_saved_by_an_older_build_is_read_once_and_marked_for_rewriting() {
        // No entropy, with the password, as builds before this one saved it.
        let path = temp_path();
        let old = br#"{"workspace_id":"w1","username":"a","password":"old-pw","refresh_token":null}"#;
        std::fs::write(&path, crate::dpapi::protect(old, None).unwrap()).unwrap();
        let loaded = load_from(&path).expect("still readable");
        assert_eq!(loaded.legacy_password, "old-pw", "kept in memory for its one-time exchange");
        assert!(loaded.outdated_format);
        // Read-only: loading never rewrites (the worker does, once).
        assert_eq!(crate::dpapi::unprotect(&std::fs::read(&path).unwrap(), None).unwrap(), old);

        // Without entropy but no password: still rewritten (to gain the entropy).
        let newer = br#"{"workspace_id":"w1","username":"a","refresh_token":"r"}"#;
        std::fs::write(&path, crate::dpapi::protect(newer, None).unwrap()).unwrap();
        assert!(load_from(&path).unwrap().outdated_format);

        // With the entropy, but still carrying a password field.
        std::fs::write(&path, crate::dpapi::protect(old, Some(ENTROPY)).unwrap()).unwrap();
        assert!(load_from(&path).unwrap().outdated_format);

        std::fs::write(&path, b"not a dpapi blob").unwrap();
        assert!(load_from(&path).is_none(), "a corrupt file means signed out");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn credentials_serialize_with_optional_project() {
        let json = br#"{"workspace":"acme","username":"a","password":"b"}"#; // saved by an older build
        let parsed: Credentials = serde_json::from_slice(json).expect("project_id is optional");
        assert_eq!((parsed.username.as_str(), parsed.legacy_password.as_str()), ("a", "b"));
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
        let path = temp_path();
        let at = DateTime::parse_from_rfc3339("2026-10-05T09:00:00Z").unwrap().with_timezone(&Utc);
        let mut saved = Credentials::new("alice@acme.com".into(), Some("https://tracker.acme.com".into()));
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

        // Saving again replaces the file.
        saved.pause = None;
        save_at(&path, &saved).unwrap();
        assert!(load_from(&path).unwrap().pause.is_none());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn the_project_list_is_usable_offline_for_seven_days() {
        let at = DateTime::parse_from_rfc3339("2026-10-05T09:00:00Z").unwrap().with_timezone(&Utc);
        let mut saved = Credentials::new("a".into(), None);
        assert!(saved.usable_projects(at).is_none(), "nothing saved yet");
        saved.projects = Some(ProjectCache { resolved_at: at, projects: vec![project("p1")] });
        assert_eq!(saved.usable_projects(at + TimeDelta::days(6)).map(<[Project]>::len), Some(1));
        assert!(saved.usable_projects(at + TimeDelta::days(7)).is_some());
        assert!(saved.usable_projects(at + TimeDelta::days(7) + TimeDelta::seconds(1)).is_none(), "too old");
        assert!(saved.usable_projects(at - TimeDelta::hours(1)).is_none(), "resolved in the future: the clock is off");
    }

    #[test]
    fn accounts_compare_like_the_server() {
        let saved = Credentials::new("Alice@Acme.com".into(), None);
        assert!(saved.same_user(" alice@acme.com "));
        assert!(!saved.same_user("bob@acme.com"));
        let server = "https://tracker.acme.com";
        assert_eq!(saved.owner_key(server), crate::storage::owner_key(server, "alice@acme.com"));
        assert_ne!(saved.owner_key(server), saved.owner_key("https://tracker.globex.com"), "the server is part of the account");
    }

    #[test]
    fn a_sign_in_saved_without_a_password_still_loads() {
        let json = br#"{"workspace_id":"w1","username":"a","refresh_token":"r"}"#;
        let parsed: Credentials = serde_json::from_slice(json).expect("the password is optional");
        assert_eq!(parsed.legacy_password, "");
        assert_eq!(parsed.refresh_token.as_deref(), Some("r"));
    }
}
