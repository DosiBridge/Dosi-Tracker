//! Ephemeral private `dbus-daemon`s for D-Bus tests.
//!
//! Each bus listens on a socket in its own temporary directory; nothing
//! connects to the developer's session or system bus. A system-flavoured bus
//! stands in for the system bus (logind) and is handed to code under test
//! explicitly (or through `DBUS_SYSTEM_BUS_ADDRESS` in a child's environment).

use std::fs;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};

use tempfile::TempDir;

/// Which bus a [`PrivateTestBus`] stands in for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusKind {
    Session,
    System,
}

impl BusKind {
    fn config_type(self) -> &'static str {
        match self {
            BusKind::Session => "session",
            BusKind::System => "system",
        }
    }
}

/// A private `dbus-daemon`, killed when dropped.
pub struct PrivateTestBus {
    child: Child,
    address: String,
    kind: BusKind,
    _temp_dir: TempDir,
}

impl PrivateTestBus {
    /// A private session bus.
    pub fn new() -> Self {
        Self::start(BusKind::Session)
    }

    /// A private session bus.
    pub fn session() -> Self {
        Self::start(BusKind::Session)
    }

    /// A private bus that plays the system bus (logind and friends).
    pub fn system() -> Self {
        Self::start(BusKind::System)
    }

    /// Start a private bus of the given kind. The bus is listening once this
    /// returns (`dbus-daemon` prints its address only after binding).
    pub fn start(kind: BusKind) -> Self {
        let temp_dir = tempfile::Builder::new()
            .prefix("dosi-test-dbus-")
            .tempdir()
            .expect("Failed creating tempdir for private dbus socket");

        let config_path = temp_dir.path().join("bus.conf");
        let socket_dir = temp_dir.path().to_str().unwrap();
        let bus_type = kind.config_type();
        let config_content = format!(
            r#"<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>{bus_type}</type>
  <listen>unix:tmpdir={socket_dir}</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
"#
        );
        fs::write(&config_path, config_content).expect("Failed writing private dbus config");

        let mut child = Command::new("dbus-daemon")
            .arg(format!("--config-file={}", config_path.display()))
            .arg("--print-address=1")
            .arg("--nofork")
            .env_remove("DBUS_SESSION_BUS_ADDRESS")
            .env_remove("DBUS_SYSTEM_BUS_ADDRESS")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("Failed to spawn private dbus-daemon (is dbus-daemon installed?)");

        let stdout = child.stdout.take().expect("Child must have stdout");
        let mut reader = BufReader::new(stdout);
        let mut address = String::new();
        reader
            .read_line(&mut address)
            .expect("Failed reading private bus address from dbus-daemon stdout");
        let address = address.trim().to_string();
        assert!(
            !address.is_empty(),
            "Empty address returned by private dbus-daemon"
        );

        Self {
            child,
            address,
            kind,
            _temp_dir: temp_dir,
        }
    }

    /// The bus address, e.g. `unix:path=/tmp/dosi-test-dbus-XXXX/dbus-YYYY,guid=…`.
    pub fn address(&self) -> &str {
        &self.address
    }

    pub fn kind(&self) -> BusKind {
        self.kind
    }
}

impl Default for PrivateTestBus {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for PrivateTestBus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
