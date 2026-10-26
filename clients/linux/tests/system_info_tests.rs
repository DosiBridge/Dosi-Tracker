#![cfg(target_os = "linux")]
//! Device context (design 8.2): `osVersion` from os-release fixtures of the
//! supported distributions plus the kernel release, the computer name, and a
//! device id that is random, created once, private, stable across runs and
//! different per HOME — never the machine id.

mod support;

use std::os::unix::fs::PermissionsExt;

use dosi_tracker_linux::paths::LinuxPaths;
use dosi_tracker_linux::tracking::system::{
    load_or_create_device_id, os_version_from, DeviceInfo, MAX_OS_VERSION_CHARS,
};
use support::xdg::TestXdg;

const UBUNTU: &str = r#"PRETTY_NAME="Ubuntu 24.04.1 LTS"
NAME="Ubuntu"
VERSION_ID="24.04"
VERSION="24.04.1 LTS (Noble Numbat)"
VERSION_CODENAME=noble
ID=ubuntu
ID_LIKE=debian
HOME_URL="https://www.ubuntu.com/"
UBUNTU_CODENAME=noble
LOGO=ubuntu-logo
"#;

const DEBIAN: &str = r#"PRETTY_NAME="Debian GNU/Linux 12 (bookworm)"
NAME="Debian GNU/Linux"
VERSION_ID="12"
VERSION="12 (bookworm)"
VERSION_CODENAME=bookworm
ID=debian
"#;

const FEDORA: &str = r#"NAME="Fedora Linux"
VERSION="43 (Workstation Edition)"
RELEASE_TYPE=stable
ID=fedora
VERSION_ID=43
PLATFORM_ID="platform:f43"
PRETTY_NAME="Fedora Linux 43 (Workstation Edition)"
ANSI_COLOR="0;38;2;60;110;180"
CPE_NAME="cpe:/o:fedoraproject:fedora:43"
"#;

const ARCH: &str = r#"NAME="Arch Linux"
PRETTY_NAME="Arch Linux"
ID=arch
BUILD_ID=rolling
ANSI_COLOR="38;2;23;147;209"
HOME_URL="https://archlinux.org/"
"#;

/// A fake root with `etc/os-release` (or `usr/lib/os-release`).
fn root_with(rel: &str, text: &str) -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
    root
}

fn paths(xdg: &TestXdg) -> LinuxPaths {
    LinuxPaths::resolve(
        Some(&xdg.home),
        Some(&xdg.config_home),
        Some(&xdg.data_home),
        Some(&xdg.state_home),
    )
}

#[test]
fn os_version_reads_pretty_name_and_the_kernel_for_each_distribution() {
    let cases = [
        (
            UBUNTU,
            "6.8.0-45-generic",
            "Ubuntu 24.04.1 LTS (Linux 6.8.0-45-generic)",
        ),
        (
            DEBIAN,
            "6.1.0-25-amd64",
            "Debian GNU/Linux 12 (bookworm) (Linux 6.1.0-25-amd64)",
        ),
        (
            FEDORA,
            "6.17.1-300.fc43.x86_64",
            "Fedora Linux 43 (Workstation Edit (Linux 6.17.1-300.fc43.x86_64)",
        ),
        (ARCH, "6.17.2-arch1-1", "Arch Linux (Linux 6.17.2-arch1-1)"),
    ];
    for (text, kernel, expected) in cases {
        let root = root_with("etc/os-release", text);
        let version = os_version_from(root.path(), Some(kernel));
        assert_eq!(version, expected, "{text}");
        assert!(version.chars().count() <= MAX_OS_VERSION_CHARS);
    }
}

#[test]
fn os_version_falls_back_sensibly() {
    // /usr/lib/os-release when /etc has none.
    let root = root_with("usr/lib/os-release", ARCH);
    assert_eq!(
        os_version_from(root.path(), Some("6.1")),
        "Arch Linux (Linux 6.1)"
    );
    // NAME and VERSION without PRETTY_NAME.
    let root = root_with("etc/os-release", "NAME=\"Example OS\"\nVERSION=\"7\"\n");
    assert_eq!(os_version_from(root.path(), None), "Example OS 7");
    // Nothing at all.
    let empty = tempfile::tempdir().unwrap();
    assert_eq!(
        os_version_from(empty.path(), Some("6.9.0")),
        "Linux (Linux 6.9.0)"
    );
    assert_eq!(os_version_from(empty.path(), None), "Linux");
    // Always within 64 characters, cut at a character boundary.
    let root = root_with(
        "etc/os-release",
        &format!("PRETTY_NAME=\"{}\"\n", "ü".repeat(80)),
    );
    assert_eq!(
        os_version_from(root.path(), Some("6.1")).chars().count(),
        MAX_OS_VERSION_CHARS
    );
}

