//! Incremental reader for Claude Code's JSONL transcripts, keeping only what the board shows:
//! the turn state, open tool calls, the files changed and the last few steps.

use crate::describe::{describe_tool, first_line};
use crate::records::modified_ms;
use serde_json::Value;
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

#[derive(Debug, Clone, PartialEq)]
pub struct Pending {
    pub id: String,
    pub name: String,
    pub doing: String,
    /// Target of a SendMessage, so its title can be looked up later.
    pub to: Option<String>,
    /// For AskUserQuestion: the first question and its answers.
    pub question: Option<Question>,
    pub ts: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Question {
    pub text: String,
    pub options: Vec<String>,
}

/// One file the session changed, with lines added and removed across the whole session.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FileChange {
    pub path: String,
    pub name: String,
    pub added: usize,
    pub removed: usize,
    /// When it was last changed, so the newest sit first.
    #[serde(skip)]
    pub at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Step {
    pub doing: String,
    pub at: i64,
}

/// How many steps the panel keeps.
pub const STEPS_KEPT: usize = 12;

#[derive(Debug, Default, Clone)]
pub struct Transcript {
    pub(crate) offset: u64,
    pub(crate) partial: Vec<u8>,
    pub mtime_ms: i64,
    pub turn_open: bool,
    pub turn_started_at: i64,
    pub turn_ended_at: i64,
    pub last_assistant_at: i64,
    pub last_error_at: i64,
    /// Open tool calls in the order they started.
    pub pending: Vec<Pending>,
    pub last_text: String,
    pub files: HashMap<String, FileChange>,
    /// The latest tool calls, oldest first.
    pub steps: VecDeque<Step>,
}

fn parse_ts(o: &Value) -> i64 {
    o.get("timestamp")
        .and_then(Value::as_str)
        .and_then(|s| OffsetDateTime::parse(s, &Rfc3339).ok())
        .map_or(0, |t| (t.unix_timestamp_nanos() / 1_000_000) as i64)
}

impl Transcript {
    pub fn last_pending(&self) -> Option<&Pending> {
        self.pending.last()
    }

    /// Changed files, most recently changed first.
    pub fn changed_files(&self) -> Vec<FileChange> {
        let mut out: Vec<FileChange> = self.files.values().cloned().collect();
        out.sort_by(|a, b| b.at.cmp(&a.at).then_with(|| a.path.cmp(&b.path)));
        out
    }

