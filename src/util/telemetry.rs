//! Opt-in usage numbers, sent to the zerd site so the harness can be tuned on more than one person's traces.
//!
//! Off until the user says yes (`zerd telemetry on`, or the question in `zerd setup`). What leaves the machine is one
//! small record per finished request, built only from counts and names zerd itself defines. Nothing the user typed,
//! no file names, no commands, no error text and no code is in it. Example of a whole record:
//!
//!   {"v":1,"rid":"9b1e...","install":"3f0c...","zerd":"0.1.2","os":"macos","arch":"aarch64",
//!    "model":"MiniMaxAI/MiniMax-M3","intent":"change","status":"unverified","stop":"token budget reached",
//!    "rounds":41,"estimate":15,"sec":512,"model_sec":300,"tool_sec":90,"calls":43,"model_errors":0,
//!    "prompt_tokens":4100000,"cached_tokens":3900000,"completion_tokens":21000,"peak_prompt":148000,"cold_calls":2,
//!    "window":1000000,"changed":12,"repairs":0,"checks":1,"checks_failed":0,"compactions":1,"prunes":2,
//!    "tools":{"edit":[30,2],"shell":[14,3]},"nudges":{"edit_churn":2}}
//!
//! It must never get in the way of the work, so the request itself only appends that one line to a local file.
//! A background task then posts everything waiting in one batch. If the network is down, slow, or the program exits
//! first, the lines simply stay on disk and go out with a later batch.
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
const STOPS: &[&str] = &["", "token budget reached", "no verification commands", "a check was already failing before the change", "checks after budget stop", "agent loop aborted", "checks still running in the background", "only docs changed, nothing to run"];

/// Stop collecting when this much is waiting: someone who is offline for a month should not grow a file forever.
const MAX_WAITING_BYTES: u64 = 256 * 1024;
/// A batch somebody started sending this long ago and never finished (the program exited mid-send) is picked up again.
const STALE_SECS: u64 = 120;

fn queue_file(data_dir: &Path) -> PathBuf {
    data_dir.join("telemetry_queue.jsonl")
}

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
        // "off" means off: records still waiting to go out are thrown away, not sent later
        for e in std::fs::read_dir(data_dir).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name == "telemetry_queue.jsonl" || name.starts_with("telemetry_sending_") {
                let _ = std::fs::remove_file(e.path());
            }
        }
        return match std::fs::remove_file(state_file(data_dir)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    let id = read_state(data_dir)["install"].as_str().map(String::from).unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(data_dir)?;
    std::fs::write(state_file(data_dir), json!({"enabled": true, "install": id}).to_string())
}

/// The record for the request that just ended, from the trace events since its `task_start`. `extra` holds the
/// few things the trace does not know (model, intent, the round estimate, how the checks went) and is merged in.
pub fn record(events: &[Value], install: &str, extra: Value) -> Value {
    let start = events.iter().rposition(|e| e["event"] == "task_start").unwrap_or(0);
    let ev = &events[start..];
    let end = ev.iter().rfind(|e| e["event"] == "task_end");
    let note = ev.iter().rfind(|e| e["event"] == "task_transition").and_then(|e| e["note"].as_str()).unwrap_or("");
    // one pass over the events: a long session has thousands of them
    let (mut calls, mut model_errors, mut cold_calls, mut compactions, mut prunes) = (0i64, 0i64, 0i64, 0i64, 0i64);
    let (mut prompt, mut cached, mut completion, mut peak) = (0i64, 0i64, 0i64, 0i64);
    let (mut model_sec, mut tool_sec) = (0f64, 0f64);
    let mut tools: Map<String, Value> = Map::new();
    let mut nudges: Map<String, Value> = Map::new();
    for e in ev {
        match e["event"].as_str().unwrap_or("") {
            "model_call" => {
                let t = |k: &str| e["tokens"][k].as_i64().unwrap_or(0);
                calls += 1;
                model_errors += (e["ok"] == false) as i64;
                prompt += t("prompt_tokens");
                cached += t("cached_tokens");
                completion += t("completion_tokens");
                peak = peak.max(t("prompt_tokens"));
                // a big prompt with nothing served from cache: the expensive kind of call
                cold_calls += (t("prompt_tokens") > 15_000 && t("cached_tokens") == 0) as i64;
                model_sec += e["sec"].as_f64().unwrap_or(0.0);
            }
            "tool_call" => {
                let name = e["tool"].as_str().unwrap_or("");
                let key = if BUILTIN.contains(&name) { name } else if name.starts_with("subagent_") { "subagent" } else { "other" };
                let slot = tools.entry(key.to_string()).or_insert(json!([0, 0]));
                slot[0] = json!(slot[0].as_i64().unwrap_or(0) + 1);
                if e["ok"] != true {
                    slot[1] = json!(slot[1].as_i64().unwrap_or(0) + 1);
                }
                tool_sec += e["sec"].as_f64().unwrap_or(0.0);
            }
            "nudge" => {
                let slot = nudges.entry(e["kind"].as_str().unwrap_or("other").to_string()).or_insert(json!(0));
                *slot = json!(slot.as_i64().unwrap_or(0) + 1);
            }
            "compaction" => compactions += 1,
            "prune" => prunes += 1,
            _ => {}
        }
    }
    let mut rec = json!({
        "v": 1,
        "rid": uuid::Uuid::new_v4().to_string(), // lets the server drop a record that a retry delivered twice
        "install": install,
        "zerd": env!("CARGO_PKG_VERSION"),
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "status": end.and_then(|e| e["status"].as_str()).unwrap_or("unknown"),
        "stop": if STOPS.contains(&note) { note } else if note.starts_with("Stopped: this change needs approval") { "no approver" } else { "other" },
        "sec": end.and_then(|e| e["sec"].as_f64()).unwrap_or(0.0).round(),
        "model_sec": model_sec.round(),
        "tool_sec": tool_sec.round(),
        "calls": calls,
        "model_errors": model_errors,
        "prompt_tokens": prompt,
        "cached_tokens": cached,
        "completion_tokens": completion,
        "peak_prompt": peak,
        "cold_calls": cold_calls,
        "changed": end.and_then(|e| e["changed"].as_array()).map_or(0, Vec::len),
        "repairs": end.and_then(|e| e["repairs"].as_i64()).unwrap_or(0),
        "compactions": compactions,
        "prunes": prunes,
        "tools": tools,
        "nudges": nudges,
    });
    if let (Some(r), Value::Object(x)) = (rec.as_object_mut(), extra) {
        r.extend(x);
    }
    rec
}

