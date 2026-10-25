//! Window and running application models and pure filename aggregation.
//!
//! Enforces:
//! - Full compatibility with backend `WindowInfoDto` and Windows `WindowInfo`.
//! - Maximum cap of `MAX_RUNNING_PROGRAMS = 200` per block (CONTRACT §1).
//! - Case-insensitive deduplication in first-seen order.
//! - Filtering of empty entries and Linux kernel worker threads (`[kworker/...]`).
//! - Privacy invariant: running programs are names only (window titles and metrics are empty/None).

use serde::{Deserialize, Serialize};

/// The server's cap on the running-programs list of one block.
pub const MAX_RUNNING_PROGRAMS: usize = 200;

/// A window (app + title). For foreground usage it also carries how long it
/// was in front during the block and the input made while it was; running
/// programs leave those out.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowInfo {
    pub app_name: String,
    pub window_title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyboard_hits: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mouse_clicks: Option<u64>,
}

/// The file name of an executable path (`/usr/bin/code` → `code`, `C:\...\Code.exe` → `Code.exe`).
pub fn file_name(path: &str) -> &str {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return "";
    }
    trimmed.rsplit(['\\', '/']).next().unwrap_or(trimmed)
}

/// Is this a Linux kernel worker or internal system thread rather than a user application?
/// In Linux `/proc`, kernel threads have no cmdline and their `comm` or `stat` is enclosed in brackets `[...]`.
pub fn is_kernel_thread(name: &str) -> bool {
    let trimmed = name.trim();
    trimmed.starts_with('[') && trimmed.ends_with(']')
}

/// Running programs by name only — never their window titles (privacy: the
/// titles of background windows are not the worker's focus and can hold
/// document names or e-mail subjects). One entry per executable, compared
/// case-insensitively, in first-seen order; unnamed entries and kernel threads
/// are dropped. At most [`MAX_RUNNING_PROGRAMS`].
pub fn program_list<I>(app_names: I) -> Vec<WindowInfo>
where
    I: IntoIterator<Item = String>,
{
    let mut seen = std::collections::HashSet::new();
    app_names
        .into_iter()
        .filter(|raw| !is_kernel_thread(raw))
        .map(|name| file_name(&name).trim().to_string())
        .filter(|name| !name.is_empty() && !is_kernel_thread(name))
        .filter(|name| seen.insert(name.to_lowercase()))
        .take(MAX_RUNNING_PROGRAMS)
        .map(|app_name| WindowInfo {
            app_name,
            window_title: String::new(),
            seconds: None,
            keyboard_hits: None,
            mouse_clicks: None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn executable_paths_become_clean_file_names() {
        assert_eq!(file_name("/usr/bin/code"), "code");
        assert_eq!(file_name("/usr/local/bin/slack"), "slack");
        assert_eq!(file_name(r"C:\Program Files\App\test.exe"), "test.exe");
        assert_eq!(file_name("firefox"), "firefox");
        assert_eq!(file_name(""), "");
        assert_eq!(file_name("  /bin/bash  "), "bash");
    }

    #[test]
    fn kernel_threads_are_identified() {
        assert!(is_kernel_thread("[kworker/0:0]"));
        assert!(is_kernel_thread("[migration/0]"));
        assert!(is_kernel_thread(" [rcu_sched] "));
        assert!(!is_kernel_thread("firefox"));
        assert!(!is_kernel_thread("[unclosed"));
        assert!(!is_kernel_thread("unopened]"));
    }

    #[test]
    fn running_programs_are_names_only_and_one_per_executable_case_insensitively() {
        let names = [
            "/usr/bin/code".to_string(),
            "google-chrome".to_string(),
            "Code".to_string(),
            "".to_string(),
            "   ".to_string(),
            "[kworker/0:1]".to_string(),
            "GOOGLE-CHROME".to_string(),
            "/usr/bin/slack".to_string(),
        ];

        let programs = program_list(names);
        let listed: Vec<&str> = programs.iter().map(|p| p.app_name.as_str()).collect();
        assert_eq!(listed, ["code", "google-chrome", "slack"]);
        assert!(programs.iter().all(|p| p.window_title.is_empty()));
        assert!(programs
            .iter()
            .all(|p| p.seconds.is_none() && p.keyboard_hits.is_none() && p.mouse_clicks.is_none()));
    }

    #[test]
    fn running_programs_respect_the_server_cap() {
        let programs = program_list((0..500).map(|i| format!("app{i}")));
        assert_eq!(programs.len(), MAX_RUNNING_PROGRAMS);
    }
}
