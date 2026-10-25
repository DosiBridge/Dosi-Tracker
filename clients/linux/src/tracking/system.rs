//! Device context sent with every block and heartbeat (design 8.2, 8.3):
//! the device id, the computer name, the OS version, the IANA time zone and
//! UTC offset, the agent version and platform `linux`.
//!
//! - The device id is a random UUIDv4 kept in `$XDG_DATA_HOME/dosi-tracker/
//!   device.json` (0600), created once and never derived from
//!   `/etc/machine-id`, MAC addresses or DMI data (invariant I6). A damaged
//!   file is moved aside and a new id is created.
//! - `deviceName` is the `uname` node name, at most 128 characters.
//! - `osVersion` is `/etc/os-release` `PRETTY_NAME` (falling back to
//!   `/usr/lib/os-release`, then `NAME VERSION`, then `Linux`) plus
//!   ` (Linux <kernel release>)`, at most 64 characters. The root is
//!   injectable so tests read fixtures.

use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, Offset, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::agent::model::PLATFORM;

/// Longest `deviceName` the server accepts.
pub const MAX_DEVICE_NAME_CHARS: usize = 128;

/// Longest `osVersion` the server accepts.
pub const MAX_OS_VERSION_CHARS: usize = 64;

/// The agent's version (`agentVersion`).
pub const AGENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Contents of `device.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceFile {
    pub device_id: Uuid,
    pub created_at: DateTime<Utc>,
}

/// What [`load_or_create_device_id`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceId {
    pub id: Uuid,
    /// The file did not exist (or was damaged) and was created now.
    pub created: bool,
    /// Where a damaged file was moved.
    pub moved_aside: Option<PathBuf>,
}

/// The device id stored at `path`, creating it (0600, in a 0700 directory)
/// the first time. Two agents starting at once agree on one id: the file is
/// published with `link(2)`, which never replaces an existing file.
pub fn load_or_create_device_id(path: &Path) -> std::io::Result<DeviceId> {
    let mut moved_aside = None;
    match read_device_file(path)? {
        Some(Ok(file)) => {
            return Ok(DeviceId {
                id: file.device_id,
                created: false,
                moved_aside: None,
            })
        }
        Some(Err(())) => {
            let aside = crate::agent::session_store::damaged_path(path, Utc::now());
            std::fs::rename(path, &aside)?;
            tracing::warn!(moved_to = %aside.display(), "device.json was damaged; a new device id is created");
            moved_aside = Some(aside);
        }
        None => {}
    }

    let dir = path
        .parent()
        .ok_or_else(|| std::io::Error::other("device.json has no directory"))?;
    crate::paths::ensure_private_dir(dir)?;
    let file = DeviceFile {
        device_id: Uuid::new_v4(),
        created_at: Utc::now(),
    };
    let json = serde_json::to_vec_pretty(&file).map_err(std::io::Error::other)?;
    let temp = dir.join(format!(".device.json.tmp-{}", Uuid::new_v4().simple()));
    let published = (|| {
        use std::os::unix::fs::OpenOptionsExt;
        let mut out = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&temp)?;
        out.write_all(&json)?;
        out.sync_all()?;
        drop(out);
        std::fs::hard_link(&temp, path)
    })();
    let _ = std::fs::remove_file(&temp);
    match published {
        Ok(()) => {
            std::fs::File::open(dir)?.sync_all()?;
            Ok(DeviceId {
                id: file.device_id,
                created: true,
                moved_aside,
            })
        }
        // Another agent was faster: use its id.
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => match read_device_file(path)? {
            Some(Ok(theirs)) => Ok(DeviceId {
                id: theirs.device_id,
                created: false,
                moved_aside,
            }),
            _ => Err(std::io::Error::other(
                "device.json changed while it was created",
            )),
        },
        Err(e) => Err(e),
    }
}

/// `None`: missing. `Some(Err)`: present but not a valid device file.
/// A symlink or another user's file is an error (never read, never moved).
fn read_device_file(path: &Path) -> std::io::Result<Option<Result<DeviceFile, ()>>> {
    use std::io::Read;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let file = match std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let meta = file.metadata()?;
    // SAFETY: geteuid has no preconditions and cannot fail.
    if !meta.is_file() || meta.uid() != unsafe { libc::geteuid() } {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!(
                "refusing {}: not a regular file of this user",
                path.display()
            ),
        ));
    }
    let mut bytes = Vec::new();
    file.take(64 * 1024).read_to_end(&mut bytes)?;
    Ok(Some(
        serde_json::from_slice::<DeviceFile>(&bytes)
            .ok()
            .filter(|f| f.device_id.get_version_num() == 4)
            .ok_or(()),
    ))
}

/// The `uname(2)` node name and kernel release.
pub fn uname() -> Option<(String, String)> {
    // SAFETY: utsname is plain data; uname fills it and NUL-terminates each field.
    let mut uts: libc::utsname = unsafe { std::mem::zeroed() };
    if unsafe { libc::uname(&mut uts) } != 0 {
        return None;
    }
    let field = |raw: &[libc::c_char]| {
        let bytes: Vec<u8> = raw
            .iter()
            .take_while(|c| **c != 0)
            .map(|c| *c as u8)
            .collect();
        String::from_utf8_lossy(&bytes).into_owned()
    };
    Some((field(&uts.nodename), field(&uts.release)))
}

/// `deviceName`: trimmed and cut to [`MAX_DEVICE_NAME_CHARS`]; `None` when blank.
pub fn device_name_from(node_name: &str) -> Option<String> {
    let name: String = node_name
        .trim()
        .chars()
        .take(MAX_DEVICE_NAME_CHARS)
        .collect();
    let name = name.trim_end();
    (!name.is_empty()).then(|| name.to_owned())
}

/// Parse an os-release file (shell-like `KEY=value` lines, values optionally
/// quoted with `"` or `'`, backslash escapes inside double quotes).
pub fn parse_os_release(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let (key, raw) = line.split_once('=')?;
            let key = key.trim();
            if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return None;
            }
            Some((key.to_owned(), unquote(raw.trim())))
        })
        .collect()
}

