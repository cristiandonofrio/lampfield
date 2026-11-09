//! Sidebar groups. The desktop app mirrors them into claude_desktop_config.json under
//! preferences.epitaxyPrefs, in two keys:
//!   dframe-code-sections: every section with its name, order and visibility
//!   dframe-group-scopes:  which session sits in which group, as "code:<sessionId>"
//! Each is keyed by scope ("<org>/<account>"). Only these keys are deserialised, so the
//! rest of that file, which holds connector config, is never kept.
//!
//! No colour is stored for a group anywhere, so `colour_for` makes one from its id.

use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[derive(Deserialize)]
struct Config {
    preferences: Prefs,
}

#[derive(Deserialize)]
struct Prefs {
    #[serde(rename = "epitaxyPrefs")]
    epitaxy: Epitaxy,
}

#[derive(Deserialize)]
struct Epitaxy {
    #[serde(rename = "dframe-code-sections")]
    sections: HashMap<String, Sections>,
    #[serde(rename = "dframe-group-scopes")]
    scopes: HashMap<String, Scope>,
}

#[derive(Deserialize)]
struct Sections {
    sections: Vec<Section>,
}

#[derive(Deserialize)]
struct Section {
    id: String,
    kind: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    order: i64,
    #[serde(default = "yes")]
    shown: bool,
}

fn yes() -> bool {
    true
}

#[derive(Deserialize)]
struct Scope {
    #[serde(default)]
    assignments: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Sidebar {
    /// In sidebar order.
    pub groups: Vec<Group>,
    /// Session id to group id.
    pub assignments: HashMap<String, String>,
}

pub fn parse(bytes: &[u8]) -> Result<Sidebar, String> {
    let config: Config =
        serde_json::from_slice(bytes).map_err(|e| format!("group keys not found or unreadable: {e}"))?;
    let e = config.preferences.epitaxy;
    let mut sections: Vec<Section> = e.sections.into_values().flat_map(|s| s.sections).collect();
    sections.retain(|s| s.kind == "manual" && s.shown);
    if sections.is_empty() {
        return Err("no custom sidebar groups listed".into());
    }
    sections.sort_by_key(|s| s.order);
    let groups = sections.into_iter().map(|s| Group { id: s.id, name: s.name }).collect();
    let assignments = e
        .scopes
        .into_values()
        .flat_map(|s| s.assignments)
        .filter_map(|(k, g)| k.strip_prefix("code:").map(|id| (id.to_string(), g)))
        .collect();
    Ok(Sidebar { groups, assignments })
}

#[derive(Default)]
pub struct SidebarCache {
    mtime: Option<i64>,
    good: Option<Sidebar>,
    error: String,
}

impl SidebarCache {
    /// The latest sidebar, or why there isn't one. A bad read after a good one keeps
    /// the good copy, since the app may be halfway through writing the file.
    pub fn read(&mut self, file: &Path) -> Result<&Sidebar, String> {
        let mtime = fs::metadata(file).ok().map(|m| crate::records::modified_ms(&m));
        if mtime.is_none() {
            self.error = format!("{} is missing", file.display());
        } else if mtime != self.mtime {
            match fs::read(file).map_err(|e| e.to_string()).and_then(|b| parse(&b)) {
                Ok(s) => {
                    self.good = Some(s);
                    self.mtime = mtime;
                }
                Err(e) => self.error = e,
            }
        }
        self.good.as_ref().ok_or_else(|| self.error.clone())
    }
}

/// Stable across runs and machines, unlike the standard library's hasher.
pub fn colour_for(id: &str) -> String {
    let hash = id.bytes().fold(0x811c9dc5u32, |h, b| (h ^ b as u32).wrapping_mul(0x01000193));
    format!("hsl({}, 55%, 50%)", hash % 360)
}

/// Fallback when groups can't be read: the project a session belongs to, with any
/// worktree folder stripped so a worktree joins its parent project.
pub fn project_folder(origin_cwd: &str, cwd: &str) -> String {
    let path = if origin_cwd.is_empty() { cwd } else { origin_cwd };
    crate::paths::strip_worktree(path).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn config() -> Vec<u8> {
        json!({"preferences": {"epitaxyPrefs": {
            "dframe-code-sections": {"org/acct": {"sections": [
                {"id": "pinned", "kind": "pinned", "name": "", "order": 0},
                {"id": "cg-b", "kind": "manual", "name": "Beta", "order": 3},
                {"id": "cg-a", "kind": "manual", "name": "Alpha", "order": 2},
                {"id": "cg-h", "kind": "manual", "name": "Hidden", "order": 4, "shown": false},
                {"id": "sessions", "kind": "sessions", "name": "", "order": 5}
            ]}},
            "dframe-group-scopes": {"org/acct": {
                "groups": [], "order": {},
                "assignments": {"code:local_1": "cg-a", "code:session_x": "cg-b", "other:local_2": "cg-a"}
            }}
        }}, "unrelated": {"token": "made-up"}})
        .to_string()
        .into_bytes()
    }

    #[test]
    fn reads_custom_groups_in_sidebar_order() {
        let s = parse(&config()).unwrap();
        let names: Vec<_> = s.groups.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, ["Alpha", "Beta"]);
        assert_eq!(s.assignments["local_1"], "cg-a");
        assert_eq!(s.assignments["session_x"], "cg-b");
        assert!(!s.assignments.contains_key("local_2"));
    }

    #[test]
    fn missing_keys_are_an_error_not_a_panic() {
        assert!(parse(b"{}").is_err());
        assert!(parse(b"not json").is_err());
    }

    #[test]
    fn cache_keeps_the_last_good_copy_through_a_bad_write() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("c.json");
        let mut cache = SidebarCache::default();
        assert!(cache.read(&f).unwrap_err().contains("missing"));
        fs::write(&f, config()).unwrap();
        assert_eq!(cache.read(&f).unwrap().groups.len(), 2);
        fs::write(&f, b"{ half written").unwrap();
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
        File::options().write(true).open(&f).unwrap().set_modified(later).unwrap();
        assert_eq!(cache.read(&f).unwrap().groups.len(), 2);
    }
    use std::fs::File;

    #[test]
    fn colours_are_stable_and_differ_by_id() {
        assert_eq!(colour_for("cg-a"), colour_for("cg-a"));
        assert_ne!(colour_for("cg-a"), colour_for("cg-b"));
    }

    #[test]
    fn worktrees_join_their_project() {
        assert_eq!(project_folder("/p/app/.claude/worktrees/x", "/q"), "/p/app");
        assert_eq!(project_folder("", "/q/app"), "/q/app");
        assert_eq!(project_folder("C:\\p\\app\\.claude\\worktrees\\x", ""), "C:\\p\\app");
    }
}
