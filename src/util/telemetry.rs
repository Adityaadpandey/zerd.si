//! Opt-in usage numbers, sent to the zerd site so the harness can be tuned on more than one person's traces.
//!
//! Off until the user runs `zerd telemetry on`. What leaves the machine is one small record per finished request,
//! built only from counts and names zerd itself defines. Nothing the user typed, no file names, no commands, no
//! error text and no code is in it. Example of a whole record:
//!
//!   {"v":1,"install":"3f0c...","zerd":"0.1.2","os":"macos","arch":"aarch64","model":"MiniMaxAI/MiniMax-M3",
//!    "intent":"change","status":"unverified","stop":"token budget reached","rounds":41,"estimate":15,"sec":512,
//!    "calls":43,"prompt_tokens":4100000,"cached_tokens":3900000,"completion_tokens":21000,"changed":12,
//!    "repairs":0,"checks":1,"checks_failed":0,"compactions":1,"prunes":2,
//!    "tools":{"edit":[30,2],"shell":[14,3]},"nudges":{"edit_churn":2}}
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

const DEFAULT_URL: &str = "https://zerd.si/api/telemetry";

/// Tools zerd ships. Anything else (an MCP tool, a user's own tool or sub-agent) could carry a private name, so it
/// is counted under "other".
const BUILTIN: &[&str] = &[
    "advisor", "ask_user", "edit", "git", "glob", "graph", "grep", "list_dir", "notes", "outline", "parallel_tasks", "read_file", "recall_result", "search", "shell", "todos", "tool_search",
    "use_skill", "web_fetch", "web_search", "write_file",
];

/// Why a request ended, when the reason is one of zerd's own fixed phrases. A failure note can quote an error
/// message with paths in it, so anything not on this list is sent as "other".
const STOPS: &[&str] = &["", "token budget reached", "no verification commands", "a check was already failing before the change", "checks after budget stop", "agent loop aborted"];

fn state_file(data_dir: &Path) -> PathBuf {
    data_dir.join("telemetry.json")
}

fn read_state(data_dir: &Path) -> Value {
    std::fs::read_to_string(state_file(data_dir)).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(json!({}))
}

/// The user said yes, and nothing in the environment says no (`DO_NOT_TRACK=1` and `ZERD_TELEMETRY=0` both win).
pub fn enabled(data_dir: &Path) -> bool {
    let no = |k: &str, v: &str| std::env::var(k).map_or(false, |x| x == v);
    !no("DO_NOT_TRACK", "1") && !no("ZERD_TELEMETRY", "0") && read_state(data_dir)["enabled"] == true
}

