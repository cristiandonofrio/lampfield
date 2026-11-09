//! Path helpers that work on macOS and Windows. Session records store folders as plain
//! strings in the style of the machine that wrote them, so these work on strings rather
//! than `std::path`, which only understands the platform it was built for.

use std::path::PathBuf;

/// True for `/Users/me/x`, `C:\Users\me\x`, `C:/Users/me/x` and `\\server\share\x`.
pub fn is_absolute(p: &str) -> bool {
    let b = p.as_bytes();
    p.starts_with('/')
        || p.starts_with("\\\\")
        || (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/'))
}

/// The last part of a path, whichever separator it uses.
pub fn base_name(p: &str) -> &str {
    let trimmed = p.trim_end_matches(['/', '\\']);
    trimmed.rsplit(['/', '\\']).next().filter(|s| !s.is_empty()).unwrap_or(trimmed)
}

/// Cuts a worktree folder back to the project it belongs to.
pub fn strip_worktree(p: &str) -> &str {
    for marker in ["/.claude/worktrees/", "\\.claude\\worktrees\\"] {
        if let Some(i) = p.find(marker) {
            return &p[..i];
        }
    }
    p
}

/// Throwaway folders the Claude app makes for sessions with no project.
pub fn is_scratch(p: &str) -> bool {
    p.contains("/scratch-workspaces/") || p.contains("\\scratch-workspaces\\")
}

/// Where Claude's files live on this machine.
#[derive(Debug, Clone)]
pub struct Paths {
    /// One JSON record per session: `<root>/<org>/<account>/local_*.json`.
    pub sessions_root: PathBuf,
    /// Claude Code transcripts: `<root>/<encoded project path>/<cli session id>.jsonl`.
    pub transcripts_root: PathBuf,
    /// Holds the sidebar groups.
    pub desktop_config: PathBuf,
}

fn env_path(key: &str) -> Option<PathBuf> {
    std::env::var_os(key).filter(|v| !v.is_empty()).map(PathBuf::from)
}

fn home() -> Option<PathBuf> {
    env_path("HOME").or_else(|| env_path("USERPROFILE"))
}

/// The Claude desktop app's data folder. On Windows this is `%APPDATA%\Claude`, the same
/// place its documented `claude_desktop_config.json` lives. The Microsoft Store build keeps
/// it inside its package folder instead, so that is tried when the first one is missing.
pub fn claude_app_dir() -> Option<PathBuf> {
    if let Some(dir) = env_path("LAMPFIELD_CLAUDE_DIR") {
        return Some(dir);
    }
    #[cfg(target_os = "macos")]
    {
        home().map(|h| h.join("Library/Application Support/Claude"))
    }
    #[cfg(target_os = "windows")]
    {
        let roaming = env_path("APPDATA").map(|a| a.join("Claude"));
        if roaming.as_ref().is_some_and(|d| d.exists()) {
            return roaming;
        }
        let packaged = env_path("LOCALAPPDATA").and_then(|local| {
            std::fs::read_dir(local.join("Packages")).ok()?.flatten().map(|e| e.path()).find_map(|pkg| {
                let name = pkg.file_name()?.to_string_lossy().to_lowercase();
                let dir = pkg.join("LocalCache").join("Roaming").join("Claude");
                (name.contains("claude") && dir.exists()).then_some(dir)
            })
        });
        packaged.or(roaming)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        env_path("XDG_CONFIG_HOME").or_else(|| home().map(|h| h.join(".config"))).map(|c| c.join("Claude"))
    }
}

/// Claude Code's own folder, `~/.claude`, unless CLAUDE_CONFIG_DIR moves it.
pub fn claude_code_dir() -> Option<PathBuf> {
    env_path("CLAUDE_CONFIG_DIR").or_else(|| home().map(|h| h.join(".claude")))
}

pub fn default_paths() -> Option<Paths> {
    // For trying the app on made-up data: a folder holding sessions/, projects/ and config.json.
    if let Some(root) = env_path("LAMPFIELD_DATA") {
        return Some(Paths {
            sessions_root: root.join("sessions"),
            transcripts_root: root.join("projects"),
            desktop_config: root.join("config.json"),
        });
    }
    let app = claude_app_dir()?;
    Some(Paths {
        sessions_root: app.join("claude-code-sessions"),
        transcripts_root: claude_code_dir()?.join("projects"),
        desktop_config: app.join("claude_desktop_config.json"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute_paths_on_both_systems() {
        for p in ["/Users/me/x", "C:\\Users\\me\\x", "c:/Users/me", "\\\\server\\share\\x"] {
            assert!(is_absolute(p), "{p}");
        }
        for p in ["", "relative\\x", "relative/x", "C:", "C:relative"] {
            assert!(!is_absolute(p), "{p}");
        }
    }

    #[test]
    fn base_names_with_either_separator() {
        assert_eq!(base_name("/Users/me/portal"), "portal");
        assert_eq!(base_name("C:\\Users\\me\\portal\\"), "portal");
        assert_eq!(base_name("C:\\code/mixed\\name.ts"), "name.ts");
        assert_eq!(base_name("plain"), "plain");
    }

    #[test]
    fn worktrees_and_scratch_folders_on_both_systems() {
        assert_eq!(strip_worktree("/p/app/.claude/worktrees/x"), "/p/app");
        assert_eq!(strip_worktree("C:\\p\\app\\.claude\\worktrees\\x"), "C:\\p\\app");
        assert_eq!(strip_worktree("C:\\p\\app"), "C:\\p\\app");
        assert!(is_scratch("/x/scratch-workspaces/y"));
        assert!(is_scratch("C:\\x\\scratch-workspaces\\y"));
        assert!(!is_scratch("C:\\x\\work"));
    }
}
