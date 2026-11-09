//! Reads the Claude desktop app's session data and turns it into groups of sessions.
//!
//! Everything here is read-only. Files are only ever opened for reading, and nothing
//! in this crate touches the network. The formats are internal to Claude and
//! undocumented, so an app update could change them.

pub mod describe;
pub mod groups;
pub mod paths;
pub mod records;
pub mod scan;
pub mod state;
pub mod transcript;

pub use paths::{default_paths, Paths};
pub use scan::Scanner;
pub use transcript::{FileChange, Question, Step};

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Working,
    Needs,
    Idle,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub id: String,
    pub title: String,
    pub state: State,
    /// Finer wording than the state, for example "Quiet" or "Stopped".
    pub label: String,
    pub doing: String,
    /// Subagents running right now.
    pub helpers: usize,
    /// The project folder, with any worktree folder stripped.
    pub folder: String,
    /// Milliseconds since 1970.
    pub created_at: i64,
    pub last_activity: i64,
    /// When you last sent it a message. Tiles sort by this, newest first, so a tile only
    /// moves when you do something with it. 0 if never.
    pub last_used: i64,
    /// When the last turn started or ended. A finish you've marked as seen is keyed on it.
    pub turn_end: i64,
    /// Finished a turn since you last opened it in Claude. See `is_unread`.
    pub unread: bool,
    pub question: Option<Question>,
    /// Most recently changed first.
    pub files: Vec<FileChange>,
    /// Oldest first.
    pub steps: Vec<Step>,
    /// See `state::Status::alert_key`. Stays in this process, the window doesn't need it.
    #[serde(skip)]
    pub alert_key: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub id: String,
    pub name: String,
    pub colour: String,
    pub folder: String,
    pub sessions: Vec<Session>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    /// "sidebar" or "folder".
    pub group_source: String,
    /// Why the fallback was used. Empty when the sidebar groups were read.
    pub group_note: String,
    pub groups: Vec<Group>,
    pub total_records: usize,
    pub archived_hidden: usize,
    /// Where it looked, shown when nothing was found.
    pub sessions_root: String,
}

/// What the tray shows.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Attention {
    /// Id and title of each session that needs you or has failed.
    pub urgent: Vec<(String, String)>,
    /// Sessions that may be waiting for approval.
    pub quiet: usize,
}

/// How long an unread finish stays unread.
pub const UNREAD_FOR_MS: i64 = 36 * 60 * 60 * 1000;

/// A session finished a turn you haven't looked at: it's idle (not a waved-off alert) and
/// its last turn ended after you last switched to it in Claude. Sessions never focused
/// don't count, and nothing stays unread past `UNREAD_FOR_MS`.
pub fn is_unread(state: State, label: &str, turn_end: i64, focused: i64, now: i64) -> bool {
    state == State::Idle && label != "Dismissed" && focused > 0 && turn_end > focused + 2_000 && now - turn_end < UNREAD_FOR_MS
}

pub fn attention(snapshot: &Snapshot) -> Attention {
    let mut out = Attention::default();
    for s in snapshot.groups.iter().flat_map(|g| &g.sessions) {
        match s.state {
            State::Needs | State::Failed => out.urgent.push((s.id.clone(), s.title.clone())),
            State::Working if s.label == "Quiet" => out.quiet += 1,
            _ => {}
        }
    }
    out
}

/// The key to remember when you dismiss a session's alert. None when it has no alert.
pub fn alert_to_dismiss(snapshot: &Snapshot, id: &str) -> Option<String> {
    snapshot
        .groups
        .iter()
        .flat_map(|g| &g.sessions)
        .find(|s| s.id == id && matches!(s.state, State::Needs | State::Failed) && !s.alert_key.is_empty())
        .map(|s| s.alert_key.clone())
}

/// Drops dismissals that no longer apply. Returns true if anything was dropped.
pub fn prune_dismissed(snapshot: &Snapshot, dismissed: &mut std::collections::HashMap<String, String>) -> bool {
    let still: std::collections::HashSet<&str> = snapshot
        .groups
        .iter()
        .flat_map(|g| &g.sessions)
        .filter(|s| s.label == "Dismissed")
        .map(|s| s.id.as_str())
        .collect();
    let before = dismissed.len();
    dismissed.retain(|id, _| still.contains(id.as_str()));
    dismissed.len() != before
}

/// The folder of the most recently active session that lives somewhere real.
pub fn group_folder(sessions: &[Session]) -> String {
    sessions
        .iter()
        .filter(|s| paths::is_absolute(&s.folder) && !paths::is_scratch(&s.folder))
        .max_by_key(|s| s.last_activity)
        .map(|s| s.folder.clone())
        .unwrap_or_default()
}

