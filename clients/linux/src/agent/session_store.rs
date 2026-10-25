//! `session.json` (design 9.1): what must outlive a restart for the signed-in
//! account and is not a secret — server, account, workspace, the project last
//! chosen, the project list last resolved (so tracking can start offline and
//! go on while the keyring is locked), a pause and the task note.
//!
//! Ported from the non-secret part of the Windows `Credentials`, with its
//! 7-day [`ProjectCache`] rule (and at most 10 minutes "in the future").
//! Secrets live in the Secret Service ([`crate::credentials`]), never here.
//!
//! The file is written atomically as 0600 in a 0700 directory
//! ([`crate::paths::write_private_atomic`]). A file that cannot be read as a
//! format-1 session is moved aside to `session.json.damaged-<utc>` and a
//! fresh session starts; a symlink or another user's file is refused.

use std::path::{Path, PathBuf};

use chrono::{DateTime, TimeDelta, Utc};
use serde::{Deserialize, Serialize};

use super::model::Project;

/// The only `format_version` this build reads and writes.
pub const FORMAT_VERSION: u32 = 1;

/// How long the saved project list may be used to track without reaching
/// the server (an offline start, switching project offline, or a locked
/// keyring).
pub const PROJECT_CACHE_MAX_AGE: TimeDelta = TimeDelta::days(7);

/// A saved list newer than this (the device clock went backwards) is not
/// trusted either.
pub const PROJECT_CACHE_MAX_FUTURE: TimeDelta = TimeDelta::minutes(10);

/// Longest task note kept (the server's `description` limit).
pub const MAX_TASK_NOTE_CHARS: usize = 200;

/// The project list as last resolved by the server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
pub struct SavedPause {
    pub since: DateTime<Utc>,
    #[serde(default)]
    pub until: Option<DateTime<Utc>>,
}

/// The workspace tracked in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedWorkspace {
    pub id: String,
    #[serde(default)]
    pub name: String,
}

/// Contents of `session.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionData {
    pub format_version: u32,
    /// The server address used at sign-in (normalized).
    #[serde(default)]
    pub server: Option<String>,
    /// The account (email) signed in.
    #[serde(default)]
    pub account: Option<String>,
    #[serde(default)]
    pub workspace: Option<SavedWorkspace>,
    /// Project the user last chose, so tracking resumes where it left off.
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub projects: Option<ProjectCache>,
    /// Tracking was paused when the agent last ran: it starts paused again.
    #[serde(default)]
    pub pause: Option<SavedPause>,
    /// "What are you working on?"
    #[serde(default)]
    pub task_note: Option<String>,
}

impl Default for SessionData {
    fn default() -> Self {
        Self {
            format_version: FORMAT_VERSION,
            server: None,
            account: None,
            workspace: None,
            project_id: None,
            projects: None,
            pause: None,
            task_note: None,
        }
    }
}

impl SessionData {
    /// The saved project list, if it may still be used at `now`.
    pub fn usable_projects(&self, now: DateTime<Utc>) -> Option<&[Project]> {
        self.projects
            .as_ref()
            .filter(|cache| cache.usable_at(now))
            .map(|cache| cache.projects.as_slice())
    }

    /// Set the task note: trimmed, at most [`MAX_TASK_NOTE_CHARS`]
    /// characters, and `None` when blank.
    pub fn set_task_note(&mut self, note: Option<&str>) {
        self.task_note = note.and_then(normalize_task_note);
    }

    /// Forget everything about the account (sign out). The server is kept so
    /// the next sign-in is prefilled.
    pub fn signed_out(&self) -> Self {
        Self {
            server: self.server.clone(),
            ..Self::default()
        }
    }
}

/// A task note as stored and sent: trimmed, at most 200 characters, `None`
/// when blank.
pub fn normalize_task_note(note: &str) -> Option<String> {
    let cut: String = note.trim().chars().take(MAX_TASK_NOTE_CHARS).collect();
    let cut = cut.trim_end();
    (!cut.is_empty()).then(|| cut.to_owned())
}

/// The result of [`load`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded {
    pub data: SessionData,
    /// Where a damaged file was moved before starting fresh.
    pub moved_aside: Option<PathBuf>,
}

/// Why the session file could not be used.
#[derive(Debug)]
pub enum SessionStoreError {
    /// A symlink, not a regular file, or another user's file.
    Refused(String),
    Io(std::io::Error),
}

impl std::fmt::Display for SessionStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionStoreError::Refused(why) => write!(f, "refusing the session file: {why}"),
            SessionStoreError::Io(e) => write!(f, "session file: {e}"),
        }
    }
}

impl std::error::Error for SessionStoreError {}

impl From<std::io::Error> for SessionStoreError {
    fn from(e: std::io::Error) -> Self {
        SessionStoreError::Io(e)
    }
}

