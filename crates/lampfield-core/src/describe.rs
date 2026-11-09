//! Plain wording for what a tool call is doing.

use serde_json::Value;

/// Cuts to `n` characters with an ellipsis.
pub fn trunc(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let cut: String = s.chars().take(n.saturating_sub(1)).collect();
    format!("{}…", cut.trim_end())
}

/// First line that has text and isn't a tag, with runs of whitespace squeezed.
pub fn first_line(s: &str) -> String {
    let line = s.split('\n').map(str::trim).find(|x| !x.is_empty() && !x.starts_with('<')).unwrap_or("");
    line.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn basename(input: &Value, key: &str) -> String {
    match input.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()) {
        Some(p) => crate::paths::base_name(p).to_string(),
        None => "a file".into(),
    }
}

fn host_of(url: &str) -> String {
    url.split_once("://")
        .map(|(_, rest)| rest.split(['/', '?', '#']).next().unwrap_or(""))
        .filter(|h| !h.is_empty())
        .unwrap_or("a web page")
        .to_string()
}

pub fn describe_tool(name: &str, input: &Value) -> String {
    let s = |k: &str| input.get(k).and_then(Value::as_str).unwrap_or("");
    match name {
        "Edit" | "MultiEdit" => format!("Editing {}", basename(input, "file_path")),
        "Write" => format!("Writing {}", basename(input, "file_path")),
        "NotebookEdit" => format!("Editing {}", basename(input, "notebook_path")),
        "Read" => format!("Reading {}", basename(input, "file_path")),
        "Grep" => format!("Searching for \"{}\"", trunc(s("pattern"), 32)),
        "Glob" => format!("Looking for {}", trunc(s("pattern"), 32)),
        "Bash" if !s("description").is_empty() => trunc(s("description"), 70),
        "Bash" => format!("Running {}", trunc(&first_line(s("command")), 50)),
        "WebSearch" => format!("Searching the web for \"{}\"", trunc(s("query"), 40)),
        "WebFetch" => format!("Reading {}", host_of(s("url"))),
        "Agent" | "Task" => {
            let what = if s("description").is_empty() { s("subagent_type") } else { s("description") };
            format!("Briefing a helper: {}", trunc(what, 50))
        }
        "SendMessage" => "Messaging another session".into(),
        "AskUserQuestion" => {
            let q = input.pointer("/questions/0/question").and_then(Value::as_str).unwrap_or("a question");
            format!("Asking you: {}", trunc(q, 90))
        }
        "ExitPlanMode" => "Waiting for you to approve a plan".into(),
        "TodoWrite" => "Updating its to-do list".into(),
        "Skill" => format!("Using the {} skill", s("skill")).replace("  ", " "),
        "ToolSearch" => "Loading tools".into(),
        n if n.starts_with("mcp__Claude_Browser__") || n.starts_with("mcp__claude-in-chrome__") => {
            "Using the browser".into()
        }
        "mcp__ccd_session__spawn_task" => format!("Starting a session: {}", trunc(s("title"), 50)),
        n if n.starts_with("mcp__ccd_session") => "Managing sessions".into(),
        n if n.starts_with("mcp__") => {
            format!("Using {}", n.rsplit("__").next().unwrap_or(n).replace(['-', '_'], " "))
        }
        n => format!("Using {n}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn describes_common_tools() {
        assert_eq!(describe_tool("Edit", &json!({"file_path": "/a/b/c.rs"})), "Editing c.rs");
        assert_eq!(describe_tool("Read", &json!({})), "Reading a file");
        assert_eq!(describe_tool("Edit", &json!({"file_path": "C:\\code\\app\\main.rs"})), "Editing main.rs");
        assert_eq!(describe_tool("Bash", &json!({"description": "Run tests"})), "Run tests");
        assert_eq!(describe_tool("Bash", &json!({"command": "ls -la\npwd"})), "Running ls -la");
        assert_eq!(describe_tool("WebFetch", &json!({"url": "https://example.com/x?y"})), "Reading example.com");
        assert_eq!(describe_tool("mcp__thing__do_a-job", &json!({})), "Using do a job");
        assert_eq!(describe_tool("mcp__Claude_Browser__navigate", &json!({})), "Using the browser");
        assert_eq!(
            describe_tool("AskUserQuestion", &json!({"questions": [{"question": "Which one?"}]})),
            "Asking you: Which one?"
        );
    }

    #[test]
    fn trunc_counts_characters_not_bytes() {
        assert_eq!(trunc("héllo wörld", 6), "héllo…");
        assert_eq!(trunc("short", 10), "short");
    }

    #[test]
    fn first_line_skips_tags_and_blanks() {
        assert_eq!(first_line("\n<tag>\n  hello   there \nmore"), "hello there");
        assert_eq!(first_line(""), "");
    }
}
