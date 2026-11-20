//! `.zerd/prompt.yaml`: one file per project saying what zerd may work on, the rules for doing it, and what "done"
//! means here. zerd writes a first draft from the repo the first time it opens a project (you edit it), puts it in
//! front of the model with the other project rules, and asks you to confirm every task before running it.
//! Each answer is appended to `.zerd/confirmations.jsonl` so there is a trace of what was asked and approved.
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Deserialize, Clone)]
#[serde(default)]
pub struct ProjectPrompt {
    pub project: String,
    /// What this project is and what zerd is allowed to work on in it.
    pub scope: String,
    /// Ask "is this correct?" before running each task.
    pub confirm_tasks: bool,
    /// How work gets done here (style, tools, process).
    pub rules: Vec<String>,
    /// Commands that must pass before a task counts as finished.
    pub done_when: Vec<String>,
    /// Paths or areas zerd must not change.
    pub never_touch: Vec<String>,
}

pub fn path(cwd: &Path) -> PathBuf {
    cwd.join(".zerd").join("prompt.yaml")
}

pub fn log_path(cwd: &Path) -> PathBuf {
    cwd.join(".zerd").join("confirmations.jsonl")
}

pub fn load(cwd: &Path) -> Option<ProjectPrompt> {
    let text = std::fs::read_to_string(path(cwd)).ok()?;
    serde_yaml_ng::from_str(&text).ok()
}

fn quote(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into()) // JSON strings are valid YAML
}

/// Guess the project's name and its test/build commands from the files in its root.
fn detect(cwd: &Path) -> (String, Vec<String>, Vec<String>) {
    let dir_name = cwd.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "project".into());
    let mut name = dir_name;
    let mut done = vec![];
    let mut rules = vec![];
    if let Ok(t) = std::fs::read_to_string(cwd.join("Cargo.toml")) {
        if let Some(n) = t.parse::<toml::Table>().ok().and_then(|t| t.get("package")?.get("name")?.as_str().map(String::from)) {
            name = n;
        }
        done.extend(["cargo build".to_string(), "cargo test".to_string()]);
        rules.push("Rust project: keep `cargo build` warning-free.".to_string());
    }
    if let Ok(t) = std::fs::read_to_string(cwd.join("package.json")) {
        let v: serde_json::Value = serde_json::from_str(&t).unwrap_or_default();
        if let Some(n) = v["name"].as_str() {
            name = n.to_string();
        }
        let pm = if cwd.join("pnpm-lock.yaml").exists() { "pnpm" } else if cwd.join("yarn.lock").exists() { "yarn" } else { "npm" };
        for s in ["lint", "build", "test"] {
            if v["scripts"][s].is_string() {
                done.push(format!("{pm} run {s}"));
            }
        }
        rules.push(format!("Node project: use {pm}, not another package manager."));
    }
    if cwd.join("pyproject.toml").exists() || cwd.join("requirements.txt").exists() {
        done.push("pytest".to_string());
        rules.push("Python project: do not install packages globally.".to_string());
    }
    if cwd.join("go.mod").exists() {
        done.push("go test ./...".to_string());
    }
    (name, done, rules)
}

/// The first draft. Plain text, not serialized, so the comments explaining each field survive.
pub fn draft(cwd: &Path) -> String {
    let (name, done, rules) = detect(cwd);
    let list = |items: &[String], empty: &str| {
        if items.is_empty() { format!("  {empty}") } else { items.iter().map(|i| format!("  - {}", quote(i))).collect::<Vec<_>>().join("\n") }
    };
    format!(
        "# zerd project prompt. zerd drafted this from the repo; edit it freely, it is read at every start.\n\
         version: 1\n\
         project: {}\n\
         \n\
         # What this project is and what zerd may work on in it.\n\
         scope: {}\n\
         \n\
         # Before running each task zerd restates it and asks if it is correct. Every answer is logged to .zerd/confirmations.jsonl.\n\
         confirm_tasks: true\n\
         \n\
         # How work gets done here.\n\
         rules:\n{}\n\
         \n\
         # Commands that must pass before a task counts as finished.\n\
         done_when:\n{}\n\
         \n\
         # Paths zerd must not change.\n\
         never_touch:\n{}\n",
        quote(&name),
        quote("Describe what this project is and which parts zerd may change."),
        list(&rules, "[]"),
        list(&done, "[]"),
        list(&[], "[]"),
    )
}