/// Called when a request ends. If the user opted in, write the record to the waiting file (one short append, no
/// network) and let a background task post whatever is waiting. Nothing here can fail the run or make it wait.
pub fn send_if_enabled(data_dir: &Path, events: &[Value], extra: Value) {
    if !enabled(data_dir) {
        return;
    }
    let queue = queue_file(data_dir);
    if std::fs::metadata(&queue).map_or(0, |m| m.len()) < MAX_WAITING_BYTES {
        use std::io::Write;
        let rec = record(events, read_state(data_dir)["install"].as_str().unwrap_or(""), extra);
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&queue) {
            let _ = writeln!(f, "{rec}");
        }
    }
    let dir = data_dir.to_path_buf();
    tokio::spawn(async move { flush(&dir).await });
}

/// Post everything that is waiting, in one request.
///
/// The waiting file is first renamed to a name of our own, so a second zerd running next to this one cannot send the
/// same lines. Files are deleted only after the server said yes. If the send fails or the program exits halfway,
/// the renamed file is still there, and whichever flush comes next (two minutes later or next week) takes it along.
async fn flush(data_dir: &Path) -> bool {
    let mine = data_dir.join(format!("telemetry_sending_{}.jsonl", uuid::Uuid::new_v4()));
    let _ = std::fs::rename(queue_file(data_dir), &mine);
    let old = |p: &PathBuf| std::fs::metadata(p).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).map_or(false, |d| d.as_secs() > STALE_SECS);
    let batch: Vec<PathBuf> = std::fs::read_dir(data_dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().map_or(false, |n| n.to_string_lossy().starts_with("telemetry_sending_")) && (p == &mine || old(p)))
        .take(20)
        .collect();
    let records: Vec<Value> = batch.iter().filter_map(|p| std::fs::read_to_string(p).ok()).flat_map(|t| t.lines().filter_map(|l| serde_json::from_str(l).ok()).collect::<Vec<Value>>()).take(200).collect();
    if records.is_empty() {
        batch.iter().for_each(|p| drop(std::fs::remove_file(p)));
        return true;
    }
    // one client for the life of the process: building it loads the TLS roots, which is too much work to redo per send
    static CLIENT: std::sync::OnceLock<Option<reqwest::Client>> = std::sync::OnceLock::new();
    let Some(client) = CLIENT.get_or_init(|| reqwest::Client::builder().timeout(std::time::Duration::from_secs(5)).build().ok()) else { return false };
    let url = std::env::var("ZERD_TELEMETRY_URL").unwrap_or_else(|_| DEFAULT_URL.into());
    let sent = client.post(url).json(&records).send().await.map_or(false, |r| r.status().is_success());
    if sent {
        batch.iter().for_each(|p| drop(std::fs::remove_file(p)));
    }
    sent
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
            println!("When on, one record per finished request: a random install id, zerd version, OS, model name, how the");
            println!("request ended, and counts (rounds, seconds, token totals, files changed, calls and errors per built-in tool).");
            println!("Never: what you typed, file names, commands, error text or code.");
            let waiting = std::fs::read_to_string(queue_file(&dir)).map_or(0, |t| t.lines().count());
            if waiting > 0 {
                println!("{waiting} record(s) are waiting to be sent. They are plain text in {}", queue_file(&dir).display());
            }
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
        let r = record(&events, "id-1", json!({"model": "some/model", "intent": "change", "checks_failed": 1}));
        assert_eq!(r["tools"], json!({"shell": [1, 1], "edit": [1, 0], "other": [1, 0]}));
        assert!(r["status"] == "failed" && r["stop"] == "other" && r["changed"] == 1 && r["checks_failed"] == 1 && r["prompt_tokens"] == 1000 && r["peak_prompt"] == 1000 && r["model"] == "some/model" && r["nudges"]["edit_churn"] == 1);
        let text = r.to_string();
        for private in ["SECRET-PROJECT", "/Users/me", "private_name", "acme_internal", "private.env"] {
            assert!(!text.contains(private), "{private} leaked into {text}");
        }
    }

    #[tokio::test]
    async fn an_opted_in_install_posts_its_records_in_one_batch_and_keeps_them_when_the_send_fails() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let server = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        unsafe { std::env::set_var("ZERD_TELEMETRY_URL", format!("http://{}/api/telemetry", server.local_addr().unwrap())) };
        let events = vec![json!({"event": "task_start", "request": "hello"}), json!({"event": "task_end", "status": "verified", "sec": 3.0, "changed": []})];
        let d = tempfile::tempdir().unwrap();
        let waiting = |d: &Path| std::fs::read_dir(d).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().starts_with("telemetry_")).filter(|e| e.file_name() != "telemetry.json").count();

        // no consent: nothing is written and nothing is sent
        send_if_enabled(d.path(), &events, json!({"model": "some/model"}));
        assert!(waiting(d.path()) == 0 && tokio::time::timeout(std::time::Duration::from_millis(300), server.accept()).await.is_err(), "sent without consent");

        // the server answers 500: the batch stays on disk for a later try
        set_enabled(d.path(), true).unwrap();
        std::fs::write(queue_file(d.path()), format!("{}\n", record(&events, "id", json!({"model": "first"})))).unwrap();
        let server = &server;
        let answer = |code: &'static str| async move {
            let (mut sock, _) = tokio::time::timeout(std::time::Duration::from_secs(5), server.accept()).await.expect("nothing was sent").unwrap();
            let mut got = String::new();
            let mut buf = [0u8; 8192];
            while !got.ends_with(']') {
                let n = sock.read(&mut buf).await.unwrap();
                assert!(n > 0, "connection closed early: {got}");
                got.push_str(&String::from_utf8_lossy(&buf[..n]));
            }
            let _ = sock.write_all(format!("HTTP/1.1 {code}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n").as_bytes()).await;
            got
        };
        let (ok, _) = tokio::join!(flush(d.path()), answer("500 Internal Server Error"));
        assert!(!ok && waiting(d.path()) == 1, "a failed send must keep the records");

        // next request: its own record goes out now, and the failed batch joins once it counts as abandoned
        for e in std::fs::read_dir(d.path()).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().starts_with("telemetry_sending_")) {
            let long_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(STALE_SECS + 60);
            std::fs::File::options().write(true).open(e.path()).unwrap().set_modified(long_ago).unwrap();
        }
        send_if_enabled(d.path(), &events, json!({"model": "second"}));
        let got = answer("204 No Content").await;
        assert!(got.starts_with("POST /api/telemetry") && got.contains("\"model\":\"first\"") && got.contains("\"model\":\"second\"") && !got.contains("hello"), "{got}");
        for _ in 0..50 {
            if waiting(d.path()) == 0 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(40)).await;
        }
        assert_eq!(waiting(d.path()), 0, "delivered records must be removed");
    }

    /// Posts one real record to a running site and expects a yes. Not part of the normal run:
    ///   ZERD_TELEMETRY_E2E=http://localhost:3000/api/telemetry cargo test -q against_a_running_site -- --ignored
    #[tokio::test]
    #[ignore]
    async fn a_real_record_is_accepted_against_a_running_site() {
        let url = std::env::var("ZERD_TELEMETRY_E2E").expect("set ZERD_TELEMETRY_E2E to the endpoint");
        unsafe { std::env::set_var("ZERD_TELEMETRY_URL", url) };
        let d = tempfile::tempdir().unwrap();
        set_enabled(d.path(), true).unwrap();
        let events = vec![
            json!({"event": "task_start", "request": "hello"}),
            json!({"event": "model_call", "ok": true, "sec": 2.5, "tokens": {"prompt_tokens": 20000, "cached_tokens": 0, "completion_tokens": 50}}),
            json!({"event": "tool_call", "tool": "edit", "ok": true, "sec": 0.1}),
            json!({"event": "task_end", "status": "verified", "sec": 3.0, "repairs": 0, "changed": ["a.rs"]}),
        ];
        let install = read_state(d.path())["install"].as_str().unwrap().to_string();
        let rec = record(&events, &install, json!({"model": "e2e/model", "intent": "change", "rounds": 2, "estimate": 3, "approval": "auto", "pi_mode": 1}));
        std::fs::write(queue_file(d.path()), format!("{rec}\n{rec}\n")).unwrap(); // the same record twice: the server must keep one
        assert!(flush(d.path()).await, "the site refused the batch");
    }
}