fn unquote(raw: &str) -> String {
    let mut chars = raw.chars().peekable();
    match chars.peek() {
        Some('"') => {
            chars.next();
            let mut out = String::new();
            while let Some(c) = chars.next() {
                match c {
                    '"' => break,
                    '\\' => {
                        if let Some(next) = chars.next() {
                            if !matches!(next, '$' | '"' | '\\' | '`') {
                                out.push('\\');
                            }
                            out.push(next);
                        }
                    }
                    other => out.push(other),
                }
            }
            out
        }
        Some('\'') => {
            chars.next();
            chars.take_while(|c| *c != '\'').collect()
        }
        _ => raw.to_owned(),
    }
}

/// The distribution's name from the os-release files under `root`.
pub fn distribution_name(root: &Path) -> Option<String> {
    ["etc/os-release", "usr/lib/os-release"]
        .iter()
        .find_map(|rel| std::fs::read_to_string(root.join(rel)).ok())
        .and_then(|text| {
            let fields = parse_os_release(&text);
            let get = |key: &str| {
                fields
                    .iter()
                    .find(|(k, _)| k == key)
                    .map(|(_, v)| v.trim().to_owned())
                    .filter(|v| !v.is_empty())
            };
            get("PRETTY_NAME").or_else(|| match (get("NAME"), get("VERSION")) {
                (Some(name), Some(version)) => Some(format!("{name} {version}")),
                (Some(name), None) => Some(name),
                _ => None,
            })
        })
}

/// `osVersion`: `<PRETTY_NAME> (Linux <kernel release>)`, at most
/// [`MAX_OS_VERSION_CHARS`] characters. When too long, the distribution
/// name is shortened first so the kernel release stays whole.
pub fn os_version_from(root: &Path, kernel_release: Option<&str>) -> String {
    let distro = distribution_name(root).unwrap_or_else(|| "Linux".to_owned());
    let cut = |text: &str, max: usize| -> String {
        let cut: String = text.chars().take(max).collect();
        cut.trim_end().to_owned()
    };
    let Some(kernel) = kernel_release.map(str::trim).filter(|k| !k.is_empty()) else {
        return cut(&distro, MAX_OS_VERSION_CHARS);
    };
    let suffix = format!(" (Linux {kernel})");
    let budget = MAX_OS_VERSION_CHARS.saturating_sub(suffix.chars().count());
    if distro.chars().count() <= budget {
        format!("{distro}{suffix}")
    } else if budget >= 8 {
        format!("{}{suffix}", cut(&distro, budget))
    } else {
        cut(&format!("{distro}{suffix}"), MAX_OS_VERSION_CHARS)
    }
}

