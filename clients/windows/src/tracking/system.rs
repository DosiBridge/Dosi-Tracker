//! What this device is: a stable install id, its name, the OS and agent
//! versions, and the local time zone — sent with every block and heartbeat
//! so reports can tell devices apart and group days by the worker's own
//! calendar.

use std::path::Path;

use chrono::{DateTime, Local, Offset, TimeZone, Utc};
use serde::{Deserialize, Serialize};

/// `platform` on the wire.
pub const PLATFORM: &str = "windows";

/// This agent's version (`agentVersion`).
pub const AGENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Where the install id is kept, under the machine-local data folder.
const DEVICE_FILE: &str = "device.json";

/// The parts of the device context that do not change while the agent runs,
/// collected once. The time zone is read per block instead: it follows the
/// user when they travel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    /// Random UUID generated once per install; `None` when it cannot be
    /// stored (a new one each run would split one device into many).
    pub device_id: Option<String>,
    pub device_name: Option<String>,
    pub os_version: Option<String>,
    pub platform: &'static str,
    pub agent_version: &'static str,
}

impl DeviceInfo {
    /// Read the install id (creating it on first run, in
    /// `%LOCALAPPDATA%\DosiTracker\device.json`), the computer name and the
    /// OS version.
    pub fn collect() -> Self {
        Self::collect_in(crate::shell::local_data_dir().as_deref())
    }

    /// [`DeviceInfo::collect`] with the install id kept in `data_dir`.
    pub fn collect_in(data_dir: Option<&Path>) -> Self {
        let device_id = data_dir.and_then(|dir| match load_or_create_device_id(&dir.join(DEVICE_FILE)) {
            Ok(id) => Some(id),
            Err(e) => {
                tracing::warn!(?e, "could not read or store the device id");
                None
            }
        });
        Self {
            device_id,
            device_name: device_name(),
            os_version: os_version(),
            platform: PLATFORM,
            agent_version: AGENT_VERSION,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeviceFile {
    device_id: String,
}

/// The install id stored at `path`, or a new random one saved there when
/// the file is missing or unreadable.
pub fn load_or_create_device_id(path: &Path) -> std::io::Result<String> {
    let existing = std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<DeviceFile>(&bytes).ok())
        .and_then(|file| uuid::Uuid::parse_str(file.device_id.trim()).ok());
    if let Some(id) = existing {
        return Ok(id.to_string());
    }
    let id = uuid::Uuid::new_v4().to_string();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Write-then-rename, so a crash never leaves a half-written id behind.
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, serde_json::to_vec(&DeviceFile { device_id: id.clone() })?)?;
    std::fs::rename(&temp, path)?;
    Ok(id)
}

/// The computer's name (its DNS host name, as Settings shows it).
#[cfg(windows)]
pub fn device_name() -> Option<String> {
    use windows::core::PWSTR;
    use windows::Win32::System::SystemInformation::{ComputerNamePhysicalDnsHostname, GetComputerNameExW};

    let mut size: u32 = 0;
    // The first call only reports the size needed.
    let _ = unsafe { GetComputerNameExW(ComputerNamePhysicalDnsHostname, PWSTR::null(), &mut size) };
    if size == 0 {
        return std::env::var("COMPUTERNAME").ok().filter(|name| !name.is_empty());
    }
    let mut buffer = vec![0u16; size as usize];
    unsafe { GetComputerNameExW(ComputerNamePhysicalDnsHostname, PWSTR(buffer.as_mut_ptr()), &mut size) }.ok()?;
    let name = String::from_utf16_lossy(&buffer[..size as usize]);
    (!name.trim().is_empty()).then(|| name.trim().to_string())
}

#[cfg(not(windows))]
pub fn device_name() -> Option<String> {
    None
}

/// e.g. `"Windows 11 Pro 23H2 (10.0.22631)"`, from the registry.
#[cfg(windows)]
pub fn os_version() -> Option<String> {
    let product = registry::string("ProductName")?;
    let display = registry::string("DisplayVersion").or_else(|| registry::string("ReleaseId"));
    let major = registry::dword("CurrentMajorVersionNumber").unwrap_or(10);
    let minor = registry::dword("CurrentMinorVersionNumber").unwrap_or(0);
    let build = registry::string("CurrentBuildNumber")
        .or_else(|| registry::string("CurrentBuild"))
        .and_then(|b| b.trim().parse().ok())?;
    Some(format_os_version(&product, display.as_deref(), major, minor, build))
}

#[cfg(not(windows))]
pub fn os_version() -> Option<String> {
    None
}

/// Format the OS version. Windows 11 still names itself "Windows 10" in the
/// registry's `ProductName`; its build number (22000 and up) tells.
pub fn format_os_version(product: &str, display_version: Option<&str>, major: u32, minor: u32, build: u32) -> String {
    let product = product.trim();
    let product = if build >= 22_000 && product.starts_with("Windows 10") {
        format!("Windows 11{}", &product["Windows 10".len()..])
    } else {
        product.to_string()
    };
    let text = match display_version.map(str::trim).filter(|v| !v.is_empty()) {
        Some(display) => format!("{product} {display} ({major}.{minor}.{build})"),
        None => format!("{product} ({major}.{minor}.{build})"),
    };
    // The server keeps 64 characters.
    text.chars().take(64).collect()
}

/// The device's IANA time zone (e.g. `"Asia/Dhaka"`), if Windows can name it.
pub fn time_zone() -> Option<String> {
    iana_time_zone::get_timezone()
        .ok()
        .map(|zone| zone.trim().to_string())
        .filter(|zone| !zone.is_empty() && zone.len() <= 64)
}

/// The device's UTC offset at `at`, in minutes (+360 for Dhaka; it follows
/// daylight saving time).
pub fn utc_offset_minutes(at: DateTime<Utc>) -> i32 {
    utc_offset_minutes_in(at, &Local)
}

/// [`utc_offset_minutes`] in a given zone.
pub fn utc_offset_minutes_in<Tz: TimeZone>(at: DateTime<Utc>, zone: &Tz) -> i32 {
    zone.offset_from_utc_datetime(&at.naive_utc()).fix().local_minus_utc() / 60
}

#[cfg(windows)]
mod registry {
    use windows::core::{w, HSTRING};
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ};