/// Create the file if this project has none. Returns true when it was just written.
pub fn ensure(cwd: &Path) -> bool {
    let p = path(cwd);
    if p.exists() {
        return false;
    }
    std::fs::create_dir_all(cwd.join(".zerd")).is_ok() && std::fs::write(&p, draft(cwd)).is_ok()
}

/// The block that goes in front of the model with the other project instructions.
pub fn instructions(cwd: &Path) -> Option<String> {
    let p = load(cwd)?;
    let mut s = format!("## Project prompt ({})\nProject: {}\n", path(cwd).display(), p.project);
    if !p.scope.trim().is_empty() {
        s.push_str(&format!("Scope: {}\n", p.scope.trim()));
    }
    for (title, items) in [("Rules", &p.rules), ("Done when (all must pass before you call a task finished)", &p.done_when), ("Never touch", &p.never_touch)] {
        if !items.is_empty() {
            s.push_str(&format!("{title}:\n{}\n", items.iter().map(|i| format!("- {i}")).collect::<Vec<_>>().join("\n")));
        }
    }
    Some(s)
}

/// Append one line to the confirmation trace.
pub fn log_confirmation(cwd: &Path, task: &str, answer: &str) {
    use std::io::Write;
    let line = serde_json::json!({"time": chrono::Local::now().to_rfc3339(), "task": task, "answer": answer});
    if std::fs::create_dir_all(cwd.join(".zerd")).is_ok() {
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(log_path(cwd)) {
            let _ = writeln!(f, "{line}");
        }
    }
}

/// Turn `confirm_tasks` on or off in place, keeping the file's comments.
pub fn set_confirm(cwd: &Path, on: bool) -> bool {
    let p = path(cwd);
    let Ok(text) = std::fs::read_to_string(&p) else { return false };
    let mut found = false;
    let mut lines: Vec<String> = text
        .lines()
        .map(|l| {
            if l.starts_with("confirm_tasks:") {
                found = true;
                format!("confirm_tasks: {on}")
            } else {
                l.to_string()
            }
        })
        .collect();
    if !found {
        lines.push(format!("confirm_tasks: {on}"));
    }
    std::fs::write(p, lines.join("\n") + "\n").is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_draft_parses_and_carries_the_detected_commands() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("Cargo.toml"), "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n").unwrap();
        assert!(ensure(d.path()));
        assert!(!ensure(d.path()), "an existing file is never overwritten");
        let p = load(d.path()).expect("draft is valid yaml");
        assert_eq!(p.project, "demo");
        assert!(p.confirm_tasks && p.done_when.contains(&"cargo test".to_string()));
        assert!(instructions(d.path()).unwrap().contains("cargo test"));
    }

    #[test]
    fn confirmations_are_logged_and_the_switch_keeps_comments() {
        let d = tempfile::tempdir().unwrap();
        ensure(d.path());
        log_confirmation(d.path(), "fix \"the\" bug", "yes");
        let log = std::fs::read_to_string(log_path(d.path())).unwrap();
        assert!(log.contains("\"answer\":\"yes\"") && log.contains("fix \\\"the\\\" bug"));
        assert!(set_confirm(d.path(), false));
        assert!(!load(d.path()).unwrap().confirm_tasks);
        assert!(std::fs::read_to_string(path(d.path())).unwrap().starts_with("# zerd project prompt"));
    }
}
