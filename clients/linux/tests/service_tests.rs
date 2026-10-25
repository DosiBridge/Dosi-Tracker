#![cfg(target_os = "linux")]
//! Service integration with a recording `systemctl` first on the PATH
//! (`support::fakes::systemd_shim`): the session-start call sequence, the env
//! file written 0600 before `start`, restart on a new login session, the
//! detached fallback without a user manager, the one-second budget, managed
//! autostart entries and units (unmanaged files untouched), and the rendered
//! unit and entry checked by `systemd-analyze verify --user` and
//! `desktop-file-validate`.

mod support;

use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use dosi_tracker_linux::paths::LinuxPaths;
use dosi_tracker_linux::service::{
    self, disable_autostart, enable_autostart, install_user_unit, render_autostart, render_unit,
    session_start, FileChange, ServiceEnv, ServiceError, SessionStart, SessionStartContext,
    Systemctl, UnitInstall, MANAGED_MARKER,
};
use support::fakes::systemd_shim::SystemctlShim;
use support::xdg::TestXdg;

fn paths(xdg: &TestXdg) -> LinuxPaths {
    LinuxPaths::resolve(
        Some(&xdg.home),
        Some(&xdg.config_home),
        Some(&xdg.data_home),
        Some(&xdg.state_home),
    )
    .with_runtime_dir(xdg.runtime_dir.as_deref())
}

/// A temporary home, its paths, and a shim watching the session env file.
struct Setup {
    xdg: TestXdg,
    paths: LinuxPaths,
    shim: SystemctlShim,
    systemctl: Systemctl,
}

fn setup() -> Setup {
    let xdg = TestXdg::new();
    let paths = paths(&xdg);
    let shim = SystemctlShim::new(&paths.session_env_file().unwrap());
    let systemctl = systemctl_for(&xdg, &shim);
    Setup {
        xdg,
        paths,
        shim,
        systemctl,
    }
}

fn systemctl_for(xdg: &TestXdg, shim: &SystemctlShim) -> Systemctl {
    let mut vars: Vec<(String, String)> = vec![
        ("PATH".into(), shim.path_var()),
        ("HOME".into(), xdg.home.display().to_string()),
        // Never handed to systemctl: only the variables a manager client needs.
        ("DISPLAY".into(), ":0".into()),
        ("SOME_TOKEN".into(), "canary".into()),
    ];
    if let Some(runtime) = &xdg.runtime_dir {
        vars.push(("XDG_RUNTIME_DIR".into(), runtime.display().to_string()));
    }
    Systemctl::new(ServiceEnv::from_vars(vars))
}

fn session_vars(id: &str) -> BTreeMap<String, String> {
    [
        ("DISPLAY", ":0"),
        ("XAUTHORITY", "/run/user/1000/gdm/Xauthority"),
        ("XDG_SESSION_TYPE", "x11"),
        ("XDG_SESSION_ID", id),
        ("XDG_CURRENT_DESKTOP", "ubuntu:GNOME"),
        ("XDG_SESSION_DESKTOP", "ubuntu"),
        ("DBUS_SESSION_BUS_ADDRESS", "unix:path=/run/user/1000/bus"),
        ("HOME", "/never/written"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v.to_owned()))
    .collect()
}

/// A stand-in `dosi-tracker` that records how it was started.
fn recording_exe(dir: &Path) -> (PathBuf, PathBuf) {
    let exe = dir.join("dosi-tracker");
    let record = dir.join("spawned.txt");
    std::fs::write(
        &exe,
        format!(
            "#!/bin/sh\n{{ echo \"ARGS $*\"; env | sort; }} > '{}.tmp' && mv '{0}.tmp' '{0}'\n",
            record.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
    (exe, record)
}

fn context<'a>(
    s: &'a Setup,
    vars: BTreeMap<String, String>,
    exe: PathBuf,
) -> SessionStartContext<'a> {
    SessionStartContext {
        paths: &s.paths,
        session_vars: vars,
        exe,
        systemctl: &s.systemctl,
    }
}