    /// Edit and Write results carry the change itself: a patch of +/- lines for an edit,
    /// the whole content for a new file. Counting those gives real line counts.
    fn apply_file_result(&mut self, r: &Value, ts: i64) {
        let Some(path) = r.get("filePath").and_then(Value::as_str).filter(|p| !p.is_empty()) else { return };
        let (mut added, mut removed) = (0, 0);
        let hunks = r.get("structuredPatch").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]);
        for line in hunks.iter().flat_map(|h| h.get("lines").and_then(Value::as_array).into_iter().flatten()) {
            match line.as_str().and_then(|l| l.chars().next()) {
                Some('+') => added += 1,
                Some('-') => removed += 1,
                _ => {}
            }
        }
        if hunks.is_empty() && r.get("type").and_then(Value::as_str) == Some("create") {
            let content = r.get("content").and_then(Value::as_str).unwrap_or("");
            added = content.lines().count();
        }
        let entry = self.files.entry(path.to_string()).or_insert_with(|| FileChange {
            path: path.to_string(),
            name: crate::paths::base_name(path).to_string(),
            added: 0,
            removed: 0,
            at: 0,
        });
        entry.added += added;
        entry.removed += removed;
        entry.at = ts.max(entry.at);
    }

    pub fn apply_line(&mut self, line: &[u8]) {
        let Ok(o) = serde_json::from_slice::<Value>(line) else { return };
        let ts = parse_ts(&o);
        match o.get("type").and_then(Value::as_str) {
            Some("assistant") => self.apply_assistant(&o, ts),
            Some("user") => self.apply_user(&o, ts),
            Some("system") if o.get("subtype").and_then(Value::as_str) == Some("api_error") => {
                self.last_error_at = ts;
            }
            _ => {}
        }
    }

    fn apply_assistant(&mut self, o: &Value, ts: i64) {
        let m = o.get("message").unwrap_or(&Value::Null);
        for b in m.get("content").and_then(Value::as_array).into_iter().flatten() {
            match b.get("type").and_then(Value::as_str) {
                Some("tool_use") => {
                    let id = b.get("id").and_then(Value::as_str).unwrap_or("").to_string();
                    let name = b.get("name").and_then(Value::as_str).unwrap_or("").to_string();
                    let input = b.get("input").unwrap_or(&Value::Null);
                    let to = (name == "SendMessage")
                        .then(|| input.get("to").and_then(Value::as_str).map(str::to_string))
                        .flatten();
                    let question = (name == "AskUserQuestion").then(|| question_of(input)).flatten();
                    let doing = describe_tool(&name, input);
                    if !matches!(name.as_str(), "ToolSearch" | "TodoWrite") && !name.starts_with("mcp__ccd_turn") {
                        self.steps.push_back(Step { doing: doing.clone(), at: ts });
                        while self.steps.len() > STEPS_KEPT {
                            self.steps.pop_front();
                        }
                    }
                    self.pending.retain(|p| p.id != id);
                    self.pending.push(Pending { doing, id, name, to, question, ts });
                }
                Some("text") => {
                    let line = first_line(b.get("text").and_then(Value::as_str).unwrap_or(""));
                    if !line.is_empty() {
                        self.last_text = line;
                    }
                }
                _ => {}
            }
        }
        if ts != 0 {
            self.last_assistant_at = ts;
        }
        match m.get("stop_reason").and_then(Value::as_str) {
            Some("tool_use") if !self.turn_open => {
                self.turn_open = true;
                self.turn_started_at = ts;
            }
            Some("end_turn") => self.end_turn(ts),
            _ => {}
        }
    }

    fn apply_user(&mut self, o: &Value, ts: i64) {
        let c = o.pointer("/message/content").unwrap_or(&Value::Null);
        if let Some(blocks) = c.as_array() {
            let results: Vec<_> =
                blocks.iter().filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_result")).collect();
            if !results.is_empty() {
                if let Some(r) = o.get("toolUseResult").filter(|r| r.is_object()) {
                    self.apply_file_result(r, ts);
                }
                for b in results {
                    let id = b.get("tool_use_id").and_then(Value::as_str).unwrap_or("");
                    self.pending.retain(|p| p.id != id);
                }
                return;
            }
        }
        let interrupted = match c {
            Value::String(s) => s.starts_with("[Request interrupted"),
            Value::Array(blocks) => blocks
                .iter()
                .find(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                .and_then(|b| b.get("text").and_then(Value::as_str))
                .is_some_and(|t| t.starts_with("[Request interrupted")),
            _ => false,
        };
        if interrupted {
            self.end_turn(ts);
            return;
        }
        let peer = o.pointer("/origin/kind").and_then(Value::as_str) == Some("peer");
        if !peer && o.get("isMeta").and_then(Value::as_bool).unwrap_or(false) {
            return;
        }
        self.turn_open = true;
        self.turn_started_at = ts;
    }

    fn end_turn(&mut self, ts: i64) {
        self.turn_open = false;
        self.turn_ended_at = ts;
        self.pending.clear();
    }
}

fn question_of(input: &Value) -> Option<Question> {
    let q = input.pointer("/questions/0")?;
    let text = q.get("question").and_then(Value::as_str)?.to_string();
    let options = q
        .get("options")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|o| o.get("label").and_then(Value::as_str).map(str::to_string))
        .collect();
    Some(Question { text, options })
}

#[derive(Default)]
pub struct TranscriptCache {
    files: HashMap<PathBuf, Transcript>,
}

