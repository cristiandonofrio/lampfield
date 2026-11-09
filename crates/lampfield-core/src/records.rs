//! The desktop app's session records: one JSON file per session.

use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    pub category: Option<String>,
    pub detail: Option<String>,
    pub needs_action: Option<String>,
    /// True when the summary belongs to the latest assistant message.
    pub current: bool,
}

/// What the session's own `wrap_up` call said about its last turn.
#[derive(Debug, Clone, PartialEq)]
pub struct WrapUp {
    /// The question it left for you. None when it said nothing is pending.
    pub needs_user: Option<String>,
}

/// Only the fields the board uses. The records also hold tool permissions and connector
/// config, which never leave `pick`.
#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    pub session_id: String,
    pub cli_session_id: String,
    pub cwd: String,
    pub origin_cwd: String,
    pub title: String,
    pub is_archived: bool,
    pub created_at: i64,
    pub summary: Option<Summary>,
    pub wrap_up: Option<WrapUp>,
    /// Id of the latest reply. Changes every turn, so a dismissal can expire with it.
    pub last_assistant: String,
    /// When a turn last started or ended. Doesn't move on every tool call, so ordering
    /// by it stays calm while sessions work. Falls back to when the session was made.
    pub last_activity: i64,
    /// When you last sent it a message. 0 if never.
    pub last_used: i64,
    /// When a turn last started or ended (lastTurnEdgeAt). For an idle session, when it finished.
    pub last_turn_edge: i64,
    /// When you last switched to this session in Claude. 0 if never.
    pub last_focused: i64,
}

fn text(raw: &Value, key: &str) -> Option<String> {
    raw.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string)
}

pub fn pick(raw: &Value) -> Option<Record> {
    let summary = raw.get("postTurnSummary").filter(|v| v.is_object()).map(|pts| {
        let for_id = raw.get("postTurnSummaryFor").and_then(Value::as_str);
        let last = raw.get("lastAssistantUuid").and_then(Value::as_str);
        Summary {
            category: text(pts, "status_category"),
            detail: text(pts, "status_detail"),
            needs_action: text(pts, "needs_action"),
            current: for_id.is_some() && for_id == last,
        }
    });
    let wrap_up = raw
        .get("turnWrapUp")
        .filter(|v| v.is_object())
        .map(|w| WrapUp { needs_user: text(w, "needsUser") });
    Some(Record {
        session_id: text(raw, "sessionId")?,
        cli_session_id: text(raw, "cliSessionId").unwrap_or_default(),
        cwd: text(raw, "cwd").unwrap_or_default(),
        origin_cwd: text(raw, "originCwd").unwrap_or_default(),
        title: text(raw, "title").unwrap_or_else(|| "Untitled session".into()),
        is_archived: raw.get("isArchived").and_then(Value::as_bool).unwrap_or(false),
        created_at: raw.get("createdAt").and_then(Value::as_i64).unwrap_or(0),
        summary,
        wrap_up,
        last_assistant: text(raw, "lastAssistantUuid").unwrap_or_default(),
        last_activity: ["lastActivityAt", "lastTurnEdgeAt", "createdAt"]
            .iter()
            .find_map(|k| raw.get(*k).and_then(Value::as_i64).filter(|&t| t > 0))
            .unwrap_or(0),
        last_used: raw.get("latestUserFrameAt").and_then(Value::as_i64).unwrap_or(0),
        last_turn_edge: raw.get("lastTurnEdgeAt").and_then(Value::as_i64).unwrap_or(0),
        last_focused: raw.get("lastFocusedAt").and_then(Value::as_i64).unwrap_or(0),
    })
}

pub fn modified_ms(meta: &fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as i64)
}

pub fn list_dir(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir).map_or_else(|_| Vec::new(), |it| it.flatten().map(|e| e.path()).collect())
}