    const CURRENT_VERSION: windows::core::PCWSTR = w!(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion");

    /// A `REG_SZ` value of `HKLM\…\Windows NT\CurrentVersion`.
    pub fn string(name: &str) -> Option<String> {
        let mut buffer = [0u16; 256];
        let mut size = std::mem::size_of_val(&buffer) as u32;
        let status = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                CURRENT_VERSION,
                &HSTRING::from(name),
                RRF_RT_REG_SZ,
                None,
                Some(buffer.as_mut_ptr() as *mut core::ffi::c_void),
                Some(&mut size),
            )
        };
        if status != ERROR_SUCCESS {
            return None;
        }
        let chars = (size as usize / 2).min(buffer.len());
        let value = String::from_utf16_lossy(&buffer[..chars]);
        let value = value.trim_end_matches('\0').trim().to_string();
        (!value.is_empty()).then_some(value)
    }

    /// A `REG_DWORD` value of `HKLM\…\Windows NT\CurrentVersion`.
    pub fn dword(name: &str) -> Option<u32> {
        let mut value: u32 = 0;
        let mut size = std::mem::size_of::<u32>() as u32;
        let status = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                CURRENT_VERSION,
                &HSTRING::from(name),
                RRF_RT_REG_DWORD,
                None,
                Some(&mut value as *mut u32 as *mut core::ffi::c_void),
                Some(&mut size),
            )
        };
        (status == ERROR_SUCCESS).then_some(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    #[test]
    fn windows_11_is_named_from_its_build_number() {
        assert_eq!(
            format_os_version("Windows 10 Pro", Some("23H2"), 10, 0, 22631),
            "Windows 11 Pro 23H2 (10.0.22631)"
        );
        assert_eq!(
            format_os_version("Windows 10 Enterprise", Some("22H2"), 10, 0, 19045),
            "Windows 10 Enterprise 22H2 (10.0.19045)"
        );
        // Already right, no display version, or a server edition.
        assert_eq!(format_os_version("Windows 11 Home", None, 10, 0, 26100), "Windows 11 Home (10.0.26100)");
        assert_eq!(
            format_os_version("Windows Server 2022 Datacenter", Some("21H2"), 10, 0, 20348),
            "Windows Server 2022 Datacenter 21H2 (10.0.20348)"
        );
        // Kept within the server's 64 characters.
        let long = format_os_version(&"Windows 10 Pro for Workstations ".repeat(4), Some("24H2"), 10, 0, 26100);
        assert!(long.chars().count() <= 64);
        assert!(long.starts_with("Windows 11 Pro for Workstations"));
    }

    #[test]
    fn the_device_id_is_created_once_and_kept() {
        let dir = std::env::temp_dir().join(format!("dosi-device-{}", uuid::Uuid::new_v4()));
        let path = dir.join(DEVICE_FILE);
        let first = load_or_create_device_id(&path).unwrap();
        assert!(uuid::Uuid::parse_str(&first).is_ok());
        assert_eq!(load_or_create_device_id(&path).unwrap(), first, "stable across runs");
        let stored: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(stored["deviceId"], first);

        // A damaged file gets a fresh id (and is repaired).
        std::fs::write(&path, b"{ not json").unwrap();
        let replaced = load_or_create_device_id(&path).unwrap();
        assert_ne!(replaced, first);
        assert_eq!(load_or_create_device_id(&path).unwrap(), replaced);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn utc_offsets_are_in_minutes_and_follow_the_zone() {
        let at = DateTime::parse_from_rfc3339("2026-07-14T09:00:00Z").unwrap().with_timezone(&Utc);
        assert_eq!(utc_offset_minutes_in(at, &FixedOffset::east_opt(6 * 3600).unwrap()), 360);
        assert_eq!(utc_offset_minutes_in(at, &FixedOffset::west_opt(5 * 3600 + 1800).unwrap()), -330);
        assert_eq!(utc_offset_minutes_in(at, &Utc), 0);
        let local = utc_offset_minutes(at);
        assert!((-840..=840).contains(&local));
    }

    #[test]
    fn the_device_context_is_complete_on_this_machine() {
        let dir = std::env::temp_dir().join(format!("dosi-device-{}", uuid::Uuid::new_v4()));
        let info = DeviceInfo::collect_in(Some(&dir));
        assert_eq!(info.platform, "windows");
        assert_eq!(info.agent_version, env!("CARGO_PKG_VERSION"));
        assert_eq!(DeviceInfo::collect_in(Some(&dir)).device_id, info.device_id, "one id per install");
        assert!(info.device_id.is_some());
        assert_eq!(DeviceInfo::collect_in(None).device_id, None, "nowhere to keep it: none");
        let _ = std::fs::remove_dir_all(&dir);
        if cfg!(windows) {
            assert!(info.device_name.as_deref().is_some_and(|n| !n.is_empty()));
            let os = info.os_version.expect("the registry names the OS");
            assert!(os.starts_with("Windows"), "{os}");
            assert!(os.contains("(10.0."), "{os}");
            let zone = time_zone().expect("Windows names its IANA zone");
            assert!(zone.contains('/') || zone == "UTC" || zone.starts_with("Etc/"), "{zone}");
        }
    }
}
