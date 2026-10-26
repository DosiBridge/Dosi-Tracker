//! The packaging assets agree with each other and with the code:
//!
//! - the app desktop entry (template) has the design 15.2 keys, uses the app
//!   id the GUI and the tray use, and its actions run real CLI commands;
//! - the AppStream metainfo names the same app, discloses what is and is never
//!   collected, and has release notes for this crate version;
//! - every hicolor PNG decodes at the size of its directory, and the SVG
//!   exists;
//! - the deb and rpm metadata install the same files (nothing under /etc),
//!   from sources that exist or that `packaging/build-dist.sh` renders;
//! - `config.example.toml` loads without a warning, covers every key and shows
//!   the built-in defaults.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use dosi_tracker_linux::agent::config::{self, AppConfig, ConfigSources};
use dosi_tracker_linux::paths::APP_ID;

const ICON_SIZES: [u32; 9] = [16, 22, 24, 32, 48, 64, 128, 256, 512];

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(relative: &str) -> String {
    std::fs::read_to_string(crate_dir().join(relative))
        .unwrap_or_else(|e| panic!("{relative}: {e}"))
}

fn desktop_entry() -> String {
    read(&format!("packaging/desktop/{APP_ID}.desktop"))
}

/// `key=value` lines of one `[group]` of a desktop entry.
fn desktop_group(text: &str, group: &str) -> Vec<(String, String)> {
    let mut in_group = false;
    let mut pairs = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_group = line == format!("[{group}]");
            continue;
        }
        if !in_group || line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line.split_once('=').expect("key=value");
        pairs.push((key.to_owned(), value.to_owned()));
    }
    pairs
}

fn value<'a>(pairs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    pairs
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

#[test]
fn the_desktop_entry_has_the_design_keys() {
    let text = desktop_entry();
    let main = desktop_group(&text, "Desktop Entry");
    let expect = [
        ("Type", "Application"),
        ("Name", "Dosi Tracker"),
        ("GenericName", "Time Tracker"),
        ("Exec", "@BINDIR@/dosi-tracker-gui"),
        ("TryExec", "@BINDIR@/dosi-tracker-gui"),
        ("Icon", APP_ID),
        ("Terminal", "false"),
        ("StartupNotify", "true"),
        ("StartupWMClass", APP_ID),
        ("X-GNOME-SingleWindow", "true"),
        ("Actions", "pause;resume;sync;"),
    ];
    for (key, expected) in expect {
        assert_eq!(value(&main, key), Some(expected), "{key}");
    }
    let categories = value(&main, "Categories").unwrap();
    assert!(categories.contains("Office;") && categories.contains("ProjectManagement;"));
    assert!(value(&main, "Keywords").is_some_and(|k| k.ends_with(';')));
    // Never an autostart entry: that one is only written by `service enable`.
    assert!(!text.contains("X-GNOME-Autostart") && !text.contains("NoDisplay=true"));
    assert!(!text.contains("X-DosiTracker-Managed"));
}

/// The desktop actions run CLI commands that exist.
#[cfg(target_os = "linux")]
#[test]
fn the_desktop_actions_run_real_commands() {
    use clap::Parser;
    let text = desktop_entry();
    for action in ["pause", "resume", "sync"] {
        let group = desktop_group(&text, &format!("Desktop Action {action}"));
        let exec = value(&group, "Exec").unwrap_or_else(|| panic!("action {action}"));
        assert_eq!(exec, format!("@BINDIR@/dosi-tracker {action}"));
        assert!(value(&group, "Name").is_some());
        let argv: Vec<&str> = exec.split(' ').collect();
        dosi_tracker_linux::cli::Cli::try_parse_from(argv)
            .unwrap_or_else(|e| panic!("{exec}: {e}"));
    }
}

#[test]
fn the_metainfo_matches_the_app_and_discloses_monitoring() {
    let text = read(&format!("packaging/metainfo/{APP_ID}.metainfo.xml"));
    assert!(text.contains(&format!("<id>{APP_ID}</id>")));
    assert!(text.contains(&format!(
        "<launchable type=\"desktop-id\">{APP_ID}.desktop</launchable>"
    )));
    assert!(text.contains("<component type=\"desktop-application\">"));
    assert!(text.contains("<project_license>MIT</project_license>"));
    assert!(text.contains("<content_rating type=\"oars-1.1\">"));
    // The monitoring nature and the never-collected list are disclosed.
    assert!(text.contains("workplace monitoring software"));
    assert!(text.contains("never collects"));
    for never in ["keystrokes", "clipboard", "webcam"] {
        assert!(text.contains(never), "{never}");
    }
    // Every version that ships has release notes.
    assert!(
        text.contains(&format!(
            "<release version=\"{}\"",
            env!("CARGO_PKG_VERSION")
        )),
        "add a <release> entry for {}",
        env!("CARGO_PKG_VERSION")
    );
}