/// Minutes east of UTC in the local time zone at `at`, clamped to ±840.
pub fn utc_offset_minutes(at: DateTime<Utc>) -> i32 {
    let offset = Local.offset_from_utc_datetime(&at.naive_utc()).fix();
    (offset.local_minus_utc() / 60).clamp(-840, 840)
}

/// The IANA name of the local time zone (`Europe/Berlin`), if known.
pub fn time_zone() -> Option<String> {
    iana_time_zone::get_timezone()
        .ok()
        .map(|tz| tz.trim().to_owned())
        .filter(|tz| !tz.is_empty())
}

/// Everything about this device the agent reports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeviceInfo {
    pub device_id: String,
    pub device_name: Option<String>,
    pub os_version: String,
    pub agent_version: String,
    pub platform: String,
    pub time_zone: Option<String>,
}

impl DeviceInfo {
    /// Read the device file (creating it once), `uname` and os-release under `/`.
    pub fn collect(device_file: &Path) -> std::io::Result<Self> {
        Self::collect_with(device_file, Path::new("/"), uname())
    }

    /// [`Self::collect`] with an injected root and `uname` answer (tests).
    pub fn collect_with(
        device_file: &Path,
        root: &Path,
        uname: Option<(String, String)>,
    ) -> std::io::Result<Self> {
        let id = load_or_create_device_id(device_file)?;
        let (node, release) = match &uname {
            Some((node, release)) => (Some(node.as_str()), Some(release.as_str())),
            None => (None, None),
        };
        Ok(Self {
            device_id: id.id.to_string(),
            device_name: node.and_then(device_name_from),
            os_version: os_version_from(root, release),
            agent_version: AGENT_VERSION.to_owned(),
            platform: PLATFORM.to_owned(),
            time_zone: time_zone(),
        })
    }

    /// The UTC offset to send with a block starting (or a heartbeat sent) at `at`.
    pub fn utc_offset_minutes(&self, at: DateTime<Utc>) -> i32 {
        utc_offset_minutes(at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn os_release_values_are_unquoted_like_a_shell_would() {
        let fields = parse_os_release(
            "# comment\nNAME=\"Fedora Linux\"\nVERSION_ID=43\nPRETTY_NAME='Single quoted'\n\
             ESCAPED=\"a \\\"b\\\" \\$c \\\\ d\"\nBAD KEY=x\n=novalue\n",
        );
        let get = |k: &str| {
            fields
                .iter()
                .find(|(key, _)| key == k)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(get("NAME"), Some("Fedora Linux"));
        assert_eq!(get("VERSION_ID"), Some("43"));
        assert_eq!(get("PRETTY_NAME"), Some("Single quoted"));
        assert_eq!(get("ESCAPED"), Some("a \"b\" $c \\ d"));
        assert_eq!(get("BAD KEY"), None);
        assert_eq!(fields.len(), 4);
    }

    #[test]
    fn device_names_are_trimmed_and_bounded() {
        assert_eq!(
            device_name_from("  work-laptop \n").as_deref(),
            Some("work-laptop")
        );
        assert_eq!(device_name_from("   "), None);
        assert_eq!(
            device_name_from(&"n".repeat(300)).unwrap().chars().count(),
            MAX_DEVICE_NAME_CHARS
        );
    }

    #[test]
    fn the_utc_offset_is_whole_minutes_within_the_server_range() {
        let offset = utc_offset_minutes(Utc::now());
        assert!((-840..=840).contains(&offset));
    }

    #[test]
    fn uname_answers_on_linux() {
        let (node, release) = uname().expect("uname works");
        assert!(!release.is_empty());
        let _ = node;
    }
}