const SHOW: &str = "--user show dosi-tracker.service --property=LoadState --property=ActiveState --property=MainPID --property=FragmentPath";

/// Variables a shell adds on its own.
fn without_shell_vars(names: &[String]) -> Vec<&str> {
    names
        .iter()
        .map(String::as_str)
        .filter(|n| !matches!(*n, "PWD" | "OLDPWD" | "SHLVL" | "_"))
        .collect()
}

#[test]
fn session_start_writes_the_env_file_then_starts_the_unit() {
    let s = setup();
    let started = Instant::now();
    let outcome = session_start(&context(
        &s,
        session_vars("2"),
        PathBuf::from("/usr/bin/dosi-tracker"),
    ))
    .unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "returns within a second"
    );
    assert_eq!(outcome, SessionStart::Started);

    assert_eq!(
        s.shim.lines(),
        [
            "--user show-environment".to_owned(),
            SHOW.to_owned(),
            "--user --no-block start dosi-tracker.service".to_owned(),
        ]
    );
    let calls = s.shim.calls();
    assert_eq!(
        calls[0].watched_mode, "600",
        "the env file exists before systemctl runs"
    );
    assert_eq!(
        calls[2].watched_mode, "600",
        "and is 0600 when the unit starts"
    );
    for call in &calls {
        assert_eq!(
            without_shell_vars(&call.env_names),
            ["HOME", "LC_ALL", "PATH", "XDG_RUNTIME_DIR"],
            "an explicit environment"
        );
    }

    let env_file = s.paths.session_env_file().unwrap();
    let text = std::fs::read_to_string(&env_file).unwrap();
    assert!(text.contains("DISPLAY=\":0\"\n"));
    assert!(text.contains("XDG_SESSION_ID=\"2\"\n"));
    assert!(text.contains("DBUS_SESSION_BUS_ADDRESS=\"unix:path=/run/user/1000/bus\"\n"));
    assert!(!text.contains("HOME="), "only session variables");
    assert_eq!(
        std::fs::metadata(&env_file).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn a_new_login_session_restarts_a_running_agent() {
    let s = setup();
    let exe = PathBuf::from("/usr/bin/dosi-tracker");
    assert_eq!(
        session_start(&context(&s, session_vars("2"), exe.clone())).unwrap(),
        SessionStart::Started
    );

    // The same session again (the hook ran twice): start, which is a no-op.
    s.shim.set_unit(
        "loaded",
        "active",
        4242,
        Some("/usr/lib/systemd/user/dosi-tracker.service"),
    );
    assert_eq!(
        session_start(&context(&s, session_vars("2"), exe.clone())).unwrap(),
        SessionStart::Started
    );

    // Logged out and in again: the running agent belongs to the old session.
    assert_eq!(
        session_start(&context(&s, session_vars("5"), exe.clone())).unwrap(),
        SessionStart::Restarted
    );
    assert_eq!(
        s.shim.lines().last().unwrap(),
        "--user --no-block restart dosi-tracker.service"
    );
    assert!(std::fs::read_to_string(s.paths.session_env_file().unwrap())
        .unwrap()
        .contains("XDG_SESSION_ID=\"5\""));

    // A new session with the agent stopped: a plain start.
    s.shim.set_unit(
        "loaded",
        "inactive",
        0,
        Some("/usr/lib/systemd/user/dosi-tracker.service"),
    );
    assert_eq!(
        session_start(&context(&s, session_vars("6"), exe)).unwrap(),
        SessionStart::Started
    );
    assert_eq!(
        s.shim.lines().last().unwrap(),
        "--user --no-block start dosi-tracker.service"
    );
}

#[test]
fn without_a_user_manager_the_daemon_is_spawned_detached() {
    let s = setup();
    s.shim.set_user_manager(false);
    let bin = tempfile::tempdir().unwrap();
    let (exe, record) = recording_exe(bin.path());
    let outcome = session_start(&context(&s, session_vars("2"), exe)).unwrap();
    assert!(
        matches!(outcome, SessionStart::Detached { pid } if pid > 0),
        "{outcome:?}"
    );
    assert_eq!(
        s.shim.lines(),
        ["--user show-environment"],
        "nothing is started through systemd"
    );

    let deadline = Instant::now() + Duration::from_secs(5);
    while !record.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    let spawned = std::fs::read_to_string(&record).expect("the daemon was started");
    assert!(spawned.starts_with("ARGS daemon --detach\n"), "{spawned}");
    for line in [
        "DISPLAY=:0",
        "XDG_SESSION_ID=2",
        &format!(
            "XDG_RUNTIME_DIR={}",
            s.xdg.runtime_dir.as_ref().unwrap().display()
        ),
        &format!("HOME={}", s.xdg.home.display()),
        &format!("XDG_CONFIG_HOME={}", s.xdg.config_home.display()),
        &format!("XDG_STATE_HOME={}", s.xdg.state_home.display()),
    ] {
        assert!(spawned.lines().any(|l| l == line), "{line} in {spawned}");
    }
    assert!(!spawned.contains("SOME_TOKEN"), "an explicit environment");
    assert!(!spawned.contains("/never/written"));
}

#[test]
fn without_a_loadable_unit_the_daemon_is_spawned_detached_too() {
    let s = setup();
    s.shim.set_unit("not-found", "inactive", 0, None);
    let bin = tempfile::tempdir().unwrap();
    let (exe, _record) = recording_exe(bin.path());
    let outcome = session_start(&context(&s, session_vars("2"), exe)).unwrap();
    assert!(matches!(outcome, SessionStart::Detached { .. }));
    assert!(!s.shim.lines().iter().any(|l| l.contains(" start ")));
}

#[test]
fn session_start_returns_within_a_second_even_when_systemctl_hangs() {
    let s = setup();
    s.shim.set_sleep(Some(5.0));
    let bin = tempfile::tempdir().unwrap();
    let (exe, _record) = recording_exe(bin.path());
    let started = Instant::now();
    let outcome = session_start(&context(&s, session_vars("2"), exe)).unwrap();
    let took = started.elapsed();
    assert!(took < Duration::from_secs(1), "{took:?}");
    assert!(
        matches!(outcome, SessionStart::Detached { .. }),
        "{outcome:?}"
    );

    // A single call is bounded by the wrapper's own limit as well.
    let quick = systemctl_for(&s.xdg, &s.shim).with_timeout(Duration::from_millis(200));
    let started = Instant::now();
    assert!(matches!(
        quick.unit("stop", false, None),
        Err(ServiceError::Timeout(_))
    ));
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn invalid_session_values_never_reach_the_unit() {
    let s = setup();
    let mut vars = session_vars("2");
    vars.insert("DISPLAY".into(), ":0\nLD_PRELOAD=/tmp/evil.so".into());
    let result = session_start(&context(&s, vars, PathBuf::from("/usr/bin/dosi-tracker")));
    assert!(
        matches!(result, Err(ServiceError::InvalidValue { .. })),
        "{result:?}"
    );
    assert!(!s.paths.session_env_file().unwrap().exists());
    assert!(s.shim.lines().is_empty(), "nothing started");
}

#[test]
fn without_a_runtime_dir_nothing_is_started() {
    let xdg = TestXdg::builder().runtime_dir(false).build();
    let paths = paths(&xdg);
    let shim = SystemctlShim::new(&xdg.home.join("unused"));
    let systemctl = systemctl_for(&xdg, &shim);
    let ctx = SessionStartContext {
        paths: &paths,
        session_vars: session_vars("2"),
        exe: PathBuf::from("/usr/bin/dosi-tracker"),
        systemctl: &systemctl,
    };
    assert!(matches!(
        session_start(&ctx),
        Err(ServiceError::NoRuntimeDir)
    ));
    assert!(shim.lines().is_empty());
    assert!(
        !systemctl.user_manager_available(None),
        "no XDG_RUNTIME_DIR, no user manager"
    );
}

#[test]
fn the_managed_autostart_entry_is_written_and_removed_with_an_absolute_exec() {
    let s = setup();
    let entry = s.paths.autostart_entry();
    let exe = Path::new("/home/a/.local/bin/dosi-tracker");
    assert!(!service::autostart_enabled(&entry));
    assert_eq!(enable_autostart(&entry, exe).unwrap(), FileChange::Written);
    assert_eq!(
        enable_autostart(&entry, exe).unwrap(),
        FileChange::Unchanged
    );
    let text = std::fs::read_to_string(&entry).unwrap();
    assert!(text.contains("Exec=/home/a/.local/bin/dosi-tracker service session-start\n"));
    assert!(text.contains("TryExec=/home/a/.local/bin/dosi-tracker\n"));
    assert!(text.lines().any(|l| l == MANAGED_MARKER));
    assert!(service::autostart_enabled(&entry));
    assert!(service::status(&s.paths, &s.systemctl).enabled_at_login);
    assert!(
        enable_autostart(&entry, Path::new("bin/dosi-tracker")).is_err(),
        "relative Exec refused"
    );

    assert_eq!(disable_autostart(&entry).unwrap(), FileChange::Removed);
    assert_eq!(disable_autostart(&entry).unwrap(), FileChange::Absent);
    assert!(!entry.exists());
}

#[test]
fn unmanaged_autostart_entries_are_left_untouched() {
    let s = setup();
    let entry = s.paths.autostart_entry();
    std::fs::create_dir_all(entry.parent().unwrap()).unwrap();
    let theirs = "[Desktop Entry]\nType=Application\nName=Mine\nExec=/usr/bin/true\n";
    std::fs::write(&entry, theirs).unwrap();
    assert!(matches!(
        enable_autostart(&entry, Path::new("/usr/bin/dosi-tracker")),
        Err(ServiceError::Unmanaged(_))
    ));
    assert!(matches!(
        disable_autostart(&entry),
        Err(ServiceError::Unmanaged(_))
    ));
    assert_eq!(std::fs::read_to_string(&entry).unwrap(), theirs);
    assert!(!service::autostart_enabled(&entry));

    // A symlink is never followed.
    std::fs::remove_file(&entry).unwrap();
    let target = s.xdg.home.join("target.desktop");
    std::fs::write(
        &target,
        render_autostart(Path::new("/usr/bin/dosi-tracker")).unwrap(),
    )
    .unwrap();
    std::os::unix::fs::symlink(&target, &entry).unwrap();
    assert!(matches!(
        disable_autostart(&entry),
        Err(ServiceError::Unmanaged(_))
    ));
    assert!(target.exists());
}

#[test]
fn the_user_unit_is_written_only_when_none_is_loadable() {
    let s = setup();
    let bindir = Path::new("/opt/dosi/bin");
    let target = s.paths.user_unit_file();

    // A packaged unit exists: nothing is written.
    let result = install_user_unit(&s.paths, bindir, &s.systemctl).unwrap();
    assert_eq!(
        result,
        UnitInstall::AlreadyLoadable(Some("/usr/lib/systemd/user/dosi-tracker.service".into()))
    );
    assert!(!target.exists());

    // A custom prefix: the manager finds none, so ours is written and loaded.
    s.shim.set_unit("not-found", "inactive", 0, None);
    assert_eq!(
        install_user_unit(&s.paths, bindir, &s.systemctl).unwrap(),
        UnitInstall::Written(target.clone())
    );
    let text = std::fs::read_to_string(&target).unwrap();
    assert!(text.contains("ExecStart=/opt/dosi/bin/dosi-tracker daemon --systemd"));
    assert!(service::is_managed(&text));
    assert_eq!(s.shim.lines().last().unwrap(), "--user daemon-reload");

    // Loaded from our own file next time: kept as it is.
    s.shim
        .set_unit("loaded", "inactive", 0, Some(target.to_str().unwrap()));
    assert_eq!(
        install_user_unit(&s.paths, bindir, &s.systemctl).unwrap(),
        UnitInstall::Unchanged(target.clone())
    );
    assert_eq!(
        service::remove_user_unit(&s.paths).unwrap(),
        FileChange::Removed
    );

    // Someone else's unit with that name is never replaced.
    std::fs::write(&target, "[Service]\nExecStart=/usr/bin/true\n").unwrap();
    s.shim.set_unit("not-found", "inactive", 0, None);
    assert!(matches!(
        install_user_unit(&s.paths, bindir, &s.systemctl),
        Err(ServiceError::Unmanaged(_))
    ));
    assert!(matches!(
        service::remove_user_unit(&s.paths),
        Err(ServiceError::Unmanaged(_))
    ));
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "[Service]\nExecStart=/usr/bin/true\n"
    );
}

#[test]
fn status_reports_installed_enabled_active_and_pid() {
    let s = setup();
    s.shim.set_unit(
        "loaded",
        "active",
        4242,
        Some("/usr/lib/systemd/user/dosi-tracker.service"),
    );
    let status = service::status(&s.paths, &s.systemctl);
    assert!(status.user_manager && status.installed && status.active);
    assert!(!status.enabled_at_login);
    assert_eq!(status.pid, Some(4242));
    assert_eq!(
        status.unit_path.as_deref(),
        Some("/usr/lib/systemd/user/dosi-tracker.service")
    );

    s.shim.set_user_manager(false);
    let status = service::status(&s.paths, &s.systemctl);
    assert!(!status.user_manager && !status.installed && !status.active);
    assert_eq!(status.pid, None);
}

#[test]
fn a_failing_start_is_reported() {
    let s = setup();
    s.shim.set_failing(true);
    let result = session_start(&context(
        &s,
        session_vars("2"),
        PathBuf::from("/usr/bin/dosi-tracker"),
    ));
    match result {
        Err(ServiceError::Systemctl(message)) => {
            assert!(message.contains("Job failed"), "{message}")
        }
        other => panic!("{other:?}"),
    }
}

/// Run a validator; the CI image has them (they are part of the gate).
fn run_tool(program: &str, args: &[&str], xdg: &TestXdg) -> std::process::Output {
    xdg.command(program)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("{program} is required for this test (install it): {e}"))
}

