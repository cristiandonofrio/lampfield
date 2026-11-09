//! Prints where Lampfield looks and what it finds, as counts only, with no session
//! titles or file names. Handy on a new machine: `cargo run -p lampfield-core --example check`.

use lampfield_core::{default_paths, Scanner};

fn main() {
    let Some(paths) = default_paths() else {
        println!("Could not work out where the Claude app keeps its data on this machine.");
        return;
    };
    for (what, p) in [("Sessions", &paths.sessions_root), ("Transcripts", &paths.transcripts_root), ("Sidebar config", &paths.desktop_config)] {
        println!("{what:<15} {} ({})", p.display(), if p.exists() { "found" } else { "MISSING" });
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64;
    let snap = Scanner::new(paths).snapshot(now);
    let sessions: Vec<_> = snap.groups.iter().flat_map(|g| &g.sessions).collect();
    println!(
        "{} records, {} archived, {} open, in {} groups (from {}){}",
        snap.total_records,
        snap.archived_hidden,
        sessions.len(),
        snap.groups.len(),
        snap.group_source,
        if snap.group_note.is_empty() { String::new() } else { format!(": {}", snap.group_note) }
    );
    let mut states = std::collections::BTreeMap::new();
    for s in &sessions {
        *states.entry(format!("{:?}/{}", s.state, s.label)).or_insert(0) += 1;
    }
    println!("States: {states:?}");
    let with = |f: &dyn Fn(&&lampfield_core::Session) -> bool| sessions.iter().filter(|s| f(s)).count();
    println!(
        "Panel data: {} with file changes, {} with steps, {} with a question, {} unread",
        with(&|s| !s.files.is_empty()),
        with(&|s| !s.steps.is_empty()),
        with(&|s| s.question.is_some()),
        with(&|s| s.unread)
    );
    let lines: usize = sessions.iter().flat_map(|s| &s.files).map(|f| f.added + f.removed).sum();
    println!("Lines changed across open sessions: {lines}");
}