#[test]
fn the_device_id_is_created_once_privately_and_stays_stable() {
    let xdg = TestXdg::new();
    let file = paths(&xdg).device_file();
    let first = load_or_create_device_id(&file).unwrap();
    assert!(first.created);
    assert_eq!(first.id.get_version_num(), 4, "a random UUIDv4");
    assert_eq!(
        std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        std::fs::metadata(file.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    let again = load_or_create_device_id(&file).unwrap();
    assert!(!again.created);
    assert_eq!(again.id, first.id, "stable across runs");

    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(json["device_id"], first.id.to_string());
    assert!(json["created_at"].is_string());

    if let Ok(machine_id) = std::fs::read_to_string("/etc/machine-id") {
        assert_ne!(
            first.id.simple().to_string(),
            machine_id.trim(),
            "never the machine id"
        );
    }
}

#[test]
fn every_home_gets_its_own_device_id() {
    let one = TestXdg::new();
    let two = TestXdg::new();
    let a = load_or_create_device_id(&paths(&one).device_file()).unwrap();
    let b = load_or_create_device_id(&paths(&two).device_file()).unwrap();
    assert_ne!(a.id, b.id);
}

#[test]
fn agents_starting_together_agree_on_one_id() {
    let xdg = TestXdg::new();
    let file = paths(&xdg).device_file();
    let ids: Vec<_> = (0..8)
        .map(|_| {
            let file = file.clone();
            std::thread::spawn(move || load_or_create_device_id(&file).unwrap())
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|h| h.join().unwrap())
        .collect();
    assert!(ids.iter().all(|d| d.id == ids[0].id), "{ids:?}");
    assert_eq!(ids.iter().filter(|d| d.created).count(), 1);
}

#[test]
fn a_damaged_device_file_is_moved_aside_and_replaced() {
    let xdg = TestXdg::new();
    let file = paths(&xdg).device_file();
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, b"{\"device_id\": \"not-a-uuid\"}").unwrap();
    let id = load_or_create_device_id(&file).unwrap();
    assert!(id.created);
    let aside = id.moved_aside.expect("moved aside");
    assert!(aside
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("device.json.damaged-"));
    assert_eq!(load_or_create_device_id(&file).unwrap().id, id.id);

    // A symlink is never followed or replaced.
    let link_home = TestXdg::new();
    let link = paths(&link_home).device_file();
    std::fs::create_dir_all(link.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&file, &link).unwrap();
    assert!(load_or_create_device_id(&link).is_err());
}

#[test]
fn device_info_carries_the_linux_context() {
    let xdg = TestXdg::new();
    let root = root_with("etc/os-release", DEBIAN);
    let info = DeviceInfo::collect_with(
        &paths(&xdg).device_file(),
        root.path(),
        Some(("  dev-box\n".into(), "6.1.0-25-amd64".into())),
    )
    .unwrap();
    assert_eq!(info.platform, "linux");
    assert_eq!(info.device_name.as_deref(), Some("dev-box"));
    assert_eq!(
        info.os_version,
        "Debian GNU/Linux 12 (bookworm) (Linux 6.1.0-25-amd64)"
    );
    assert_eq!(info.agent_version, env!("CARGO_PKG_VERSION"));
    assert_eq!(
        uuid::Uuid::parse_str(&info.device_id)
            .unwrap()
            .get_version_num(),
        4
    );
    let long_name = "n".repeat(200);
    let info = DeviceInfo::collect_with(
        &paths(&xdg).device_file(),
        root.path(),
        Some((long_name, String::new())),
    )
    .unwrap();
    assert_eq!(info.device_name.as_ref().unwrap().chars().count(), 128);
    assert_eq!(
        info.os_version, "Debian GNU/Linux 12 (bookworm)",
        "no kernel known"
    );
    let offset = info.utc_offset_minutes(chrono::Utc::now());
    assert!((-840..=840).contains(&offset));
}
