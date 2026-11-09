//! Puts records, transcripts and groups together into a snapshot for the window.

use crate::groups::{colour_for, project_folder, SidebarCache};
use crate::records::{list_dir, modified_ms, Record, RecordCache};
use crate::state::derive;
use crate::transcript::TranscriptCache;
use crate::paths::{base_name, Paths};
use crate::{group_folder, Group, Session, Snapshot, State};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

const HELPER_ACTIVE_MS: i64 = 90 * 1000;
const HELPER_SCAN_MS: i64 = 30 * 60 * 1000;

/// Claude Code names a project's transcript folder after its path, with every
/// character that isn't a letter or digit replaced by a dash.
fn encode_path(p: &str) -> String {
    p.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

pub struct Scanner {
    paths: Paths,
    records: RecordCache,
    transcripts: TranscriptCache,
    sidebar: SidebarCache,
    transcript_paths: HashMap<String, PathBuf>,
    /// Session id to the alert key you dismissed. Set by the caller, see `lib::dismiss`.
    pub dismissed: HashMap<String, String>,
}

impl Scanner {
    pub fn new(paths: Paths) -> Self {
        Scanner {
            paths,
            records: RecordCache::default(),
            transcripts: TranscriptCache::default(),
            sidebar: SidebarCache::default(),
            transcript_paths: HashMap::new(),
            dismissed: HashMap::new(),
        }
    }

    fn find_transcript(&mut self, rec: &Record) -> Option<PathBuf> {
        if rec.cli_session_id.is_empty() {
            return None;
        }
        if let Some(known) = self.transcript_paths.get(&rec.cli_session_id).filter(|p| p.exists()) {
            return Some(known.clone());
        }
        let name = format!("{}.jsonl", rec.cli_session_id);
        let root = &self.paths.transcripts_root;
        let found = [&rec.cwd, &rec.origin_cwd]
            .iter()
            .filter(|p| !p.is_empty())
            .map(|p| root.join(encode_path(p)).join(&name))
            .find(|p| p.exists())
            .or_else(|| list_dir(root).into_iter().map(|d| d.join(&name)).find(|p| p.exists()))?;
        self.transcript_paths.insert(rec.cli_session_id.clone(), found.clone());
        Some(found)
    }

    fn active_helpers(&mut self, transcript: &Path, now: i64) -> usize {
        let dir = transcript.with_extension("").join("subagents");
        let mut active = 0;
        for f in list_dir(&dir) {
            if f.extension().is_none_or(|e| e != "jsonl") {
                continue;
            }
            let Ok(meta) = fs::metadata(&f) else { continue };
            if now - modified_ms(&meta) > HELPER_SCAN_MS {
                continue;
            }
            if let Some(t) = self.transcripts.read(&f) {
                if t.turn_open && now - t.mtime_ms < HELPER_ACTIVE_MS {
                    active += 1;
                }
            }
        }
        active
    }

    pub fn snapshot(&mut self, now: i64) -> Snapshot {
        let all = self.records.read_all(&self.paths.sessions_root);
        let titles: HashMap<&str, &str> =
            all.iter().map(|r| (r.session_id.as_str(), r.title.as_str())).collect();
        let title_for = |id: &str| titles.get(id).map(|t| t.to_string());

        let mut live: Vec<&Record> = all.iter().filter(|r| !r.is_archived).collect();
        // Most recent first, like Claude's own list.
        live.sort_by_key(|r| std::cmp::Reverse(r.last_activity));

        let mut sessions = Vec::with_capacity(live.len());
        for rec in &live {
            let file = self.find_transcript(rec);
            let t = file.as_ref().and_then(|f| self.transcripts.read(f));
            let st = derive(rec, t, now, &title_for);
            let (files, steps) = t.map_or((vec![], vec![]), |t| (t.changed_files(), t.steps.iter().cloned().collect()));
            let helpers = file.as_ref().map_or(0, |f| self.active_helpers(f, now));
            // You waved this alert off and nothing new has happened since.
            let dismissed = !st.alert_key.is_empty() && self.dismissed.get(&rec.session_id) == Some(&st.alert_key);
            let (state, label) = if dismissed { (State::Idle, "Dismissed".to_string()) } else { (st.state, st.label) };
            let unread = crate::is_unread(state, &label, rec.last_turn_edge, rec.last_focused, now);
            sessions.push(Session {
                id: rec.session_id.clone(),
                title: rec.title.clone(),
                state,
                label,
                doing: st.doing,
                helpers,
                folder: project_folder(&rec.origin_cwd, &rec.cwd),
                created_at: rec.created_at,
                last_activity: rec.last_activity,
                last_used: rec.last_used,
                turn_end: rec.last_turn_edge,
                unread,
                question: if dismissed { None } else { st.question },
                files,
                steps,
                alert_key: st.alert_key,
            });
        }

        let (mut groups, group_source, group_note) = match self.sidebar.read(&self.paths.desktop_config) {
            Ok(sidebar) => {
                let mut by_group: HashMap<&str, Vec<Session>> = HashMap::new();
                let mut loose = Vec::new();
                for s in sessions {
                    let known = sidebar
                        .assignments
                        .get(&s.id)
                        .and_then(|g| sidebar.groups.iter().find(|x| x.id == *g));
                    match known {
                        Some(g) => by_group.entry(g.id.as_str()).or_default().push(s),
                        None => loose.push(s),
                    }
                }
                let mut groups: Vec<Group> = sidebar
                    .groups
                    .iter()
                    .map(|g| Group {
                        id: g.id.clone(),
                        name: g.name.clone(),
                        colour: colour_for(&g.id),
                        folder: String::new(),
                        sessions: by_group.remove(g.id.as_str()).unwrap_or_default(),
                    })
                    .collect();
                if !loose.is_empty() {
                    groups.push(Group {
                        id: "ungrouped".into(),
                        name: "Ungrouped".into(),
                        colour: "hsl(0, 0%, 55%)".into(),
                        folder: String::new(),
                        sessions: loose,
                    });
                }
                (groups, "sidebar", String::new())
            }
            Err(why) => (by_folder(&live, sessions), "folder", why),
        };

        for g in &mut groups {
            g.folder = group_folder(&g.sessions);
        }

        Snapshot {
            group_source: group_source.into(),
            group_note,
            groups,
            total_records: all.len(),
            archived_hidden: all.len() - live.len(),
            sessions_root: self.paths.sessions_root.display().to_string(),
        }
    }
}

fn by_folder(live: &[&Record], sessions: Vec<Session>) -> Vec<Group> {
    let mut folders: BTreeMap<String, Vec<Session>> = BTreeMap::new();
    for (rec, s) in live.iter().zip(sessions) {
        folders.entry(project_folder(&rec.origin_cwd, &rec.cwd)).or_default().push(s);
    }
    folders
        .into_iter()
        .map(|(path, sessions)| Group {
            name: base_name(&path).to_string(),
            colour: colour_for(&path),
            folder: String::new(),
            id: path,
            sessions,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::State;
    use serde_json::json;

    const NOW: i64 = 4_000_000_000_000;

    struct Fixture {
        _dir: tempfile::TempDir,
        paths: Paths,
    }

    /// Made-up data laid out the way the desktop app lays out the real thing.
    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let paths = Paths {
            sessions_root: root.join("sessions"),
            transcripts_root: root.join("projects"),
            desktop_config: root.join("config.json"),
        };
        let acct = paths.sessions_root.join("org/acct");
        fs::create_dir_all(&acct).unwrap();
        let rec = |id: &str, title: &str, archived: bool, created: i64| {
            let body = json!({"sessionId": id, "cliSessionId": format!("cli-{id}"), "cwd": "/made/up",
                "originCwd": "/made/up", "title": title, "isArchived": archived, "createdAt": created});
            fs::write(acct.join(format!("{id}.json")), body.to_string()).unwrap();
        };
        rec("local_a", "First", false, 1);
        rec("local_b", "Second", false, 2);
        rec("local_c", "Gone", true, 3);
        rec("local_d", "Loose", false, 4);
        fs::write(acct.join("deleted_x"), "{}").unwrap();

        let tdir = paths.transcripts_root.join("-made-up");
        fs::create_dir_all(&tdir).unwrap();
        let open = json!({"type": "user", "message": {"content": "Go"}}).to_string();
        fs::write(tdir.join("cli-local_a.jsonl"), format!("{open}\n")).unwrap();
        let helper_dir = tdir.join("cli-local_a/subagents");
        fs::create_dir_all(&helper_dir).unwrap();
        fs::write(helper_dir.join("agent-1.jsonl"), format!("{open}\n")).unwrap();

        fs::write(
            &paths.desktop_config,
            json!({"preferences": {"epitaxyPrefs": {
                "dframe-code-sections": {"o/a": {"sections": [
                    {"id": "cg-1", "kind": "manual", "name": "One", "order": 1},
                    {"id": "cg-2", "kind": "manual", "name": "Empty", "order": 2}]}},
                "dframe-group-scopes": {"o/a": {"assignments": {
                    "code:local_a": "cg-1", "code:local_b": "cg-1", "code:local_c": "cg-1"}}}
            }}})
            .to_string(),
        )
        .unwrap();
        Fixture { _dir: dir, paths }
    }

    fn now() -> i64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
    }

    #[test]
    fn groups_follow_the_sidebar_and_hide_archived_sessions() {
        let f = fixture();
        let snap = Scanner::new(f.paths.clone()).snapshot(now());
        assert_eq!(snap.group_source, "sidebar");
        assert_eq!(snap.total_records, 4);
        assert_eq!(snap.archived_hidden, 1);
        let names: Vec<_> = snap.groups.iter().map(|h| h.name.as_str()).collect();
        assert_eq!(names, ["One", "Empty", "Ungrouped"]);
        // No activity times in the fixture, so newest made comes first.
        let one: Vec<_> = snap.groups[0].sessions.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(one, ["Second", "First"]);
        assert!(snap.groups[1].sessions.is_empty());
        assert_eq!(snap.groups[2].sessions[0].title, "Loose");
    }

    #[test]
    fn states_and_helpers_come_from_the_transcripts() {
        let f = fixture();
        let snap = Scanner::new(f.paths.clone()).snapshot(now());
        let first = snap.groups[0].sessions.iter().find(|s| s.title == "First").unwrap();
        assert_eq!(first.state, State::Working);
        assert_eq!(first.helpers, 1);
        let second = snap.groups[0].sessions.iter().find(|s| s.title == "Second").unwrap();
        assert_eq!((second.state, second.label.as_str()), (State::Idle, "No log found"));
    }

    #[test]
    fn the_most_recently_active_session_comes_first() {
        let f = fixture();
        let acct = f.paths.sessions_root.join("org/acct");
        let body = json!({"sessionId": "local_a", "cliSessionId": "cli-local_a", "cwd": "/made/up",
            "title": "First", "createdAt": 1, "lastActivityAt": 99});
        fs::write(acct.join("local_a.json"), body.to_string()).unwrap();
        let snap = Scanner::new(f.paths.clone()).snapshot(now());
        let one: Vec<_> = snap.groups[0].sessions.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(one, ["First", "Second"]);
    }

    #[test]
    fn a_dismissed_alert_rests_until_the_next_reply() {
        let f = fixture();
        let acct = f.paths.sessions_root.join("org/acct");
        let blocked = |reply: &str| {
            let body = json!({"sessionId": "local_e", "cliSessionId": "cli-local_e", "cwd": "/made/up",
                "title": "Asks", "createdAt": 5, "lastAssistantUuid": reply, "postTurnSummaryFor": reply,
                "postTurnSummary": {"status_category": "blocked", "needs_action": "Pick one"}});
            fs::write(acct.join("local_e.json"), body.to_string()).unwrap();
        };
        let tdir = f.paths.transcripts_root.join("-made-up");
        let turn = [
            json!({"type": "user", "message": {"content": "Go"}}),
            json!({"type": "assistant", "message": {"stop_reason": "end_turn", "content": [{"type": "text", "text": "Pick one?"}]}}),
        ];
        let lines: Vec<String> = turn.iter().map(|v| v.to_string()).collect();
        fs::write(tdir.join("cli-local_e.jsonl"), lines.join("\n") + "\n").unwrap();

        blocked("u1");
        let mut s = Scanner::new(f.paths.clone());
        let find = |snap: &Snapshot| snap.groups.iter().flat_map(|h| &h.sessions).find(|x| x.id == "local_e").unwrap().clone();
        let snap = s.snapshot(now());
        assert_eq!(find(&snap).state, State::Needs);

        s.dismissed.insert("local_e".into(), crate::alert_to_dismiss(&snap, "local_e").unwrap());
        let e = find(&s.snapshot(now()));
        assert_eq!((e.state, e.label.as_str()), (State::Idle, "Dismissed"));

        // Records are re-read on mtime, so make sure the rewrite lands on a new one.
        std::thread::sleep(std::time::Duration::from_millis(20));
        blocked("u2");
        assert_eq!(find(&s.snapshot(now())).state, State::Needs);
    }

    #[test]
    fn stale_helpers_and_turns_drop_off_with_time() {
        let f = fixture();
        let mut s = Scanner::new(f.paths.clone());
        s.snapshot(now());
        let first = s.snapshot(NOW).groups[0].sessions.iter().find(|s| s.title == "First").unwrap().clone();
        assert_eq!((first.state, first.helpers), (State::Idle, 0));
    }

    #[test]
    fn falls_back_to_project_folders_and_says_why() {
        let f = fixture();
        fs::write(&f.paths.desktop_config, "{}").unwrap();
        let snap = Scanner::new(f.paths.clone()).snapshot(now());
        assert_eq!(snap.group_source, "folder");
        assert!(!snap.group_note.is_empty());
        assert_eq!(snap.groups.len(), 1);
        assert_eq!(snap.groups[0].name, "up");
        assert_eq!(snap.groups[0].sessions.len(), 3);
    }

    #[test]
    fn missing_folders_give_an_empty_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        let paths = Paths { sessions_root: p.join("x"), transcripts_root: p.join("y"), desktop_config: p.join("z") };
        let snap = Scanner::new(paths).snapshot(now());
        assert_eq!(snap.total_records, 0);
        assert_eq!(snap.group_source, "folder");
    }

    #[test]
    fn paths_encode_like_claude_code() {
        assert_eq!(encode_path("/Users/a b/c.d"), "-Users-a-b-c-d");
    }
}