fn percent_encode(text: &str) -> String {
    let mut encoded = String::new();
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => encoded.push(byte as char),
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

/// A new Claude Code session with a prompt already typed, asking it to archive these
/// sessions. This app can't archive them itself: only Claude can change its sessions.
/// Takes (id, title) pairs. None if no id is a local session.
pub fn archive_link(sessions: &[(String, String)]) -> Option<String> {
    let lines: Vec<String> = sessions
        .iter()
        .filter(|(id, _)| session_link(id).is_some())
        .map(|(id, title)| format!("- {} ({id})", title.replace(['\n', '\r'], " ")))
        .collect();
    if lines.is_empty() {
        return None;
    }
    let prompt = format!(
        "Archive these Claude Code sessions. I hid them in Lampfield and I'm done with them. \
         Use your session tools to archive each one by its id, then tell me which were archived \
         and which, if any, couldn't be.\n\n{}",
        lines.join("\n")
    );
    Some(format!("claude://code/new?q={}", percent_encode(&prompt)))
}

/// The deep link the Claude app answers to. None unless the id looks like a local
/// session, because the result is handed to the system's link opener.
pub fn session_link(id: &str) -> Option<String> {
    let rest = id.strip_prefix("local_")?;
    let ok = !rest.is_empty() && rest.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    ok.then(|| format!("claude://claude.ai/epitaxy/{id}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn session(id: &str, state: State, label: &str, folder: &str) -> Session {
        Session {
            id: id.into(),
            title: format!("title {id}"),
            state,
            label: label.into(),
            doing: String::new(),
            helpers: 0,
            folder: folder.into(),
            created_at: 0,
            last_activity: 0,
            last_used: 0,
            turn_end: 0,
            unread: false,
            question: None,
            files: vec![],
            steps: vec![],
            alert_key: String::new(),
        }
    }

    fn snap(sessions: Vec<Session>) -> Snapshot {
        let g = Group { id: "g".into(), name: "g".into(), colour: String::new(), folder: String::new(), sessions };
        Snapshot {
            group_source: String::new(),
            group_note: String::new(),
            groups: vec![g],
            total_records: 0,
            archived_hidden: 0,
            sessions_root: String::new(),
        }
    }

    #[test]
    fn a_finish_is_unread_until_you_open_the_session() {
        let now = 100 * 60 * 60 * 1000;
        let hour = 60 * 60 * 1000;
        assert!(is_unread(State::Idle, "Idle", now - hour, now - 2 * hour, now));
        assert!(!is_unread(State::Idle, "Idle", now - 2 * hour, now - hour, now), "opened since");
        assert!(!is_unread(State::Working, "Working", now - hour, now - 2 * hour, now), "still busy");
        assert!(!is_unread(State::Idle, "Dismissed", now - hour, now - 2 * hour, now), "waved off");
        assert!(!is_unread(State::Idle, "Idle", now - hour, 0, now), "never focused");
        assert!(!is_unread(State::Idle, "Idle", now - 40 * hour, now - 41 * hour, now), "too old");
    }

    #[test]
    fn attention_counts_urgent_and_quiet_only() {
        let s = snap(vec![
            session("a", State::Needs, "", ""),
            session("b", State::Failed, "", ""),
            session("c", State::Working, "Quiet", ""),
            session("d", State::Working, "Working", ""),
            session("e", State::Idle, "", ""),
        ]);
        let got = attention(&s);
        assert_eq!(got.urgent.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(), ["a", "b"]);
        assert_eq!(got.quiet, 1);
    }

    #[test]
    fn only_sessions_with_an_alert_can_be_dismissed() {
        let mut a = session("a", State::Needs, "Needs you", "");
        a.alert_key = "reply:u1".into();
        let s = snap(vec![a, session("b", State::Idle, "Idle", "")]);
        assert_eq!(alert_to_dismiss(&s, "a").as_deref(), Some("reply:u1"));
        assert_eq!(alert_to_dismiss(&s, "b"), None);
    }

    #[test]
    fn dismissals_drop_once_the_session_moves_on() {
        let s = snap(vec![session("a", State::Idle, "Dismissed", ""), session("b", State::Needs, "Needs you", "")]);
        let mut d: std::collections::HashMap<String, String> =
            [("a", "k"), ("b", "old"), ("gone", "k")].iter().map(|(i, k)| (i.to_string(), k.to_string())).collect();
        assert!(prune_dismissed(&s, &mut d));
        assert_eq!(d.keys().collect::<Vec<_>>(), ["a"]);
    }

    #[test]
    fn group_folder_is_the_newest_real_folder_on_either_system() {
        let mut a = session("a", State::Idle, "", "C:\\old\\project");
        a.last_activity = 1;
        let mut b = session("b", State::Idle, "", "C:\\new\\project");
        b.last_activity = 3;
        let mut c = session("c", State::Idle, "", "C:\\x\\scratch-workspaces\\y");
        c.last_activity = 5;
        assert_eq!(group_folder(&[a.clone(), b, c.clone()]), "C:\\new\\project");
        assert_eq!(group_folder(&[c]), "");
        a.folder = "/mac/project".into();
        assert_eq!(group_folder(&[a]), "/mac/project");
    }

    #[test]
    fn archive_link_lists_local_sessions_only() {
        let pick = vec![
            ("local_a1".to_string(), "First & best".to_string()),
            ("session_x".to_string(), "Not local".to_string()),
        ];
        let link = archive_link(&pick).unwrap();
        assert!(link.starts_with("claude://code/new?q=Archive%20these"));
        assert!(link.contains("First%20%26%20best%20%28local_a1%29"));
        assert!(link.contains("in%20Lampfield"));
        assert!(!link.contains("session_x"));
        assert_eq!(archive_link(&pick[1..]), None);
    }

    #[test]
    fn link_accepts_local_ids_only() {
        assert_eq!(session_link("local_ab-12").unwrap(), "claude://claude.ai/epitaxy/local_ab-12");
        assert!(session_link("session_abc").is_none());
        assert!(session_link("local_").is_none());
        assert!(session_link("local_a/../b").is_none());
        assert!(session_link("local_a&calc").is_none());
    }
}
