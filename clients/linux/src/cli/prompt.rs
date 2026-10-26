//! Interactive input: no-echo password prompt on /dev/tty, `--password-stdin`,
//! `--password-file` checks, numbered choices; secrets zeroized.
//!
//! Passwords never come from the command line: [`reject_password_argv`] runs
//! before anything else and refuses `--password`, `--password=...`, `-p` and
//! `-p...` with exit 2, without echoing what followed them.

use std::ffi::OsString;
use std::io::{BufRead, Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;

use zeroize::Zeroizing;

use super::output::CliError;
use crate::agent::state::Secret;
use crate::ipc::exit;

/// The longest password line accepted from stdin or a file.
pub const MAX_PASSWORD_BYTES: usize = 4096;

/// The refusal for a password on the command line.
pub const PASSWORD_ON_ARGV: &str = "passwords on the command line are visible to other users; use the prompt, --password-stdin or --password-file";

/// Refuse a password on the command line (exit 2). The offending value is
/// never echoed.
pub fn reject_password_argv(args: &[OsString]) -> Option<CliError> {
    for arg in args.iter().skip(1) {
        let Some(arg) = arg.to_str() else { continue };
        if arg == "--" {
            break;
        }
        let long = arg == "--password" || arg.starts_with("--password=");
        // No short option of this CLI is `p`, so any short cluster with a `p`
        // (`-p`, `-pSECRET`, `-qp`) is a password attempt.
        let short = arg.starts_with('-')
            && !arg.starts_with("--")
            && arg.len() > 1
            && arg[1..].contains('p');
        if long || short {
            return Some(CliError::usage(PASSWORD_ON_ARGV));
        }
    }
    None
}

/// Where the password comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PasswordSource {
    /// Ask on the terminal without echo.
    Prompt,
    /// The first line of stdin.
    Stdin,
    /// The first line of a private file.
    File(std::path::PathBuf),
}

/// Is there a terminal to ask on?
pub fn interactive() -> bool {
    // SAFETY: isatty only inspects the descriptor.
    let stdin_tty = unsafe { libc::isatty(0) } == 1;
    stdin_tty
        && std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/tty")
            .is_ok()
}

/// The first line of `reader`, at most [`MAX_PASSWORD_BYTES`].
pub fn first_line(reader: &mut impl Read) -> Result<Secret, CliError> {
    let mut bytes = Zeroizing::new(Vec::with_capacity(256));
    let mut byte = [0u8; 1];
    loop {
        match reader.read(&mut byte) {
            Ok(0) => break,
            Ok(_) if byte[0] == b'\n' => break,
            Ok(_) => {
                if bytes.len() >= MAX_PASSWORD_BYTES {
                    return Err(CliError::usage(format!(
                        "the password line is longer than {MAX_PASSWORD_BYTES} bytes"
                    )));
                }
                bytes.push(byte[0]);
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(CliError::failed(format!("cannot read the password: {e}"))),
        }
    }
    zeroize::Zeroize::zeroize(&mut byte);
    if bytes.last() == Some(&b'\r') {
        bytes.pop();
    }
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| CliError::usage("the password is not valid UTF-8"))?;
    if text.is_empty() {
        return Err(CliError::usage("the password is empty"));
    }
    Ok(Secret::new(text))
}

/// Check a `--password-file`: a regular file (not a symlink) of the caller,
/// mode 0600 or 0400.
pub fn check_password_file(path: &Path) -> Result<(), CliError> {
    let meta = std::fs::symlink_metadata(path)
        .map_err(|e| CliError::usage(format!("--password-file {}: {e}", path.display())))?;
    if !meta.file_type().is_file() {
        return Err(CliError::usage(format!(
            "--password-file {} is not a regular file",
            path.display()
        )));
    }
    // SAFETY: geteuid has no preconditions and cannot fail.
    if meta.uid() != unsafe { libc::geteuid() } {
        return Err(CliError::environment(format!(
            "--password-file {} belongs to another user",
            path.display()
        )));
    }
    let mode = meta.mode() & 0o777;
    if mode != 0o600 && mode != 0o400 {
        return Err(CliError::environment(format!(
            "--password-file {} has mode {mode:o}; it must be 0600 or 0400",
            path.display()
        ))
        .with_hint(format!("chmod 600 {}", path.display())));
    }
    Ok(())
}

/// Read the password from `source`.
pub fn read_password(source: &PasswordSource, prompt: &str) -> Result<Secret, CliError> {
    match source {
        PasswordSource::Stdin => first_line(&mut std::io::stdin().lock()),
        PasswordSource::File(path) => {
            check_password_file(path)?;
            let mut file = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(path)
                .map_err(|e| CliError::usage(format!("--password-file {}: {e}", path.display())))?;
            first_line(&mut file)
        }
        PasswordSource::Prompt => {
            if !interactive() {
                return Err(CliError::new(
                    "bad_request",
                    exit::USAGE,
                    "no terminal to ask for the password",
                )
                .with_hint("use --password-stdin or --password-file"));
            }
            let typed = Zeroizing::new(
                rpassword::prompt_password(prompt)
                    .map_err(|e| CliError::failed(format!("cannot read the password: {e}")))?,
            );
            if typed.is_empty() {
                return Err(CliError::usage("the password is empty"));
            }
            Ok(Secret::new(typed.as_str()))
        }
    }
}

