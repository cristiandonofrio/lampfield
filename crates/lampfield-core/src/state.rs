//! Works out a session's state. "Quiet" (a tool pending a while, maybe an approval
//! prompt) stays a Working state with the label "Quiet", because approval prompts
//! aren't written to the logs.

use crate::describe::first_line;
use crate::records::Record;
use crate::transcript::{Pending, Question, Transcript};
use crate::State;

pub const WORKING_STALE_MS: i64 = 10 * 60 * 1000;
pub const QUIET_MS: i64 = 45 * 1000;

#[derive(Debug, PartialEq)]
pub struct Status {
    pub state: State,
    pub label: String,
    pub doing: String,
    /// For Needs and Failed: what you'd be dismissing. A new question, reply or error
    /// gives a new key, so a dismissal only lasts until then. Empty otherwise.
    pub alert_key: String,
    /// The question and its answers, when the session asked one with options.
    pub question: Option<Question>,
}

fn status(state: State, label: &str, doing: impl Into<String>) -> Status {
    Status { state, label: label.into(), doing: doing.into(), alert_key: String::new(), question: None }
}

fn alert(state: State, label: &str, doing: impl Into<String>, key: String) -> Status {
    Status { alert_key: key, ..status(state, label, doing) }
}

fn pending_text(p: &Pending, title_for: &dyn Fn(&str) -> Option<String>) -> String {
    match p.to.as_deref().and_then(title_for) {
        Some(title) => format!("Messaging {title}"),
        None => p.doing.clone(),
    }
}