#[test]
fn every_icon_has_the_size_of_its_directory() {
    let root = crate_dir().join("packaging/icons/hicolor");
    for size in ICON_SIZES {
        let path = root.join(format!("{size}x{size}/apps/{APP_ID}.png"));
        let image = image::open(&path)
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
            .to_rgba8();
        assert_eq!(image.dimensions(), (size, size), "{}", path.display());
        // A white centre dot on the brand disc, transparent corners.
        let centre = image.get_pixel(size / 2, size / 2);
        assert_eq!(centre.0, [0xff, 0xff, 0xff, 0xff], "{size}: centre");
        assert_eq!(image.get_pixel(0, 0).0[3], 0, "{size}: corner");
    }
    let svg = std::fs::read_to_string(root.join(format!("scalable/apps/{APP_ID}.svg"))).unwrap();
    assert!(svg.contains("<svg") && svg.contains("viewBox=\"0 0 32 32\""));
    // The icons directories hold exactly these sizes (no stray files).
    let mut dirs: BTreeSet<String> = std::fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(dirs.remove("scalable"));
    let expected: BTreeSet<String> = ICON_SIZES.iter().map(|s| format!("{s}x{s}")).collect();
    assert_eq!(dirs, expected);
}

/// Files `packaging/build-dist.sh` renders into `target/release/dist-assets`.
const RENDERED: [&str; 5] = [
    "target/release/dist-assets/usr/dosi-tracker.service",
    "target/release/dist-assets/usr/io.github.dosibridge.DosiTracker.desktop",
    "target/release/dist-assets/completions/dosi-tracker.bash",
    "target/release/dist-assets/completions/_dosi-tracker",
    "target/release/dist-assets/completions/dosi-tracker.fish",
];

fn metadata() -> toml::Value {
    toml::from_str::<toml::Value>(&read("Cargo.toml")).unwrap()["package"]["metadata"].clone()
}

/// (source, destination, mode) of the deb assets, destinations absolute.
fn deb_assets() -> Vec<(String, String, String)> {
    metadata()["deb"]["assets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| {
            let a = a.as_array().unwrap();
            let s = |i: usize| a[i].as_str().unwrap().to_owned();
            (s(0), format!("/{}", s(1)), s(2))
        })
        .collect()
}

fn rpm_assets() -> Vec<(String, String, String)> {
    metadata()["generate-rpm"]["assets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| {
            let s = |k: &str| a[k].as_str().unwrap().to_owned();
            (s("source"), s("dest"), s("mode"))
        })
        .collect()
}