/// Turning it on makes a random id for this install, so records from one machine can be told apart from another's
/// without knowing whose machine it is. Turning it off deletes the id too.
pub fn set_enabled(data_dir: &Path, on: bool) -> std::io::Result<()> {
    if !on {
        return match std::fs::remove_file(state_file(data_dir)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    let id = read_state(data_dir)["install"].as_str().map(String::from).unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(data_dir)?;
    std::fs::write(state_file(data_dir), json!({"enabled": true, "install": id}).to_string())
}

/// The record for the request that just ended, from the trace events since its `task_start`.
/// `checks` is (how many checks the harness ran, how many of them failed).
pub fn record(events: &[Value], install: &str, model: &str, intent: &str, estimate: usize, rounds: usize, checks: (usize, usize)) -> Value {
    let start = events.iter().rposition(|e| e["event"] == "task_start").unwrap_or(0);
    let ev = &events[start..];
    let count = |name: &str| ev.iter().filter(|e| e["event"] == name).count();
    let tokens = |k: &str| ev.iter().filter(|e| e["event"] == "model_call").map(|e| e["tokens"][k].as_i64().unwrap_or(0)).sum::<i64>();
    let end = ev.iter().rfind(|e| e["event"] == "task_end");
    let note = ev.iter().rfind(|e| e["event"] == "task_transition").and_then(|e| e["note"].as_str()).unwrap_or("");
    let mut tools: Map<String, Value> = Map::new();
    for e in ev.iter().filter(|e| e["event"] == "tool_call") {
        let name = e["tool"].as_str().unwrap_or("");
        let key = if BUILTIN.contains(&name) { name } else if name.starts_with("subagent_") { "subagent" } else { "other" };
        let slot = tools.entry(key.to_string()).or_insert(json!([0, 0]));
        slot[0] = json!(slot[0].as_i64().unwrap_or(0) + 1);
        if e["ok"] != true {
            slot[1] = json!(slot[1].as_i64().unwrap_or(0) + 1);
        }
    }
    let mut nudges: Map<String, Value> = Map::new();
    for e in ev.iter().filter(|e| e["event"] == "nudge") {
        let slot = nudges.entry(e["kind"].as_str().unwrap_or("other").to_string()).or_insert(json!(0));
        *slot = json!(slot.as_i64().unwrap_or(0) + 1);
    }
    json!({
        "v": 1,
        "install": install,
        "zerd": env!("CARGO_PKG_VERSION"),
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "model": model,
        "intent": intent,
        "status": end.and_then(|e| e["status"].as_str()).unwrap_or("unknown"),
        "stop": if STOPS.contains(&note) { note } else if note.starts_with("Stopped: this change needs approval") { "no approver" } else { "other" },
        "rounds": rounds,
        "estimate": estimate,
        "sec": end.and_then(|e| e["sec"].as_f64()).unwrap_or(0.0).round(),
        "calls": count("model_call"),
        "prompt_tokens": tokens("prompt_tokens"),
        "cached_tokens": tokens("cached_tokens"),
        "completion_tokens": tokens("completion_tokens"),
        "changed": end.and_then(|e| e["changed"].as_array()).map_or(0, Vec::len),
        "repairs": end.and_then(|e| e["repairs"].as_i64()).unwrap_or(0),
        "checks": checks.0,
        "checks_failed": checks.1,
        "compactions": count("compaction"),
        "prunes": count("prune"),
        "tools": tools,
        "nudges": nudges,
    })
}

/// Send the record for the finished request, if the user opted in. Never waits and never reports a failure: a dead
/// network must not slow down or break a run.
pub fn send_if_enabled(data_dir: &Path, events: &[Value], model: &str, intent: &str, estimate: usize, rounds: usize, checks: (usize, usize)) {
    if !enabled(data_dir) {
        return;
    }
    let state = read_state(data_dir);
    let body = record(events, state["install"].as_str().unwrap_or(""), model, intent, estimate, rounds, checks);
    let url = std::env::var("ZERD_TELEMETRY_URL").unwrap_or_else(|_| DEFAULT_URL.into());
    // ponytail: fire and forget. A one-shot run gives it the runtime's one second of shutdown grace, so a slow
    // network drops that record. Queue to disk and resend on the next start if the gaps ever matter.
    tokio::spawn(async move {
        if let Ok(client) = reqwest::Client::builder().timeout(std::time::Duration::from_secs(4)).build() {
            let _ = client.post(url).json(&body).send().await;
        }
    });
}

/// `zerd telemetry [on|off|status]`
pub fn main(args: Vec<String>) -> i32 {
    let dir = crate::config::loader::get_data_dir();
    match args.first().map(String::as_str) {
        Some(word @ ("on" | "off")) => {
            if let Err(e) = set_enabled(&dir, word == "on") {
                eprintln!("zerd telemetry: could not save the choice: {e}");
                return 1;
            }
            println!("Telemetry is {word}.");
            if word == "on" {
                println!("Sent after each request: counts, timings, token totals, the model name and how the request ended.");
                println!("Never sent: what you typed, file names, commands, error text, code. Turn it off with `zerd telemetry off`.");
            }
            0
        }
        None | Some("status") => {
            println!("Telemetry is {}.", if enabled(&dir) { "on" } else { "off" });
            println!("It is opt-in: `zerd telemetry on` to share anonymous usage numbers, `zerd telemetry off` to stop.");
            0
        }
        Some(other) => {
            eprintln!("zerd telemetry: unknown option '{other}'. Use on, off or status.");
            2
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_is_off_until_the_user_turns_it_on_and_off_again_forgets_the_id() {
        let d = tempfile::tempdir().unwrap();
        assert!(!enabled(d.path()));
        set_enabled(d.path(), true).unwrap();
        assert!(read_state(d.path())["enabled"] == true && read_state(d.path())["install"].as_str().unwrap().len() == 36);
        set_enabled(d.path(), false).unwrap();
        assert!(!enabled(d.path()) && !state_file(d.path()).exists());
    }

    #[test]
    fn the_record_holds_counts_and_never_the_users_text_paths_or_commands() {
        let events = vec![
            json!({"event": "task_start", "request": "fix SECRET-PROJECT login", "task": "t1"}),
            json!({"event": "model_call", "tokens": {"prompt_tokens": 1000, "cached_tokens": 800, "completion_tokens": 50}}),
            json!({"event": "tool_call", "tool": "shell", "ok": false, "target": "cat /Users/me/private.env", "error": "no such file /Users/me"}),
            json!({"event": "tool_call", "tool": "edit", "ok": true, "target": "src/private_name.rs"}),
            json!({"event": "tool_call", "tool": "mcp__acme_internal__lookup", "ok": true}),
            json!({"event": "nudge", "kind": "edit_churn"}),
            json!({"event": "task_transition", "note": "Request failed: /Users/me/private.env unreadable"}),
            json!({"event": "task_end", "status": "failed", "sec": 12.4, "repairs": 1, "changed": ["src/private_name.rs"]}),
        ];
        let r = record(&events, "id-1", "some/model", "change", 10, 4, (2, 1));
        assert_eq!(r["tools"], json!({"shell": [1, 1], "edit": [1, 0], "other": [1, 0]}));
        assert!(r["status"] == "failed" && r["stop"] == "other" && r["changed"] == 1 && r["checks_failed"] == 1 && r["prompt_tokens"] == 1000 && r["nudges"]["edit_churn"] == 1);
        let text = r.to_string();
        for private in ["SECRET-PROJECT", "/Users/me", "private_name", "acme_internal", "private.env"] {
            assert!(!text.contains(private), "{private} leaked into {text}");
        }
    }
}