/// Read `path`. Missing -> a fresh session. Unparseable or of another format
/// version -> moved aside to `<name>.damaged-<utc>`, then a fresh session.
#[cfg(unix)]
pub fn load(path: &Path, now: DateTime<Utc>) -> Result<Loaded, SessionStoreError> {
    let bytes = match read_own_file(path)? {
        Some(bytes) => bytes,
        None => {
            return Ok(Loaded {
                data: SessionData::default(),
                moved_aside: None,
            })
        }
    };
    let parsed = serde_json::from_slice::<SessionData>(&bytes)
        .ok()
        .filter(|data| data.format_version == FORMAT_VERSION);
    match parsed {
        Some(data) => Ok(Loaded {
            data,
            moved_aside: None,
        }),
        None => {
            let aside = damaged_path(path, now);
            std::fs::rename(path, &aside)?;
            tracing::warn!(moved_to = %aside.display(), "the session file was damaged; starting fresh");
            Ok(Loaded {
                data: SessionData::default(),
                moved_aside: Some(aside),
            })
        }
    }
}

/// Write `data` to `path` (0600, atomic; the directory is created 0700).
#[cfg(unix)]
pub fn save(path: &Path, data: &SessionData) -> Result<(), SessionStoreError> {
    let dir = path
        .parent()
        .ok_or_else(|| SessionStoreError::Refused("no parent directory".into()))?;
    crate::paths::ensure_private_dir(dir)?;
    let mut data = data.clone();
    data.format_version = FORMAT_VERSION;
    let json = serde_json::to_vec_pretty(&data).map_err(std::io::Error::other)?;
    crate::paths::write_private_atomic(path, &json)?;
    Ok(())
}

/// Remove `path` (reset). Missing is fine.
#[cfg(unix)]
pub fn clear(path: &Path) -> Result<(), SessionStoreError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// `session.json.damaged-20261005T090000Z`.
pub fn damaged_path(path: &Path, now: DateTime<Utc>) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_else(|| "session.json".into());
    name.push(format!(".damaged-{}", now.format("%Y%m%dT%H%M%SZ")));
    path.with_file_name(name)
}

/// The bytes of `path` if it is a regular file of ours; `None` if missing.
#[cfg(unix)]
fn read_own_file(path: &Path) -> Result<Option<Vec<u8>>, SessionStoreError> {
    use std::io::Read;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

    let file = match std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) if e.raw_os_error() == Some(libc::ELOOP) => {
            return Err(SessionStoreError::Refused(format!(
                "{} is a symlink",
                path.display()
            )))
        }
        Err(e) => return Err(e.into()),
    };
    let meta = file.metadata()?;
    if !meta.is_file() {
        return Err(SessionStoreError::Refused(format!(
            "{} is not a regular file",
            path.display()
        )));
    }
    // SAFETY: geteuid has no preconditions and cannot fail.
    if meta.uid() != unsafe { libc::geteuid() } {
        return Err(SessionStoreError::Refused(format!(
            "{} is owned by another user",
            path.display()
        )));
    }
    let mut bytes = Vec::new();
    // A session file is small; anything huge is not ours to parse.
    file.take(4 * 1024 * 1024).read_to_end(&mut bytes)?;
    Ok(Some(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
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
    fn the_project_list_is_usable_offline_for_seven_days() {
        let at = at("2026-10-05T09:00:00Z");
        let mut saved = SessionData::default();
        assert!(saved.usable_projects(at).is_none(), "nothing saved yet");
        saved.projects = Some(ProjectCache {
            resolved_at: at,
            projects: vec![project("p1")],
        });
        assert_eq!(
            saved
                .usable_projects(at + TimeDelta::days(6))
                .map(<[Project]>::len),
            Some(1)
        );
        assert!(saved.usable_projects(at + TimeDelta::days(7)).is_some());
        assert!(
            saved
                .usable_projects(at + TimeDelta::days(7) + TimeDelta::seconds(1))
                .is_none(),
            "too old"
        );
        assert!(
            saved.usable_projects(at - TimeDelta::minutes(10)).is_some(),
            "a clock a little behind is tolerated"
        );
        assert!(
            saved.usable_projects(at - TimeDelta::hours(1)).is_none(),
            "resolved in the future: the clock is off"
        );
    }

    #[test]
    fn task_notes_are_trimmed_and_bounded() {
        let mut data = SessionData::default();
        data.set_task_note(Some("  Invoice export  "));
        assert_eq!(data.task_note.as_deref(), Some("Invoice export"));
        data.set_task_note(Some("   "));
        assert_eq!(data.task_note, None);
        data.set_task_note(Some(&"é".repeat(300)));
        assert_eq!(
            data.task_note.as_ref().unwrap().chars().count(),
            MAX_TASK_NOTE_CHARS
        );
        data.set_task_note(None);
        assert_eq!(data.task_note, None);
    }

    #[test]
    fn signing_out_keeps_only_the_server() {
        let data = SessionData {
            server: Some("https://tracker.example.com".into()),
            account: Some("a@example.com".into()),
            project_id: Some("p1".into()),
            ..SessionData::default()
        };
        let out = data.signed_out();
        assert_eq!(out.server, data.server);
        assert_eq!((out.account, out.project_id), (None, None));
    }

    #[test]
    fn the_damaged_name_carries_the_time() {
        let path = Path::new("/s/session.json");
        assert_eq!(
            damaged_path(path, at("2026-10-05T09:00:00Z")),
            PathBuf::from("/s/session.json.damaged-20261005T090000Z")
        );
    }
}