#[test]
fn deb_and_rpm_install_the_same_files_from_known_sources() {
    let deb = deb_assets();
    let rpm = rpm_assets();
    let rendered: BTreeSet<&str> = RENDERED.into_iter().collect();
    for (source, dest, mode) in deb.iter().chain(rpm.iter()) {
        if source.starts_with("target/") {
            assert!(
                rendered.contains(source.as_str())
                    || matches!(
                        source.as_str(),
                        "target/release/dosi-tracker" | "target/release/dosi-tracker-gui"
                    ),
                "{source} is neither built nor rendered by build-dist.sh"
            );
        } else {
            assert!(crate_dir().join(source).is_file(), "{source} is missing");
        }
        assert!(
            dest.starts_with("/usr/"),
            "{dest}: packages install under /usr only"
        );
        let expected_mode = if dest.starts_with("/usr/bin/") {
            "755"
        } else {
            "644"
        };
        assert_eq!(mode, expected_mode, "{dest}");
    }
    // Debian's zsh reads vendor-completions, Fedora's site-functions.
    let normalize = |assets: &[(String, String, String)]| -> BTreeSet<(String, String)> {
        assets
            .iter()
            .map(|(s, d, _)| {
                (
                    s.clone(),
                    d.replace("/zsh/vendor-completions/", "/zsh/")
                        .replace("/zsh/site-functions/", "/zsh/"),
                )
            })
            .collect()
    };
    assert_eq!(normalize(&deb), normalize(&rpm));
    let dests: BTreeSet<&str> = deb.iter().map(|(_, d, _)| d.as_str()).collect();
    for required in [
        "/usr/bin/dosi-tracker".to_owned(),
        "/usr/bin/dosi-tracker-gui".to_owned(),
        "/usr/lib/systemd/user/dosi-tracker.service".to_owned(),
        format!("/usr/share/applications/{APP_ID}.desktop"),
        format!("/usr/share/metainfo/{APP_ID}.metainfo.xml"),
        format!("/usr/share/icons/hicolor/scalable/apps/{APP_ID}.svg"),
    ] {
        assert!(dests.contains(required.as_str()), "{required}");
    }
    for size in ICON_SIZES {
        let icon = format!("/usr/share/icons/hicolor/{size}x{size}/apps/{APP_ID}.png");
        assert!(dests.contains(icon.as_str()), "{icon}");
    }
    // No maintainer scripts and no generated systemd enablement.
    let deb_meta = &metadata()["deb"];
    for key in ["maintainer-scripts", "systemd-units", "conf-files"] {
        assert!(
            deb_meta.get(key).is_none(),
            "deb metadata must not set {key}"
        );
    }
    let rpm_meta = &metadata()["generate-rpm"];
    for key in [
        "pre_install_script",
        "post_install_script",
        "pre_uninstall_script",
        "post_uninstall_script",
    ] {
        assert!(
            rpm_meta.get(key).is_none(),
            "rpm metadata must not set {key}"
        );
    }
}

#[test]
fn the_package_dependencies_follow_the_design() {
    let deb = &metadata()["deb"];
    assert_eq!(deb["name"].as_str(), Some("dosi-tracker"));
    assert_eq!(deb["section"].as_str(), Some("utils"));
    assert_eq!(deb["depends"].as_str(), Some("$auto"));
    let recommends = deb["recommends"].as_str().unwrap();
    for lib in [
        "libegl1 | libgl1",
        "libxkbcommon-x11-0",
        "libxcursor1",
        "libxrandr2",
        "libxi6",
        "libx11-xcb1",
        "xdg-desktop-portal",
        "gnome-keyring | kwalletmanager | keepassxc",
        "ca-certificates",
    ] {
        assert!(recommends.contains(lib), "deb recommends {lib}");
    }
    assert_eq!(
        deb["suggests"].as_str(),
        Some("gnome-shell-extension-appindicator")
    );
    let rpm = &metadata()["generate-rpm"];
    assert_eq!(rpm["name"].as_str(), Some("dosi-tracker"));
    let recommends = rpm["recommends"].as_table().unwrap();
    for lib in [
        "mesa-libEGL",
        "libxkbcommon-x11",
        "libXcursor",
        "libXrandr",
        "libXi",
        "xdg-desktop-portal",
        "gnome-keyring",
    ] {
        assert!(recommends.contains_key(lib), "rpm recommends {lib}");
    }
}

/// Dotted keys of a TOML table.
fn flatten(prefix: &str, table: &toml::value::Table, out: &mut BTreeSet<String>) {
    for (key, value) in table {
        let dotted = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        match value {
            toml::Value::Table(inner) => flatten(&dotted, inner, out),
            _ => {
                out.insert(dotted);
            }
        }
    }
}

#[test]
fn the_example_configuration_is_complete_valid_and_shows_the_defaults() {
    let path: &Path = &crate_dir().join("config.example.toml");
    let text = std::fs::read_to_string(path).unwrap();
    let mut keys = BTreeSet::new();
    flatten("", text.parse::<toml::Table>().as_ref().unwrap(), &mut keys);
    let all: BTreeSet<String> = config::keys().map(str::to_owned).collect();
    assert_eq!(keys, all, "config.example.toml must list every key once");

    let loaded = AppConfig::load(&ConfigSources {
        system_file: path.to_path_buf(),
        user_file: crate_dir().join("does-not-exist/config.toml"),
        env: Vec::new(),
    });
    assert!(loaded.issues.is_empty(), "{:?}", loaded.issues);
    let defaults = AppConfig::default();
    for key in config::keys() {
        if matches!(key, "api_base_url" | "dashboard_url") {
            assert!(loaded.config.value(key).is_some(), "{key}");
            continue;
        }
        assert_eq!(
            loaded.config.value(key),
            defaults.value(key),
            "config.example.toml must show the default of {key}"
        );
    }
}