pub fn derive(
    rec: &Record,
    t: Option<&Transcript>,
    now: i64,
    title_for: &dyn Fn(&str) -> Option<String>,
) -> Status {
    let Some(t) = t else { return status(State::Idle, "No log found", "") };
    let since_write = now - t.mtime_ms;

    // A question waits as long as it takes, so this comes before the stale check.
    // Otherwise the sessions left longest would be the ones to drop off the menu bar.
    if t.turn_open {
        if let Some(p) = t.pending.iter().find(|p| p.name == "AskUserQuestion" || p.name == "ExitPlanMode") {
            let mut st = alert(State::Needs, "Needs you", pending_text(p, title_for), format!("ask:{}", p.id));
            st.question = p.question.clone();
            return st;
        }
    }

    if t.turn_open && since_write < WORKING_STALE_MS {
        return match t.last_pending() {
            Some(p) if since_write > QUIET_MS => status(
                State::Working,
                "Quiet",
                format!("{}, or waiting for your approval", pending_text(p, title_for)),
            ),
            Some(p) => status(State::Working, "Working", pending_text(p, title_for)),
            None => status(State::Working, "Working", "Thinking"),
        };
    }

    if t.turn_open {
        return status(State::Idle, "Stopped", "Stopped partway through a task");
    }
    if t.last_error_at != 0 && t.last_error_at > t.last_assistant_at {
        return alert(State::Failed, "Error", "Stopped on an API error", format!("error:{}", t.last_error_at));
    }
    let summary = rec.summary.as_ref().filter(|s| s.current);
    // The session's own wrap-up says whether it left you a question. The app's summary
    // classifier also marks plain updates that end on an offer ("tell me if...") as
    // blocked, so it only decides for sessions that wrote no wrap-up.
    let needs = match &rec.wrap_up {
        Some(w) => w.needs_user.clone(),
        None => summary.filter(|s| s.category.as_deref() == Some("blocked")).map(|s| {
            s.needs_action.clone().or_else(|| s.detail.clone()).unwrap_or_else(|| "Waiting for you".into())
        }),
    };
    if let Some(doing) = needs {
        return alert(State::Needs, "Needs you", doing, format!("reply:{}", rec.last_assistant));
    }
    let doing = summary
        .and_then(|s| s.detail.clone())
        .or_else(|| Some(first_line(&t.last_text)).filter(|l| !l.is_empty()))
        .unwrap_or_else(|| "Nothing running".into());
    status(State::Idle, "Idle", doing)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::records::{Summary, WrapUp};

    const NOW: i64 = 1_000_000_000;

    fn rec(summary: Option<Summary>) -> Record {
        Record {
            session_id: "local_1".into(),
            cli_session_id: "c".into(),
            cwd: String::new(),
            origin_cwd: String::new(),
            title: "Made-up session".into(),
            is_archived: false,
            created_at: 0,
            summary,
            wrap_up: None,
            last_assistant: "u1".into(),
            last_activity: 0,
            last_used: 0,
            last_turn_edge: 0,
            last_focused: 0,
        }
    }

    fn pending(name: &str) -> Pending {
        Pending { id: "a".into(), name: name.into(), doing: format!("doing {name}"), to: None, question: None, ts: 1 }
    }

    fn open(age_ms: i64, p: Vec<Pending>) -> Transcript {
        Transcript { turn_open: true, mtime_ms: NOW - age_ms, pending: p, ..Default::default() }
    }

    fn run(r: &Record, t: &Transcript) -> Status {
        derive(r, Some(t), NOW, &|_| None)
    }

    #[test]
    fn no_transcript_reads_as_idle() {
        assert_eq!(derive(&rec(None), None, NOW, &|_| None).label, "No log found");
    }

    #[test]
    fn open_turn_is_working() {
        let s = run(&rec(None), &open(1000, vec![]));
        assert_eq!((s.state, s.doing.as_str()), (State::Working, "Thinking"));
        let s = run(&rec(None), &open(1000, vec![pending("Read")]));
        assert_eq!((s.state, s.label.as_str()), (State::Working, "Working"));
    }

    #[test]
    fn an_unanswered_question_still_needs_you_after_the_stale_limit() {
        let s = run(&rec(None), &open(WORKING_STALE_MS + 1, vec![pending("AskUserQuestion")]));
        assert_eq!((s.state, s.label.as_str()), (State::Needs, "Needs you"));
    }

    #[test]
    fn a_stale_turn_with_no_question_is_still_stopped() {
        let s = run(&rec(None), &open(WORKING_STALE_MS + 1, vec![pending("Bash")]));
        assert_eq!((s.state, s.label.as_str()), (State::Idle, "Stopped"));
    }

    #[test]
    fn a_long_pending_tool_is_quiet_but_still_working() {
        let s = run(&rec(None), &open(QUIET_MS + 1, vec![pending("Bash")]));
        assert_eq!((s.state, s.label.as_str()), (State::Working, "Quiet"));
    }

    #[test]
    fn a_question_carries_its_answers() {
        let mut p = pending("AskUserQuestion");
        p.question = Some(Question { text: "Which?".into(), options: vec!["A".into(), "B".into()] });
        let s = run(&rec(None), &open(1000, vec![p]));
        assert_eq!(s.question.unwrap().options, ["A", "B"]);
    }

    #[test]
    fn a_question_or_plan_needs_you_even_behind_other_tools() {
        for tool in ["AskUserQuestion", "ExitPlanMode"] {
            let s = run(&rec(None), &open(1000, vec![pending(tool), pending("Read")]));
            assert_eq!(s.state, State::Needs);
        }
    }

    #[test]
    fn a_stale_open_turn_is_stopped_not_working() {
        let s = run(&rec(None), &open(WORKING_STALE_MS + 1, vec![pending("Bash")]));
        assert_eq!((s.state, s.label.as_str()), (State::Idle, "Stopped"));
    }

    #[test]
    fn an_api_error_after_the_last_reply_is_failed() {
        let t = Transcript { last_error_at: 20, last_assistant_at: 10, ..Default::default() };
        assert_eq!(run(&rec(None), &t).state, State::Failed);
        let t = Transcript { last_error_at: 5, last_assistant_at: 10, ..Default::default() };
        assert_eq!(run(&rec(None), &t).state, State::Idle);
    }

    fn summary(current: bool) -> Option<Summary> {
        Some(Summary {
            category: Some("blocked".into()),
            detail: Some("Detail".into()),
            needs_action: Some("Approve the thing".into()),
            current,
        })
    }

    #[test]
    fn blocked_summary_needs_you_only_while_current() {
        let t = Transcript { last_text: "Last words".into(), ..Default::default() };
        let s = run(&rec(summary(true)), &t);
        assert_eq!((s.state, s.doing.as_str()), (State::Needs, "Approve the thing"));
        let s = run(&rec(summary(false)), &t);
        assert_eq!((s.state, s.doing.as_str()), (State::Idle, "Last words"));
    }

    #[test]
    fn the_wrap_up_overrules_a_blocked_summary() {
        let t = Transcript { last_text: "Last words".into(), ..Default::default() };
        let mut r = rec(summary(true));
        r.wrap_up = Some(WrapUp { needs_user: None });
        assert_eq!(run(&r, &t).state, State::Idle);
        r.wrap_up = Some(WrapUp { needs_user: Some("Which one?".into()) });
        let s = run(&r, &t);
        assert_eq!((s.state, s.doing.as_str()), (State::Needs, "Which one?"));
        let mut r = rec(None);
        r.wrap_up = Some(WrapUp { needs_user: Some("Which one?".into()) });
        assert_eq!(run(&r, &t).state, State::Needs);
    }

    #[test]
    fn alerts_carry_a_key_that_changes_with_the_turn() {
        let t = Transcript::default();
        let mut r = rec(summary(true));
        assert_eq!(run(&r, &t).alert_key, "reply:u1");
        r.last_assistant = "u2".into();
        assert_eq!(run(&r, &t).alert_key, "reply:u2");
        let s = run(&rec(None), &open(1000, vec![pending("AskUserQuestion")]));
        assert_eq!(s.alert_key, "ask:a");
        let t = Transcript { last_error_at: 20, last_assistant_at: 10, ..Default::default() };
        assert_eq!(run(&rec(None), &t).alert_key, "error:20");
        assert_eq!(run(&rec(None), &Transcript::default()).alert_key, "");
    }

    #[test]
    fn message_targets_use_the_session_title() {
        let mut p = pending("SendMessage");
        p.to = Some("local_2".into());
        let s = derive(&rec(None), Some(&open(1000, vec![p])), NOW, &|_| Some("Other one".into()));
        assert_eq!(s.doing, "Messaging Other one");
    }
}
