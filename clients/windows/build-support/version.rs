//! The agent's version, as the build script stamps it into the program: the
//! `agentVersion` every block and heartbeat carries, and the numbers of the
//! Windows version resource.
//!
//! Shared by `build.rs` and the crate's tests (`#[path]`), so nothing here may
//! use the crate or a dependency.
//!
//! - The release version is `Cargo.toml`'s, or `DOSI_VERSION` when CI sets it
//!   (from a `v1.2.3` tag): `MAJOR.MINOR.PATCH` with an optional
//!   `-pre.release`, never build metadata of its own.
//! - The git commit (`DOSI_GIT_SHA`, else GitHub Actions' `GITHUB_SHA`) is
//!   appended as build metadata: `1.2.3+0123456789ab`, so support and the
//!   server can tell builds apart.
//! - The resource's fourth number is `DOSI_BUILD_NUMBER` (CI's run number),
//!   0 by default.

/// The longest `agentVersion` the server keeps (`MaxAgentVersionLength`).
pub const MAX_AGENT_VERSION_LEN: usize = 32;

/// Hex digits of the commit kept in the version (as `git log` shows them,
/// with room to spare).
const SHA_DIGITS: usize = 12;

/// The fewest hex digits that still name a commit.
const MIN_SHA_DIGITS: usize = 7;

/// The release version: `override_version` (CI's `DOSI_VERSION`, a leading
/// `v` allowed) when given and not blank, otherwise `cargo_version`.
pub fn release_version(cargo_version: &str, override_version: Option<&str>) -> Result<String, String> {
    let chosen = match override_version.map(str::trim).filter(|v| !v.is_empty()) {
        Some(tag) => tag.strip_prefix('v').unwrap_or(tag),
        None => cargo_version.trim(),
    };
    numeric_core(chosen)?;
    Ok(chosen.to_owned())
}

/// `MAJOR.MINOR.PATCH` of `version`, each within what a Windows version
/// resource holds (16 bits); the pre-release after `-` must be dot-separated
/// alphanumerics and hyphens, and build metadata (`+`) is not accepted here.
pub fn numeric_core(version: &str) -> Result<[u16; 3], String> {
    let (core, pre) = match version.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (version, None),
    };
    if version.contains('+') {
        return Err(format!("version {version:?} carries build metadata; the commit is added by the build"));
    }
    if let Some(pre) = pre {
        let valid = pre.split('.').all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
        if !valid {
            return Err(format!("version {version:?} has an invalid pre-release"));
        }
    }
    let parts: Vec<&str> = core.split('.').collect();
    let [major, minor, patch] = parts.as_slice() else {
        return Err(format!("version {version:?} is not MAJOR.MINOR.PATCH"));
    };
    let number = |part: &str| {
        let leading_zero = part.len() > 1 && part.starts_with('0');
        match part.parse::<u16>() {
            Ok(n) if !leading_zero && part.chars().all(|c| c.is_ascii_digit()) => Ok(n),
            _ => Err(format!("version {version:?}: {part:?} is not a number from 0 to 65535")),
        }
    };
    Ok([number(major)?, number(minor)?, number(patch)?])
}

/// The commit as build metadata: lower-case hex, cut to [`SHA_DIGITS`];
/// `None` when no commit was given (a blank one counts as none).
pub fn short_sha(sha: Option<&str>) -> Result<Option<String>, String> {
    let Some(sha) = sha.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    if sha.len() < MIN_SHA_DIGITS || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("{sha:?} is not a git commit id"));
    }
    Ok(Some(sha.chars().take(SHA_DIGITS).collect::<String>().to_ascii_lowercase()))
}

/// The `agentVersion`: the release version, plus `+<commit>` when known —
/// shortened to [`MIN_SHA_DIGITS`] if needed to stay within
/// [`MAX_AGENT_VERSION_LEN`].
pub fn agent_version(release: &str, sha: Option<&str>) -> Result<String, String> {
    let version = match sha {
        None => release.to_owned(),
        Some(sha) => {
            let full = format!("{release}+{sha}");
            if full.len() <= MAX_AGENT_VERSION_LEN {
                full
            } else {
                format!("{release}+{}", &sha[..MIN_SHA_DIGITS.min(sha.len())])
            }
        }
    };
    if version.len() > MAX_AGENT_VERSION_LEN {
        return Err(format!("agent version {version:?} is longer than the server keeps ({MAX_AGENT_VERSION_LEN})"));
    }
    Ok(version)
}

