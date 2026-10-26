//! Durable small-file writes, shared by everything the agent saves: the
//! sign-in, the consent record, the device id and signing key, and the
//! preferences.

use std::io::Write;
use std::path::Path;

/// Write `data` to a fresh temporary sibling of `path`, flush it to disk, then
/// rename it over `path`.
///
/// On Windows, `std::fs::rename` uses `MoveFileExW(..., MOVEFILE_REPLACE_EXISTING)`,
/// replacing the file atomically on the same volume: a crash (or a full disk)
/// leaves either the old file or the new one, never a truncated mix. On any
/// error the temporary file is deleted and `path` is left as it was.
pub fn write_atomically(path: &Path, data: &[u8]) -> anyhow::Result<()> {
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let temp = parent.join(format!(".{name}.{}.tmp", uuid::Uuid::new_v4().simple()));

    let written = (|| -> std::io::Result<()> {
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&temp)?;
        file.write_all(data)?;
        file.sync_all()
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_file(&temp);
        return Err(anyhow::anyhow!("could not write {}: {e}", temp.display()));
    }
    if let Err(e) = std::fs::rename(&temp, path) {
        let _ = std::fs::remove_file(&temp);
        return Err(anyhow::anyhow!("could not replace {}: {e}", path.display()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("dosi-fsutil-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_file_is_created_then_replaced_without_leftovers() {
        let dir = temp_dir();
        let path = dir.join("nested").join("state.json");
        write_atomically(&path, b"one").unwrap();
        write_atomically(&path, b"two").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"two");
        let names: Vec<_> = std::fs::read_dir(path.parent().unwrap()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, vec![std::ffi::OsString::from("state.json")], "no temporary file is left behind");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_failed_replace_keeps_the_old_file_and_cleans_up() {
        let dir = temp_dir();
        // A directory cannot be replaced by a file.
        let path = dir.join("taken");
        std::fs::create_dir_all(path.join("inside")).unwrap();
        assert!(write_atomically(&path, b"data").is_err());
        assert!(path.join("inside").is_dir(), "what was there is untouched");
        let leftovers = std::fs::read_dir(&dir).unwrap().filter(|e| e.as_ref().unwrap().file_name() != "taken").count();
        assert_eq!(leftovers, 0, "the temporary file was removed");
        let _ = std::fs::remove_dir_all(dir);
    }
}