impl TranscriptCache {
    /// Reads only the bytes added since the last pass. Splits on newline bytes, so a
    /// half-written line waits for the next pass.
    pub fn read(&mut self, file: &Path) -> Option<&Transcript> {
        let meta = fs::metadata(file).ok()?;
        let size = meta.len();
        let t = self.files.entry(file.to_path_buf()).or_default();
        if size < t.offset {
            *t = Transcript::default();
        }
        if size > t.offset {
            let mut f = File::open(file).ok()?;
            f.seek(SeekFrom::Start(t.offset)).ok()?;
            let mut buf = std::mem::take(&mut t.partial);
            let n = f.read_to_end(&mut buf).ok()?;
            t.offset += n as u64;
            match buf.iter().rposition(|b| *b == b'\n') {
                Some(cut) => {
                    for line in buf[..cut].split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
                        t.apply_line(line);
                    }
                    t.partial = buf[cut + 1..].to_vec();
                }
                None => t.partial = buf,
            }
        }
        t.mtime_ms = modified_ms(&meta);
        Some(t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;

    fn feed(t: &mut Transcript, v: Value) {
        t.apply_line(v.to_string().as_bytes());
    }

    const TS: &str = "2026-01-02T03:04:05.678Z";

    fn assistant(content: Value, stop: &str) -> Value {
        json!({"type": "assistant", "timestamp": TS, "message": {"content": content, "stop_reason": stop}})
    }

    #[test]
    fn parses_timestamps_to_milliseconds() {
        assert_eq!(parse_ts(&json!({"timestamp": "1970-01-01T00:00:01.500Z"})), 1500);
        assert_eq!(parse_ts(&json!({"timestamp": "nonsense"})), 0);
    }

    #[test]
    fn a_turn_opens_on_a_prompt_and_closes_on_end_turn() {
        let mut t = Transcript::default();
        feed(&mut t, json!({"type": "user", "timestamp": TS, "message": {"content": "Do a made-up thing"}}));
        assert!(t.turn_open);
        feed(&mut t, assistant(json!([{"type": "text", "text": "All done"}]), "end_turn"));
        assert!(!t.turn_open);
        assert_eq!(t.last_text, "All done");
    }

    #[test]
    fn tool_results_clear_pending_calls() {
        let mut t = Transcript::default();
        feed(&mut t, assistant(json!([{"type": "tool_use", "id": "a", "name": "Read", "input": {"file_path": "/x/y.txt"}}]), "tool_use"));
        assert!(t.turn_open);
        assert_eq!(t.last_pending().unwrap().doing, "Reading y.txt");
        feed(&mut t, json!({"type": "user", "message": {"content": [{"type": "tool_result", "tool_use_id": "a"}]}}));
        assert!(t.pending.is_empty());
        assert!(t.turn_open);
    }

    #[test]
    fn interrupts_end_the_turn() {
        let mut t = Transcript::default();
        feed(&mut t, assistant(json!([{"type": "tool_use", "id": "a", "name": "Bash", "input": {}}]), "tool_use"));
        feed(&mut t, json!({"type": "user", "message": {"content": [{"type": "text", "text": "[Request interrupted by user]"}]}}));
        assert!(!t.turn_open);
        assert!(t.pending.is_empty());
    }

    #[test]
    fn meta_messages_do_not_open_a_turn_but_peer_messages_do() {
        let mut t = Transcript::default();
        feed(&mut t, json!({"type": "user", "isMeta": true, "message": {"content": "x"}}));
        assert!(!t.turn_open);
        feed(&mut t, json!({"type": "user", "isMeta": true, "origin": {"kind": "peer"}, "message": {"content": "x"}}));
        assert!(t.turn_open);
    }

    #[test]
    fn counts_lines_from_edit_and_write_results() {
        let mut t = Transcript::default();
        let result = |r: Value| json!({"type": "user", "timestamp": TS, "toolUseResult": r,
            "message": {"content": [{"type": "tool_result", "tool_use_id": "a"}]}});
        feed(&mut t, result(json!({"filePath": "C:\\code\\app\\main.rs", "type": "update",
            "structuredPatch": [{"lines": ["+new", "-old", " same", "+more"]}]})));
        feed(&mut t, result(json!({"filePath": "C:\\code\\app\\main.rs", "type": "update",
            "structuredPatch": [{"lines": ["-gone"]}]})));
        feed(&mut t, result(json!({"filePath": "/x/new.md", "type": "create", "structuredPatch": [], "content": "a\nb\nc\n"})));
        feed(&mut t, result(json!({"stdout": "not a file"})));
        let files = t.changed_files();
        assert_eq!(files.len(), 2);
        let main = files.iter().find(|f| f.name == "main.rs").unwrap();
        assert_eq!((main.added, main.removed), (2, 2));
        let new = files.iter().find(|f| f.name == "new.md").unwrap();
        assert_eq!((new.added, new.removed), (3, 0));
    }

    #[test]
    fn keeps_the_latest_steps_and_the_question() {
        let mut t = Transcript::default();
        for i in 0..(STEPS_KEPT + 3) {
            let id = format!("r{i}");
            feed(&mut t, assistant(json!([{"type": "tool_use", "id": id, "name": "Read", "input": {"file_path": format!("/f{i}.txt")}}]), "tool_use"));
        }
        feed(&mut t, assistant(json!([{"type": "tool_use", "id": "s", "name": "ToolSearch", "input": {}}]), "tool_use"));
        assert_eq!(t.steps.len(), STEPS_KEPT);
        assert_eq!(t.steps.back().unwrap().doing, format!("Reading f{}.txt", STEPS_KEPT + 2));
        feed(&mut t, assistant(json!([{"type": "tool_use", "id": "q", "name": "AskUserQuestion", "input": {"questions": [
            {"question": "Which one?", "header": "h", "multiSelect": false, "options": [{"label": "This", "description": ""}, {"label": "That", "description": ""}]}]}}]), "tool_use"));
        let q = t.pending.iter().find(|p| p.id == "q").unwrap().question.clone().unwrap();
        assert_eq!(q, Question { text: "Which one?".into(), options: vec!["This".into(), "That".into()] });
    }

    #[test]
    fn records_api_errors() {
        let mut t = Transcript::default();
        feed(&mut t, json!({"type": "system", "subtype": "api_error", "timestamp": TS}));
        assert!(t.last_error_at > 0);
    }

    #[test]
    fn garbage_lines_are_ignored() {
        let mut t = Transcript::default();
        t.apply_line(b"{not json");
        assert!(!t.turn_open);
    }

    #[test]
    fn reads_only_new_bytes_and_waits_for_a_whole_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        let mut f = File::create(&path).unwrap();
        let line = json!({"type": "user", "message": {"content": "hi"}}).to_string();
        let (a, b) = line.split_at(10);
        f.write_all(a.as_bytes()).unwrap();
        let mut cache = TranscriptCache::default();
        assert!(!cache.read(&path).unwrap().turn_open);
        writeln!(f, "{b}").unwrap();
        assert!(cache.read(&path).unwrap().turn_open);
        let end = assistant(json!([]), "end_turn").to_string();
        writeln!(f, "{end}").unwrap();
        assert!(!cache.read(&path).unwrap().turn_open);
    }

    #[test]
    fn a_shrunk_file_is_read_again_from_the_start() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        fs::write(&path, format!("{}\n{}\n", json!({"type":"user","message":{"content":"long enough to shrink"}}), assistant(json!([]), "end_turn"))).unwrap();
        let mut cache = TranscriptCache::default();
        assert!(!cache.read(&path).unwrap().turn_open);
        fs::write(&path, format!("{}\n", json!({"type":"user","message":{"content":"a"}}))).unwrap();
        assert!(cache.read(&path).unwrap().turn_open);
    }
}