/// The build number (`DOSI_BUILD_NUMBER`), 0 when not given.
pub fn build_number(value: Option<&str>) -> Result<u16, String> {
    match value.map(str::trim).filter(|v| !v.is_empty()) {
        None => Ok(0),
        Some(v) => v.parse::<u16>().map_err(|_| format!("build number {v:?} is not a number from 0 to 65535")),
    }
}

/// The version resource's four numbers, packed as `VS_FIXEDFILEINFO` holds
/// them (16 bits each, most significant first).
pub fn packed([major, minor, patch]: [u16; 3], build: u16) -> u64 {
    (u64::from(major) << 48) | (u64::from(minor) << 32) | (u64::from(patch) << 16) | u64::from(build)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_release_version_is_cargos_unless_ci_names_one() {
        assert_eq!(release_version("0.1.0", None).unwrap(), "0.1.0");
        assert_eq!(release_version("0.1.0", Some("  ")).unwrap(), "0.1.0", "blank is not given");
        assert_eq!(release_version("0.1.0", Some("v1.4.2")).unwrap(), "1.4.2", "a tag's v is dropped");
        assert_eq!(release_version("0.1.0", Some("2.0.0-beta.1")).unwrap(), "2.0.0-beta.1");
    }

    #[test]
    fn versions_the_resource_cannot_hold_are_refused() {
        for bad in ["1.2", "1.2.3.4", "1.x.3", "70000.0.0", "01.2.3", "1.2.3+meta", "1.2.3-", "1.2.3-be ta", "-1.2.3", ""] {
            assert!(release_version("0.1.0", Some(bad)).is_err() || bad.trim().is_empty(), "{bad:?} was accepted");
        }
        assert!(release_version("1.2", None).is_err(), "Cargo.toml's version is checked too");
        assert_eq!(numeric_core("1.2.3-rc.1").unwrap(), [1, 2, 3]);
        assert_eq!(numeric_core("0.0.0").unwrap(), [0, 0, 0]);
    }

    #[test]
    fn the_commit_is_short_lower_case_hex() {
        assert_eq!(short_sha(None).unwrap(), None);
        assert_eq!(short_sha(Some(" ")).unwrap(), None);
        assert_eq!(short_sha(Some("0123456789ABCDEF0123456789abcdef01234567")).unwrap().as_deref(), Some("0123456789ab"));
        assert_eq!(short_sha(Some("abc1234")).unwrap().as_deref(), Some("abc1234"));
        assert!(short_sha(Some("abc12")).is_err(), "too short to name a commit");
        assert!(short_sha(Some("not-a-sha")).is_err());
    }

    #[test]
    fn the_agent_version_carries_the_commit_within_the_servers_limit() {
        assert_eq!(agent_version("0.1.0", None).unwrap(), "0.1.0");
        assert_eq!(agent_version("0.1.0", Some("0123456789ab")).unwrap(), "0.1.0+0123456789ab");
        // A long pre-release keeps a 7-digit commit rather than overflow.
        let long = "1.2.3-beta.10.nightly";
        assert_eq!(agent_version(long, Some("0123456789ab")).unwrap(), "1.2.3-beta.10.nightly+0123456");
        assert!(agent_version(long, Some("0123456789ab")).unwrap().len() <= MAX_AGENT_VERSION_LEN);
        assert!(agent_version("1.2.3-a-very-long-pre-release-tag", None).is_err());
    }

    #[test]
    fn the_build_number_and_packing_match_the_version_resource() {
        assert_eq!(build_number(None).unwrap(), 0);
        assert_eq!(build_number(Some("417")).unwrap(), 417);
        assert!(build_number(Some("70000")).is_err());
        assert!(build_number(Some("12a")).is_err());
        assert_eq!(packed([1, 2, 3], 4), 0x0001_0002_0003_0004);
        assert_eq!(packed([65535, 0, 0], 0), 0xFFFF_0000_0000_0000);
    }
}