fn tty() -> Option<std::fs::File> {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .ok()
}

/// Ask `question` on the terminal; `None` without one (or at end of input).
pub fn ask(question: &str) -> Option<String> {
    if !interactive() {
        return None;
    }
    let mut tty = tty()?;
    write!(tty, "{question}").ok()?;
    tty.flush().ok()?;
    let mut line = String::new();
    std::io::BufReader::new(tty).read_line(&mut line).ok()?;
    let line = line.trim().to_owned();
    (!line.is_empty()).then_some(line)
}

/// Offer numbered `items`; returns the chosen index.
pub fn choose(title: &str, items: &[String]) -> Option<usize> {
    if !interactive() || items.is_empty() {
        return None;
    }
    let mut tty = tty()?;
    writeln!(tty, "{title}").ok()?;
    for (i, item) in items.iter().enumerate() {
        writeln!(tty, "  {}. {item}", i + 1).ok()?;
    }
    for _ in 0..3 {
        let answer = ask(&format!("Choose 1-{}: ", items.len()))?;
        match answer.parse::<usize>() {
            Ok(n) if (1..=items.len()).contains(&n) => return Some(n - 1),
            _ => {
                let _ = writeln!(tty, "Please enter a number between 1 and {}.", items.len());
            }
        }
    }
    None
}

/// Ask a yes/no question (default no).
pub fn confirm(question: &str) -> bool {
    ask(&format!("{question} [y/N] "))
        .is_some_and(|a| matches!(a.to_ascii_lowercase().as_str(), "y" | "yes"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<OsString> {
        std::iter::once("dosi-tracker")
            .chain(list.iter().copied())
            .map(OsString::from)
            .collect()
    }

    #[test]
    fn passwords_on_the_command_line_are_refused_without_echo() {
        for attempt in [
            &["login", "--password", "hunter2-canary"][..],
            &["login", "--password=hunter2-canary"],
            &["login", "-p", "hunter2-canary"],
            &["login", "-phunter2-canary"],
            &["-q", "login", "-qp", "hunter2-canary"],
            &["--json", "status", "--password", "x"],
        ] {
            let error = reject_password_argv(&args(attempt)).expect("refused");
            assert_eq!(error.exit_code, 2, "{attempt:?}");
            assert!(!error.message.contains("hunter2"), "{}", error.message);
        }
        for fine in [
            &["login", "--password-stdin"][..],
            &["login", "--password-file", "/tmp/pw"],
            &["status", "-v", "-q"],
            &["task", "set", "--", "-p is fine after --"],
            &["logs", "-n", "5"],
        ] {
            assert!(reject_password_argv(&args(fine)).is_none(), "{fine:?}");
        }
    }

    #[test]
    fn the_first_line_is_read_and_bounded() {
        let pw = first_line(&mut "s3cret\nsecond line\n".as_bytes()).unwrap();
        assert_eq!(pw.expose(), "s3cret");
        let crlf = first_line(&mut "abc\r\n".as_bytes()).unwrap();
        assert_eq!(crlf.expose(), "abc");
        let no_newline = first_line(&mut "abc".as_bytes()).unwrap();
        assert_eq!(no_newline.expose(), "abc");
        assert_eq!(first_line(&mut "\n".as_bytes()).unwrap_err().exit_code, 2);
        let long = "x".repeat(MAX_PASSWORD_BYTES + 1);
        assert_eq!(first_line(&mut long.as_bytes()).unwrap_err().exit_code, 2);
        let exact = "x".repeat(MAX_PASSWORD_BYTES);
        assert_eq!(
            first_line(&mut exact.as_bytes()).unwrap().expose().len(),
            MAX_PASSWORD_BYTES
        );
    }

    #[test]
    fn password_files_must_be_private_regular_files_of_ours() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("pw");
        std::fs::write(&file, "pw\n").unwrap();
        for (mode, ok) in [(0o600, true), (0o400, true), (0o640, false), (0o644, false)] {
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(mode)).unwrap();
            assert_eq!(check_password_file(&file).is_ok(), ok, "{mode:o}");
        }
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(
            read_password(&PasswordSource::File(file.clone()), "")
                .unwrap()
                .expose(),
            "pw"
        );
        let link = temp.path().join("link");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        assert_eq!(check_password_file(&link).unwrap_err().exit_code, 2);
        assert_eq!(check_password_file(temp.path()).unwrap_err().exit_code, 2);
        assert_eq!(
            check_password_file(&temp.path().join("missing"))
                .unwrap_err()
                .exit_code,
            2
        );
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(check_password_file(&file).unwrap_err().exit_code, 9);
    }
}