/// `<root>/<org>/<account>/local_*.json`
fn record_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for org in list_dir(root) {
        for acct in list_dir(&org) {
            for f in list_dir(&acct) {
                let name = f.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if name.starts_with("local_") && name.ends_with(".json") {
                    out.push(f);
                }
            }
        }
    }
    out
}

#[derive(Default)]
pub struct RecordCache {
    files: HashMap<PathBuf, (i64, Record)>,
}

impl RecordCache {
    /// Re-parses only the files whose mtime moved. A file that fails to parse is
    /// probably mid-write: the last good copy stands until the next pass.
    pub fn read_all(&mut self, root: &Path) -> Vec<Record> {
        let files = record_files(root);
        let mut out = Vec::with_capacity(files.len());
        for f in &files {
            let Ok(meta) = fs::metadata(f) else { continue };
            let mtime = modified_ms(&meta);
            if self.files.get(f).is_none_or(|(m, _)| *m != mtime) {
                let parsed = fs::read(f)
                    .ok()
                    .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                    .and_then(|v| pick(&v));
                if let Some(rec) = parsed {
                    self.files.insert(f.clone(), (mtime, rec));
                }
            }
            if let Some((_, rec)) = self.files.get(f) {
                out.push(rec.clone());
            }
        }
        self.files.retain(|f, _| files.contains(f));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn picks_fields_and_defaults_the_title() {
        let r = pick(&json!({"sessionId": "local_1", "isArchived": true, "cwd": "/a"})).unwrap();
        assert_eq!(r.title, "Untitled session");
        assert!(r.is_archived);
        assert_eq!(r.cwd, "/a");
        assert!(r.summary.is_none());
    }

    #[test]
    fn summary_is_current_only_for_the_latest_message() {
        let raw = |f: &str| json!({"sessionId": "local_1", "lastAssistantUuid": "u2", "postTurnSummaryFor": f,
            "postTurnSummary": {"status_category": "blocked", "needs_action": "Approve it"}});
        assert!(pick(&raw("u2")).unwrap().summary.unwrap().current);
        let old = pick(&raw("u1")).unwrap().summary.unwrap();
        assert!(!old.current);
        assert_eq!(old.category.as_deref(), Some("blocked"));
    }

    #[test]
    fn reads_the_wrap_up_question_if_there_is_one() {
        let r = pick(&json!({"sessionId": "local_1", "turnWrapUp": {"description": "d", "needsUser": "Which one?"}})).unwrap();
        assert_eq!(r.wrap_up.unwrap().needs_user.as_deref(), Some("Which one?"));
        let r = pick(&json!({"sessionId": "local_1", "turnWrapUp": {"description": "d"}})).unwrap();
        assert_eq!(r.wrap_up, Some(WrapUp { needs_user: None }));
        let r = pick(&json!({"sessionId": "local_1", "turnWrapUp": null})).unwrap();
        assert!(r.wrap_up.is_none());
    }

    #[test]
    fn activity_falls_back_to_the_turn_edge_then_creation() {
        let at = |v| pick(&v).unwrap().last_activity;
        assert_eq!(at(json!({"sessionId": "l", "createdAt": 1, "lastTurnEdgeAt": 2, "lastActivityAt": 3})), 3);
        assert_eq!(at(json!({"sessionId": "l", "createdAt": 1, "lastTurnEdgeAt": 2})), 2);
        assert_eq!(at(json!({"sessionId": "l", "createdAt": 1})), 1);
        assert_eq!(pick(&json!({"sessionId": "l", "latestUserFrameAt": 9})).unwrap().last_used, 9);
        let r = pick(&json!({"sessionId": "l", "lastTurnEdgeAt": 7, "lastFocusedAt": 4})).unwrap();
        assert_eq!((r.last_turn_edge, r.last_focused), (7, 4));
    }

    #[test]
    fn a_record_without_an_id_is_skipped() {
        assert!(pick(&json!({"title": "x"})).is_none());
    }
}