#[test]
fn the_rendered_unit_passes_systemd_analyze_verify() {
    let xdg = TestXdg::new();
    let root = tempfile::tempdir().unwrap();
    let bindir = root.path().join("bin");
    std::fs::create_dir_all(&bindir).unwrap();
    let exe = bindir.join("dosi-tracker");
    std::fs::write(&exe, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
    let units = root.path().join("units");
    std::fs::create_dir_all(&units).unwrap();
    for (name, text) in [
        ("dosi-tracker.service", render_unit(&bindir).unwrap()),
        (
            "managed/dosi-tracker.service",
            service::render_user_unit(&bindir).unwrap(),
        ),
    ] {
        let path = units.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text).unwrap();
        let output = run_tool(
            "systemd-analyze",
            &["verify", "--user", path.to_str().unwrap()],
            &xdg,
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{name}: {stderr}");
        assert!(
            !stderr.contains("dosi-tracker.service:"),
            "{name}: no complaints about the unit itself: {stderr}"
        );
    }
}

#[test]
fn the_rendered_autostart_entry_passes_desktop_file_validate() {
    let xdg = TestXdg::new();
    let dir = tempfile::tempdir().unwrap();
    for exe in ["/usr/bin/dosi-tracker", "/home/a b/.local/bin/dosi-tracker"] {
        let path = dir.path().join("io.github.dosibridge.DosiTracker.desktop");
        std::fs::write(&path, render_autostart(Path::new(exe)).unwrap()).unwrap();
        let output = run_tool("desktop-file-validate", &[path.to_str().unwrap()], &xdg);
        let all = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.status.success(), "{exe}: {all}");
        assert!(!all.contains("error"), "{exe}: {all}");
    }
}
