//! The program's modes: one-shot, JSON lines, RPC, parallel workers, and the interactive session with its slash commands.
use crate::{uprint, uprintln};
use super::fullscreen;
use super::input::{Completions, Input, Line, COMMANDS};
use super::prompt::{set_asker, Asker};
use super::style::*;
use super::tui::Tui;
use crate::agent::agent::Agent;
use crate::agent::events::AgentEvent;
use crate::agent::persistence::{PersistenceManager, SessionSnapshot};
use crate::agent::session::Session;
use crate::agent::verify::undo;
use crate::client::response::TokenUsage;
use crate::config::loader::{get_data_dir, save_env_key, save_last_model};
use crate::config::{ApprovalPolicy, Config, ConfigHandle};
use crate::safety::approval::{ApprovalManager, ConfirmCallback, Decision};
use crate::tools::builtin::shell::{kill_jobs, jobs_list as JOBS_LIST};
use crate::util::cost::summarize_usage;
use crate::util::re::lit;
use futures::future::BoxFuture;
use serde_json::{json, Value};
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

const MAX_ATTACH_BYTES: u64 = 60_000;

/// A real plan has steps: three or more numbered or bulleted lines in a reply of some length.
pub fn looks_like_plan(text: &str) -> bool {
    let steps = lit(r"(?m)^\s*(?:\d+[.)]|[-*•])\s+\S");
    let (mut n, mut at) = (0, 0);
    while let Some((_, e)) = steps.find(&text[at..]) {
        n += 1;
        at += e.max(1);
    }
    n >= 3 && text.split_whitespace().count() >= 60
}

/// One JSON object per line; a missing file or a line cut off by a crash is skipped, not fatal.
pub fn read_jsonl(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path).map(|t| t.lines().filter_map(|l| serde_json::from_str(l).ok()).collect()).unwrap_or_default()
}

/// Attach `@path` files to the message. Secrets and protected paths are never attached.
pub fn expand_mentions(text: &str, cwd: &Path, forbidden: &[String]) -> (String, Vec<String>) {
    let re = lit(r"(?:^|\s)@([\w./\-]+)");
    let (mut attached, mut blocks, mut at) = (Vec::<String>::new(), Vec::<String>::new(), 0);
    while at < text.len() {
        let Some(c) = re.captures(&text[at..]) else { break };
        let (_, e) = re.find(&text[at..]).unwrap();
        at += e.max(1);
        let rel = c[1].clone().unwrap_or_default();
        let f = crate::util::paths::resolve(&cwd.join(&rel));
        let name = f.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if attached.contains(&rel) || !f.exists() || crate::util::search::is_secret_name(&name) || crate::safety::guard::is_forbidden_path(&rel, forbidden).is_some() {
            continue;
        }
        if f.is_file() && std::fs::metadata(&f).map_or(false, |m| m.len() <= MAX_ATTACH_BYTES) {
            let Ok(body) = std::fs::read(&f) else { continue };
            blocks.push(format!("<file path=\"{rel}\">\n{}\n</file>", String::from_utf8_lossy(&body)));
        } else if f.is_dir() {
            let mut names: Vec<String> = std::fs::read_dir(&f).map(|rd| rd.flatten().filter(|e| !crate::util::search::is_secret_name(&e.file_name().to_string_lossy())).map(|e| format!("{}{}", e.file_name().to_string_lossy(), if e.path().is_dir() { "/" } else { "" })).collect()).unwrap_or_default();
            names.sort();
            names.truncate(100);
            blocks.push(format!("<directory path=\"{rel}\">\n{}\n</directory>", names.join("\n")));
        } else {
            continue;
        }
        attached.push(rel);
    }
    if blocks.is_empty() { (text.to_string(), attached) } else { (format!("{text}\n\n{}", blocks.join("\n\n")), attached) }
}

/// Image files named in what was typed: a path dragged into the terminal, like
/// `/var/folders/.../Screenshot\ 2026-11-18\ at\ 11.48.43 AM.png` (spaces escaped or not), or `@shot.png`.
/// Returns (where it starts in the text, where it ends, the file).
fn find_images(text: &str, cwd: &Path) -> Vec<(usize, usize, PathBuf)> {
    let lower = text.to_ascii_lowercase(); // same byte offsets as `text`
    let ends: Vec<usize> = [".png", ".jpg", ".jpeg", ".gif", ".webp"].iter().flat_map(|e| lower.match_indices(e).map(move |(i, _)| i + e.len())).collect();
    let mut found: Vec<(usize, usize, PathBuf)> = vec![];
    for (start, c) in text.char_indices() {
        let before = text[..start].chars().next_back();
        if !matches!(c, '/' | '~' | '@') || !before.map_or(true, |b| b.is_whitespace() || "'\"".contains(b)) || found.last().map_or(false, |f| start < f.1) {
            continue;
        }
        let mut ends: Vec<usize> = ends.iter().copied().filter(|e| *e > start).collect();
        ends.sort();
        for end in ends {
            // `\ ` is how a terminal escapes a space in a dropped path
            let raw: String = text[start..end].trim_start_matches('@').replace("\\ ", " ").replace('\\', "");
            let path = match raw.strip_prefix("~/") {
                Some(rest) => crate::util::platform::home_dir().unwrap_or_default().join(rest),
                None => cwd.join(&raw),
            };
            if path.is_file() {
                found.push((start, end, path));
                break;
            }
        }
    }
    found
}

/// Copy each image in the message into zerd's data folder (macOS deletes screenshot temp files within minutes, and a
/// resumed session still needs them) and add an `[image: copy]` line, which the provider adapters send as the image.
pub fn attach_images(text: &str, cwd: &Path, dir: &Path) -> (String, Vec<String>) {
    let (mut lines, mut names) = (vec![], vec![]);
    for (_, _, src) in find_images(text, cwd) {
        let name = src.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
        let dest = dir.join(format!("{stamp}-{name}"));
        if std::fs::create_dir_all(dir).is_ok() && std::fs::copy(&src, &dest).is_ok() {
            lines.push(format!("[image: {}]", dest.display()));
            names.push(name);
        }
    }
    if lines.is_empty() { (text.to_string(), names) } else { (format!("{text}\n\n{}", lines.join("\n")), names) }
}

/// A message that opens with a dragged-in file path is a message about that file, not a slash command.
fn starts_with_path(text: &str, cwd: &Path) -> bool {
    let first = text.split_whitespace().next().unwrap_or("");
    find_images(text, cwd).first().map_or(false, |f| f.0 == 0) || (first[1..].contains('/') && Path::new(first).exists())
}

fn copy_to_clipboard(text: &str) -> bool {
    for cmd in [vec!["pbcopy"], vec!["wl-copy"], vec!["xclip", "-selection", "clipboard"]] {
        if which::which(cmd[0]).is_err() {
            continue;
        }
        if let Ok(mut child) = std::process::Command::new(cmd[0]).args(&cmd[1..]).stdin(std::process::Stdio::piped()).spawn() {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
            }
            return child.wait().map_or(false, |s| s.success());
        }
    }
    false
}

/// One line on why a change ended unverified. "No verification commands found" used to be printed for every case,
/// including a budget stop and checks that were simply never run.
fn unverified_why(note: &str) -> &'static str {
    if note.contains("background") {
        "Checks are still running in the background; the result is printed here when they finish."
    } else if note.contains("budget") {
        "Stopped on the token budget, so the change may be incomplete."
    } else if note.contains("only docs") {
        "Only docs changed, so there was nothing to run."
    } else if note.contains("already failing") {
        "A check was already failing before this change, so it proves nothing about it."
    } else {
        "No check exists for these files, so the change was not tested. Verify rules in .zerd/config.toml add one."
    }
}

fn home_short(p: &Path) -> String {
    let home = crate::util::platform::home_dir().unwrap_or_default().to_string_lossy().to_string();
    p.to_string_lossy().replacen(&home, "~", 1)
}

/// Did the person type this "user" message? The agent loop also slips its own nudges in as user turns ("Budget check:
/// 17 of 25 tool rounds used...", "[background job 8 ... failed]"), and a replay should not show those as if you said
/// them. What you typed is in the trace: each turn's `task_start` keeps its request, and a note sent mid-turn is
/// marked "[the user added". With no requests in the trace (an old session) everything shows, as before.
fn typed_by_person(text: &str, requests: &[String]) -> bool {
    requests.is_empty() || text.starts_with("[the user added") || requests.iter().any(|r| !r.is_empty() && text.starts_with(r.as_str()))
}

// ---------------------------------------------------------------------------------------------------- prompts

/// One keypress from a numbered menu. Enter takes the first option, Esc or Ctrl+C the last.
fn pick_key(count: usize) -> usize {
    use crossterm::event::{read, Event, KeyCode, KeyEventKind, KeyModifiers};
    if crossterm::terminal::enable_raw_mode().is_err() {
        return count - 1;
    }
    let choice = loop {
        let Ok(Event::Key(k)) = read() else { continue };
        if k.kind == KeyEventKind::Release {
            continue;
        }
        match k.code {
            KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => break count - 1,
            KeyCode::Char(c) if c.is_ascii_digit() && c != '0' && (c as usize - '1' as usize) < count => break c as usize - '1' as usize,
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => break 0,
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => break count - 1,
            _ => {}
        }
    };
    let _ = crossterm::terminal::disable_raw_mode();
    choice
}

/// A key pasted twice (or ten times) by accident arrives as the same text repeated. Returns one copy and how many
/// there were. Only long keys count, so a short secret like "abab" is left alone.
/// Example: "sk-abc...xyzsk-abc...xyz" -> ("sk-abc...xyz", 2).
pub(crate) fn collapse_repeats(s: &str) -> (String, usize) {
    let n = s.len();
    if n >= 32 {
        for p in 16..=n / 2 {
            if n % p == 0 && s.is_char_boundary(p) && s[..p].repeat(n / p) == s {
                return (s[..p].to_string(), n / p);
            }
        }
    }
    (s.to_string(), 1)
}

/// Read a secret with a dot per character so you can see the paste landed. Enter submits, Esc or Ctrl-C cancels
/// (returns empty). Pasted text is taken whole, and a pasted newline doesn't submit it.
pub(crate) fn read_secret_line(prompt: &str) -> String {
    use crossterm::event::{read, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEventKind, KeyModifiers};
    uprint!("{prompt}");
    let _ = std::io::stdout().flush();
    if crossterm::terminal::enable_raw_mode().is_err() {
        let mut s = String::new();
        let _ = std::io::stdin().read_line(&mut s);
        return s.trim().to_string();
    }
    let _ = crossterm::execute!(std::io::stdout(), EnableBracketedPaste);
    let mut buf = String::new();
    let mut shown = 0usize;
    let draw = |buf: &str, shown: &mut usize| {
        let want = buf.chars().count();
        while *shown < want {
            uprint!("•");
            *shown += 1;
        }
        while *shown > want {
            uprint!("\x08 \x08");
            *shown -= 1;
        }
        let _ = std::io::stdout().flush();
    };
    loop {
        match read() {
            Ok(Event::Paste(t)) => buf.extend(t.chars().filter(|c| !c.is_control())),
            Ok(Event::Key(k)) if k.kind != KeyEventKind::Release => match k.code {
                KeyCode::Enter => break,
                KeyCode::Esc => {
                    buf.clear();
                    break;
                }
                KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                    buf.clear();
                    break;
                }
                KeyCode::Char('u') if k.modifiers.contains(KeyModifiers::CONTROL) => buf.clear(),
                KeyCode::Backspace => {
                    buf.pop();
                }
                KeyCode::Char(c) if !k.modifiers.contains(KeyModifiers::CONTROL) => buf.push(c),
                _ => {}
            },
            Ok(_) => {}
            Err(_) => break,
        }
        draw(&buf, &mut shown);
    }
    let _ = crossterm::execute!(std::io::stdout(), DisableBracketedPaste);
    let _ = crossterm::terminal::disable_raw_mode();
    uprintln!();
    buf.trim().to_string()
}

fn read_plain_line(prompt: &str) -> String {
    uprint!("{prompt}");
    let _ = std::io::stdout().flush();
    let mut s = String::new();
    let _ = std::io::stdin().read_line(&mut s);
    s.trim().to_string()
}

struct TuiAsker {
    tui: Arc<Tui>,
}

impl Asker for TuiAsker {
    fn ask_choice(&self, question: String, options: Vec<String>) -> BoxFuture<'static, usize> {
        let tui = self.tui.clone();
        Box::pin(async move {
            if let Some(fs) = fullscreen::current() {
                return fs.choose(question, options).await;
            }
            tui.pause();
            let mut lines = question.lines();
            let head = lines.next().unwrap_or("");
            let mut text = format!("\n {} {}\n", bold_paint("?", ACCENT), bold_paint(head, FG));
            for l in lines {
                text.push_str(&format!("   {}\n", paint(l, MUTED)));
            }
            for (i, o) in options.iter().enumerate() {
                text.push_str(&format!("   {} {o}\n", bold_paint(&format!("{}.", i + 1), ACCENT)));
            }
            uprint!("{text}");
            uprint!("\x07");
            let _ = std::io::stdout().flush();
            let n = options.len();
            let pick = tokio::task::spawn_blocking(move || pick_key(n)).await.unwrap_or(n - 1);
            tui.resume();
            pick
        })
    }

    fn read_line(&self, prompt: String) -> BoxFuture<'static, String> {
        let tui = self.tui.clone();
        Box::pin(async move {
            if let Some(fs) = fullscreen::current() {
                return fs.ask_line(&prompt, false).await;
            }
            tui.pause();
            let s = tokio::task::spawn_blocking(move || read_plain_line(&prompt)).await.unwrap_or_default();
            tui.resume();
            s
        })
    }

    fn say(&self, line: String) {
        self.tui.println(&line);
    }
}

fn confirm_callback(tui: Arc<Tui>) -> ConfirmCallback {
    Arc::new(move |c| {
        let tui = tui.clone();
        Box::pin(async move {
            if let Some(fs) = fullscreen::current() {
                let (q, always, _title) = tui.show_confirmation(&c);
                let choice = fs.choose(q, vec!["Yes".into(), always, "No, and tell me what to do differently".into()]).await;
                return if choice < 2 {
                    tui.approved(&c);
                    Decision { approved: true, always: choice == 1, feedback: None }
                } else {
                    let fb = fs.ask_line("What should I do instead? (Enter to skip)", false).await;
                    Decision { approved: false, always: false, feedback: (!fb.is_empty()).then(|| format!("The user said: {fb}")) }
                };
            }
            tui.pause();
            let (q, always, _title) = tui.show_confirmation(&c);
            tui.set_phase("Waiting for your answer");
            uprint!("\n {} {}\n   {} Yes\n   {} {always}\n   {} No, and tell me what to do differently\n\x07", bold_paint("?", ACCENT), bold_paint(&q, FG), bold_paint("1.", ACCENT), bold_paint("2.", ACCENT), bold_paint("3.", ACCENT));
            let _ = std::io::stdout().flush();
            let choice = tokio::task::spawn_blocking(|| pick_key(3)).await.unwrap_or(2);
            let d = if choice < 2 {
                tui.approved(&c);
                Decision { approved: true, always: choice == 1, feedback: None }
            } else {
                let fb = tokio::task::spawn_blocking(|| read_plain_line("What should I do instead? (Enter to skip) ")).await.unwrap_or_default();
                Decision { approved: false, always: false, feedback: (!fb.is_empty()).then(|| format!("The user said: {fb}")) }
            };
            tui.resume();
            d
        })
    })
}


/// Shift+Tab: next approval mode. Entering plan mode remembers where to come back to.
fn cycle_approval(mgr: &Arc<ApprovalManager>, config: &ConfigHandle, before_plan: &Mutex<ApprovalPolicy>) {
    let now = mgr.policy();
    let next = MODES.iter().position(|m| *m == now).map_or(MODES[0], |i| MODES[(i + 1) % MODES.len()]);
    if next == ApprovalPolicy::Plan {
        *before_plan.lock().unwrap() = if now != ApprovalPolicy::Plan { now } else { ApprovalPolicy::OnRequest };
    }
    mgr.set_policy(next);
    config.update(|c| c.approval = next);
}

/// Footer: the things you act on first (mode, model, context), details after, fitted to the width.
fn toolbar_text(config: &ConfigHandle, sess: Option<Arc<Session>>, width: usize) -> String {
        let cfg = config.get();
        let mut parts: Vec<String> = vec![];
        let running = JOBS_LIST().iter().filter(|j| !j.done()).count();
        if running > 0 {
            parts.push(format!("⚙ {running} running (/jobs)"));
        }
        if let Some(s) = &sess {
            parts.push(s.approval_manager.policy().value().to_string());
            if let Some(e) = &cfg.reasoning_effort {
                parts.push(format!("effort {e}"));
            }
        }
        parts.push(sess.as_ref().and_then(|s| s.client.last_model()).unwrap_or_else(|| cfg.model.name.clone()));
        if let Some(s) = &sess {
            let used = s.context_manager.lock().unwrap().context_used();
            let window = s.client.registry.get(&cfg.model.name).context_length; // what the agent compacts against
            if used > 0 {
                parts.push(format!("ctx {}  {}k/{}k", context_bar(100.0 * used as f64 / window as f64, 12), used / 1000, window / 1000));
            }
            if let Some(c) = summarize_usage(&s.tracer.snapshot(), &cfg.pricing)["est_cost_usd"].as_f64() {
                parts.push(format!("${c}"));
            }
            if cfg.sandbox != "off" {
                parts.push(match crate::safety::sandbox::mode(&cfg) {
                    "on" => "sandboxed",
                    "unavailable" => "no sandbox",
                    _ => "",
                }
                .to_string());
            }
        }
        if let Some(p) = &cfg.profile {
            parts.push(p.clone());
        }
        let text = parts.into_iter().filter(|p| !p.is_empty()).collect::<Vec<_>>().join("  ·  ");
        let w = width.saturating_sub(3);
        if text.chars().count() <= w { text } else { format!("{}…", text.chars().take(w - 1).collect::<String>()) }
}

// ---------------------------------------------------------------------------------------------------- the app

pub struct Cli {
    pub agent: Option<Agent>,
    pub resume_id: Option<String>,
    pub config: ConfigHandle,
    pub tui: Arc<Tui>,
    pub last_report: Option<Value>,
    before_plan: Arc<Mutex<ApprovalPolicy>>,
    /// The current session, for the full-screen footer and Shift+Tab (they run on the UI thread).
    live: Arc<Mutex<Option<Arc<Session>>>>,
}

/// Test seam: the replies the next agent made by `new_agent` will give.
#[cfg(test)]
pub(crate) static TEST_SCRIPT: Mutex<Option<Vec<Vec<crate::client::response::StreamEvent>>>> = Mutex::new(None);

pub(crate) const MODES: [ApprovalPolicy; 4] = [ApprovalPolicy::OnRequest, ApprovalPolicy::AutoEdit, ApprovalPolicy::Auto, ApprovalPolicy::Plan];

impl Cli {
    pub fn new(config: Config) -> Cli {
        let tui = Tui::new(&config.cwd);
        Cli { agent: None, resume_id: None, config: ConfigHandle::new(config), tui, last_report: None, before_plan: Arc::new(Mutex::new(ApprovalPolicy::OnRequest)), live: Arc::new(Mutex::new(None)) }
    }

    fn agent(&mut self) -> &mut Agent {
        self.agent.as_mut().expect("agent")
    }

    pub(crate) async fn new_agent(&mut self, confirm: Option<ConfirmCallback>) {
        let session = Session::create(self.config.clone()).await;
        *session.approval_manager.confirmation_callback.lock().unwrap() = confirm;
        #[cfg(test)]
        if let Some(turns) = TEST_SCRIPT.lock().unwrap().take() {
            *session.client.script.lock().unwrap() = Some(turns.into()); // tests give the new agent its model replies here
        }
        self.agent = Some(Agent::with_session(self.config.clone(), session));
        self.publish_session();
    }

    /// Tell the full-screen front end which session and steering queue are live.
    fn publish_session(&mut self) {
        let (session, steering) = {
            let a = self.agent();
            (a.session.clone(), a.steering.clone())
        };
        *self.live.lock().unwrap() = Some(session);
        if let Some(fs) = fullscreen::current() {
            fs.set_steer(steering);
        }
    }

    pub async fn run_single(&mut self, message: &str, report_path: Option<&Path>) -> Option<String> {
        // On a real terminal, ask when something needs approval instead of refusing it. Piped or CI runs have nobody to
        // answer, so they keep failing closed.
        let tty = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
        let ask = tty.then(|| confirm_callback(self.tui.clone()));
        if tty {
            set_asker(Some(Arc::new(TuiAsker { tui: self.tui.clone() })));
        }
        self.new_agent(ask).await;
        let result = self.process_message(message).await;
        if let Some(p) = report_path {
            self.write_report(p, result.as_deref());
        }
        self.agent().close().await;
        result
    }

    /// One event per stdout line, for CI and scripts. No TUI, so pair it with --approval auto.
    pub async fn run_json(&mut self, message: &str, report_path: Option<&Path>) -> bool {
        self.new_agent(None).await;
        let mut ok = true;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<AgentEvent>();
        let agent = self.agent.as_mut().unwrap();
        {
            let run = agent.run(message, tx);
            tokio::pin!(run);
            let mut report = None;
            let handle = |ev: AgentEvent, ok: &mut bool, report: &mut Option<Value>| {
                uprintln!("{}", ev.to_value());
                let _ = std::io::stdout().flush();
                match ev {
                    AgentEvent::AgentError { .. } => *ok = false,
                    AgentEvent::AgentEnd { report: r, .. } => *report = Some(r),
                    _ => {}
                }
            };
            loop {
                tokio::select! {
                    _ = &mut run => break,
                    Some(ev) = rx.recv() => handle(ev, &mut ok, &mut report),
                }
            }
            while let Ok(ev) = rx.try_recv() {
                handle(ev, &mut ok, &mut report);
            }
            self.last_report = report;
        }
        if let Some(p) = report_path {
            self.write_report(p, None);
        }
        self.agent().close().await;
        ok && !matches!(self.last_report.as_ref().and_then(|r| r["status"].as_str()), Some("failed" | "verification_failed"))
    }

    /// Drive zerd from another program: one JSON command per stdin line, events and replies on stdout.
    ///     {"type": "prompt", "message": "fix the bug", "id": "1"}   start a request (one at a time)
    ///     {"type": "steer", "message": "use python 3.12"}           note read after the current tool calls
    ///     {"type": "abort"}                                          stop the running request
    ///     {"type": "get_state"}                                      is it running, which model
    /// Every command gets {"type": "response", "command": ..., "success": ..., "id": ...}.
    pub async fn run_rpc(&mut self) {
        let (line_tx, line_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        std::thread::spawn(move || {
            let stdin = std::io::stdin();
            let mut s = String::new();
            while stdin.read_line(&mut s).map_or(false, |n| n > 0) {
                let _ = line_tx.send(std::mem::take(&mut s));
            }
        });
        self.run_rpc_lines(line_rx).await;
    }

    /// The command loop behind `run_rpc`, reading its lines from a channel so tests can feed it.
    pub(crate) async fn run_rpc_lines(&mut self, mut line_rx: tokio::sync::mpsc::UnboundedReceiver<String>) {
        self.new_agent(None).await;
        let agent = self.agent.take().unwrap();
        let steering = agent.steering.clone();
        let session_id = agent.session.session_id();
        let agent: Arc<tokio::sync::Mutex<Agent>> = Arc::new(tokio::sync::Mutex::new(agent));
        let emit = |v: Value| {
            uprintln!("{v}");
            let _ = std::io::stdout().flush();
        };
        let mut running: Option<tokio::task::JoinHandle<()>> = None;
        let model = self.config.get().model.name.clone();
        while let Some(line) = line_rx.recv().await {
            if line.trim().is_empty() {
                continue;
            }
            let cmd: Value = match serde_json::from_str::<Value>(&line) {
                Ok(v) if v["type"].is_string() => v,
                _ => {
                    emit(json!({"type": "response", "success": false, "error": "each line must be a JSON object with a type"}));
                    continue;
                }
            };
            let (kind, rid) = (cmd["type"].as_str().unwrap().to_string(), cmd.get("id").cloned().unwrap_or(Value::Null));
            let busy = running.as_ref().map_or(false, |r| !r.is_finished());
            let mut error: Option<&str> = None;
            match kind.as_str() {
                "prompt" => {
                    if busy {
                        error = Some("a request is already running; steer or abort it");
                    } else if cmd["message"].as_str().map_or(true, str::is_empty) {
                        error = Some("prompt needs a message");
                    } else {
                        let (message, agent) = (cmd["message"].as_str().unwrap().to_string(), agent.clone());
                        running = Some(tokio::spawn(async move {
                            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<AgentEvent>();
                            let mut guard = agent.lock().await;
                            let run = guard.run(&message, tx);
                            tokio::pin!(run);
                            loop {
                                tokio::select! {
                                    _ = &mut run => break,
                                    Some(ev) = rx.recv() => { uprintln!("{}", ev.to_value()); let _ = std::io::stdout().flush(); }
                                }
                            }
                            while let Ok(ev) = rx.try_recv() {
                                uprintln!("{}", ev.to_value());
                            }
                        }));
                    }
                }
                "steer" => {
                    if !busy {
                        error = Some("nothing is running to steer; send a prompt");
                    } else {
                        steering.lock().unwrap().push(cmd["message"].as_str().unwrap_or("").to_string());
                    }
                }
                "abort" => {
                    if busy {
                        if let Some(r) = &running {
                            r.abort();
                        }
                        let _ = running.take().unwrap().await;
                        agent.lock().await.on_interrupt();
                        emit(json!({"type": "aborted"}));
                    }
                }
                "get_state" => emit(json!({"type": "state", "running": busy, "model": model, "session": session_id})),
                other => {
                    let _ = other;
                    error = Some("unknown command");
                }
            }
            let mut resp = json!({"type": "response", "command": kind, "id": rid, "success": error.is_none()});
            if let Some(e) = error {
                resp["error"] = json!(if e == "unknown command" { format!("unknown command: {kind}") } else { e.to_string() });
            }
            emit(resp);
        }
        if let Some(r) = running {
            let _ = r.await; // stdin closed: let the last request finish
        }
        agent.lock().await.close().await;
    }

    pub async fn run_orchestrated(&mut self, message: &str, graph: Option<&Path>, apply: bool, report_path: Option<&Path>) -> bool {
        use crate::agent::orchestrator::{parse_graph, plan, Orchestrator};
        let cfg = (*self.config.get()).clone();
        let nodes = match graph {
            Some(g) => match std::fs::read_to_string(g).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).map(|v| parse_graph(&v)) {
                Some(Ok(n)) => n,
                Some(Err(e)) => {
                    eprintln!("{}", paint(&format!("Invalid task graph: {}", e.0), CORAL));
                    return false;
                }
                None => {
                    eprintln!("{}", paint("Could not read the task graph", CORAL));
                    return false;
                }
            },
            None => {
                let tracer = Arc::new(crate::util::trace::Tracer::new("detached", None));
                let client = Arc::new(crate::client::llm_client::LLMClient::new(self.config.clone(), tracer));
                client.initialize().await;
                plan(&client, message).await
            }
        };
        uprintln!("{} {}", bold("Parallel plan:"), nodes.iter().map(|n| n.id.clone()).collect::<Vec<_>>().join(", "));
        let workers = cfg.max_parallel_agents;
        let result = match Orchestrator::new(cfg, workers, None).run(message, nodes, apply).await {
            Ok(r) => r,
            Err(e) => {
                eprintln!("{}", paint(&e.0, CORAL));
                return false;
            }
        };
        let mut summary = result.clone();
        summary.as_object_mut().map(|o| o.remove("tasks"));
        uprintln!("{}", serde_json::to_string_pretty(&summary).unwrap_or_default());
        for t in result["tasks"].as_array().into_iter().flatten() {
            uprintln!("  {:<9}{:<14}{}s  {}", t["status"].as_str().unwrap_or(""), t["id"].as_str().unwrap_or(""), t["seconds"], t["note"].as_str().unwrap_or(""));
        }
        if let Some(p) = report_path {
            let changed: std::collections::BTreeSet<String> = result["tasks"].as_array().into_iter().flatten().flat_map(|t| t["changed"].as_array().cloned().unwrap_or_default()).filter_map(|v| v.as_str().map(String::from)).collect();
            let _ = std::fs::write(p, serde_json::to_string_pretty(&json!({"report": {"status": result["status"], "repairs": 0, "changed_files": changed, "checks": result["combined_checks"]}, "parallel": result, "model_calls": null})).unwrap_or_default());
        }
        result["status"] == "ok"
    }

    /// Metrics for one run, for the benchmark runner and for debugging.
    fn write_report(&mut self, path: &Path, response: Option<&str>) {
        let report = self.last_report.clone();
        let Some(agent) = self.agent.as_ref() else { return };
        let ev = agent.session.tracer.snapshot();
        let calls: Vec<&Value> = ev.iter().filter(|e| e["event"] == "model_call").collect();
        let usage = agent.session.context_manager.lock().unwrap().total_usage.to_value();
        let mut models: Vec<String> = calls.iter().filter(|c| c["ok"] == true).filter_map(|c| c["model"].as_str().map(String::from)).collect();
        models.sort();
        models.dedup();
        let tool_calls: Vec<&Value> = ev.iter().filter(|e| e["event"] == "tool_call").collect();
        let doc = json!({
            "report": report, "final_response": response, "session_id": agent.session.session_id(), "turns": agent.session.turns(),
            "model_calls": calls.len(), "model_call_failures": calls.iter().filter(|c| c["ok"] != true).count(), "models_used": models,
            "tool_calls": tool_calls.len(), "tool_failures": tool_calls.iter().filter(|e| e["ok"] != true).count(),
            "ttft_sec": calls.iter().filter_map(|c| c.get("ttft").filter(|t| !t.is_null()).cloned()).collect::<Vec<_>>(), "usage": usage,
        });
        let _ = std::fs::write(path, serde_json::to_string_pretty(&doc).unwrap_or_default());
    }

    /// Shift+Tab in the old UI: ask -> auto-edit -> auto -> plan -> ask. Never lands on yolo by accident.
    #[allow(dead_code)]
    fn cycle_mode(&mut self) {
        let mgr = self.agent().session.approval_manager.clone();
        cycle_approval(&mgr, &self.config, &self.before_plan);
    }

    fn toolbar(&self) -> String {
        toolbar_text(&self.config, self.agent.as_ref().map(|a| a.session.clone()), term_width())
    }

    pub async fn run_interactive(&mut self) {
        let cfg = self.config.get();
        let confirm = confirm_callback(self.tui.clone());
        self.new_agent(Some(confirm)).await;
        set_asker(Some(Arc::new(TuiAsker { tui: self.tui.clone() })));
        let reg = self.agent().session.client.registry.clone();
        let models = move || reg.all().into_iter().map(|m| m.id).collect::<Vec<_>>();
        let skills = Arc::new(Mutex::new(self.agent().session.skills.iter().filter(|k| !k.always).map(|k| (k.name.clone(), k.description.clone())).collect::<Vec<_>>()));
        let history = Some(get_data_dir().join("input_history"));

        // a real terminal gets the full-screen layout; ZERD_PLAIN=1 (or a pipe) keeps the plain scrolling one
        let mut fs = None;
        if fullscreen::wanted() {
            let (config, live) = (self.config.clone(), self.live.clone());
            let (title_cfg, footer_cfg) = (self.config.clone(), self.config.clone());
            let comp = Arc::new(Completions::new(cfg.cwd.clone(), cfg.forbidden_paths.clone(), Box::new(models.clone()), skills.clone()));
            let title = Arc::new(move || {
                let c = title_cfg.get();
                format!("{}  ·  {}", c.model.name, home_short(&c.cwd))
            });
            let footer = Arc::new(move || toolbar_text(&footer_cfg, live.lock().unwrap().clone(), crossterm::terminal::size().map_or(100, |s| s.0 as usize)));
            match fullscreen::start(self.tui.clone(), title, footer, comp, history.clone()) {
                Ok(f) => {
                    let (mgr_live, bp) = (self.live.clone(), self.before_plan.clone());
                    f.set_cycle(Arc::new(move || {
                        if let Some(s) = mgr_live.lock().unwrap().clone() {
                            cycle_approval(&s.approval_manager, &config, &bp);
                        }
                    }));
                    fs = Some(f);
                    self.publish_session();
                }
                Err(e) => uprintln!("{}", paint(&format!("full-screen mode unavailable ({e}); using the plain terminal"), AMBER)),
            }
        }
        let mut welcome = vec![
            format!("model: {}   approval: {}", cfg.model.name, cfg.approval.value()),
            format!("cwd: {}{}", home_short(&cfg.cwd), cfg.profile.as_ref().map(|p| format!("   profile: {p}")).unwrap_or_default()),
            "/help for commands · @file to attach · !cmd to run a command · Shift+Enter (or Ctrl+J) for a newline".into(),
        ];
        // a folder zerd has never worked in gets a nudge toward a first task; after that the welcome stays short
        if crate::agent::persistence::PersistenceManager::new().latest_for(&cfg.cwd.to_string_lossy()).is_none() {
            welcome.push("new here? try \"explain how this repo is laid out\" · /model switches models · `zerd doctor` if something's off".into());
        }
        self.tui.print_welcome("zerd", &welcome);
        let input = if fs.is_none() {
            let inp = Input::new(cfg.cwd.clone(), cfg.forbidden_paths.clone(), Box::new(models), history);
            *inp.skills.lock().unwrap() = skills.lock().unwrap().clone();
            Some(inp)
        } else {
            None
        };
        if let Some(id) = self.resume_id.clone() {
            if self.resume(&id).await {
                uprintln!("{}", paint(&format!("Resumed {id} ({} turns). Carry on where you left off.", self.agent().session.turns()), MINT));
            } else {
                uprintln!("{}", paint(&format!("No session with id {id}; starting a new one."), CORAL));
            }
        }
        loop {
            let read = match (&fs, &input) {
                (Some(f), _) => f.read().await,
                (None, Some(inp)) => {
                    uprintln!("{}", paint(&format!(" {}", self.toolbar()), MUTED));
                    inp.read(format!(" {}", bold_paint("✦ ", ACCENT))).await
                }
                _ => Line::Eof,
            };
            let line = match read {
                Line::Text(t) => t,
                Line::Interrupted => {
                    uprintln!("{}", paint("  (Ctrl+C) type /exit to quit", MUTED));
                    continue;
                }
                Line::Eof => break,
            };
            let user_input = line.trim().trim_start_matches('▍').trim().to_string(); // a line copied from the terminal still carries the prompt glyph
            if user_input.is_empty() {
                continue;
            }
            if fs.is_some() {
                self.tui.echo_sent(&user_input);
            }
            if user_input.starts_with('/') && !starts_with_path(&user_input, &self.config.get().cwd) {
                if !self.handle_command(&user_input).await {
                    break;
                }
                continue;
            }
            if user_input.starts_with('!') {
                self.run_bang(&user_input).await;
                continue;
            }
            let cfg = self.config.get();
            let (text, images) = attach_images(&user_input, &cfg.cwd, &get_data_dir().join("images"));
            let (text, mut attached) = expand_mentions(&text, &cfg.cwd, &cfg.forbidden_paths);
            attached.retain(|a| crate::client::adapters::image_type(a).is_none()); // the image went as a picture, not as text
            attached.extend(images);
            if !attached.is_empty() {
                uprintln!("{}", paint(&format!("  attached: {}", attached.join(", ")), MUTED));
            }
            if !self.confirm_task(&user_input).await {
                continue;
            }
            self.run_turn(&text).await;
        }
        self.autosave();
        let (sid, turns) = {
            let a = self.agent();
            (a.session.session_id(), a.session.turns())
        };
        self.agent().close().await;
        set_asker(None);
        if let Some(f) = &fs {
            f.stop(); // back to the real terminal, with the conversation left on it
        }
        uprintln!("\n{}", paint("Goodbye!", MUTED));
        if turns > 0 {
            uprintln!("{}{}", paint("Resume this session:  ", MUTED), bold(&format!("zerd --resume {sid}")));
            uprintln!("{}{}", paint("Or the newest one in this folder:  ", MUTED), bold("zerd --continue"));
        }
    }

    /// Run one request, with Ctrl+C cancelling the agent and not the app.
    async fn run_turn(&mut self, text: &str) -> bool {
        // full-screen mode gets Ctrl+C as a key, not a signal
        let cancel = fullscreen::current().map(|f| f.begin_turn());
        let result = {
            let fut = self.process_message(text);
            tokio::pin!(fut);
            tokio::select! {
                r = &mut fut => Some(r),
                _ = async {
                    match &cancel {
                        Some(n) => n.notified().await,
                        None => {
                            let _ = tokio::signal::ctrl_c().await;
                        }
                    }
                } => None,
            }
        };
        if let Some(f) = fullscreen::current() {
            f.end_turn();
        }
        if result.is_none() {
            self.tui.end_turn();
            uprintln!("\n{}", paint("Interrupted", AMBER));
            self.agent().on_interrupt();
        }
        self.autosave();
        // the turn ended before a steering note could land: run it as the next request
        let notes = std::mem::take(&mut *self.agent().steering.lock().unwrap());
        self.tui.end_turn();
        for n in notes {
            Box::pin(self.run_turn(&n)).await;
        }
        result.is_none()
    }

    /// `/loop <task>`: no approval prompts and no round limit. First a turn writes a brief file
    /// (.zerd/loop/<time>.md) holding the goal, the exact "done when" list and a progress log.
    /// Every later turn works from that file and flips `STATUS: DONE` only when each "done when"
    /// line is proven. Example: `/loop make the parser tests pass` -> brief says "done when
    /// `cargo test parser` exits 0" -> rounds repeat until it does. Ctrl+C is the only other way out.
    async fn run_loop(&mut self, task: &str) {
        let mgr = self.agent().session.approval_manager.clone();
        let before = mgr.policy();
        mgr.set_policy(ApprovalPolicy::Auto); // nobody to ask mid-loop, so approve what is safe to approve
        let rel = format!(".zerd/loop/{}.md", chrono::Local::now().format("%Y%m%d-%H%M%S"));
        let file = self.config.get().cwd.join(&rel);
        let mut stop = "interrupted".to_string();
        // Pictures go out once, with the brief. Sending them every round would re-bill them on every call,
        // so the brief turn writes down what they show and later rounds work from that text.
        let shots = task.lines().filter(|l| l.trim().starts_with("[image: ")).count();
                let see = if shots > 0 { "\nThe attached picture(s) will not be shown again, so describe in # Start everything in them that the work depends on (text, layout, colours, error messages)." } else { "" };
        let brief = format!(
            "Do not change any code yet. Look around the repo, then write the file {rel} with these sections:\n\
             # Goal\n(the task in one or two sentences, with every detail from it that the work needs)\n\
             # Start\n(what is true right now: what exists, what fails, what you found)\n\
             # Done when\n(a checklist of results that can be proven by running something, each with the exact command and what it must show)\n\
             # Progress\n(empty for now)\n\
             STATUS: IN PROGRESS{see}\n\nTask: {task}"
        );
        if !self.run_turn(&brief).await {
            if shots > 0 {
                self.agent().compact_now().await; // the brief file now carries the pictures, so drop them from the history
            }
            let prompt = format!(
                "Continue the loop in {rel}. Read it, do the next part of the work, then add a line under # Progress saying what you did and what is left. \
                 When every 'Done when' item is proven by actually running its command, change the last line to `STATUS: DONE`. Not before."
            );
            let mut round = 0;
            loop {
                round += 1;
                uprintln!("{}", paint(&format!("  loop round {round}  ({rel})"), MUTED));
                if self.run_turn(&prompt).await {
                    break;
                }
                let status = self.last_report.as_ref().and_then(|r| r["status"].as_str()).unwrap_or("").to_string();
                let done = std::fs::read_to_string(&file).map_or(false, |t| t.lines().any(|l| l.trim() == "STATUS: DONE"));
                if done && !matches!(status.as_str(), "failed" | "verification_failed") {
                    stop = "finished: every 'Done when' item is marked proven".into();
                    break;
                }
                // the file keeps the plan, so older rounds are dead weight in every later call
                let (used, window) = {
                    let cm = self.agent().session.context_manager.lock().unwrap();
                    (cm.context_used(), cm.context_window)
                };
                if used * 2 > window {
                    self.agent().compact_now().await;
                }
            }
        }
        mgr.set_policy(before);
        uprintln!("{}", paint(&format!("  loop {stop}"), if stop.starts_with("finished") { MINT } else { AMBER }));
    }

    /// `!cmd` runs a command for the user and shows the output to the model; `!!cmd` keeps it private.
    async fn run_bang(&mut self, line: &str) {
        let mut private = line.starts_with("!!");
        let cmd = line.trim_start_matches('!').trim().to_string();
        if cmd.is_empty() {
            return;
        }
        let cfg = self.config.get();
        let argv = crate::util::platform::shell_argv(&cmd);
        let mut sh = tokio::process::Command::new(&argv[0]);
        sh.args(&argv[1..]).current_dir(&cfg.cwd).env_clear().envs(crate::tools::builtin::shell::build_environment(&cfg)).kill_on_drop(true);
        let out = tokio::time::timeout(std::time::Duration::from_secs(120), crate::util::detach::detach(&mut sh).output()).await;
        let (text, code) = match out {
            Ok(Ok(o)) => (format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)), o.status.code().unwrap_or(-1)),
            Ok(Err(e)) => (e.to_string(), -1),
            Err(_) => ("[timed out after 120s]".to_string(), -1),
        };
        uprintln!("{}", text.trim_end());
        if !private && crate::safety::guard::violation("shell", &json!({"command": cmd}), &cfg).is_some() {
            uprintln!("{}", paint("  (protected by repository rules: output not shared with the model)", MUTED));
            private = true;
        }
        if !private {
            self.agent().session.context_manager.lock().unwrap().add_user_message(&format!("I ran `{cmd}` (exit {code}):\n{}", text.chars().take(8000).collect::<String>()));
            uprintln!("{}", paint("  (output added to the conversation)", MUTED));
        }
    }

    fn print_heaviest(&self, u: &Value) {
        let h = &u["heaviest"];
        if u["calls"].as_i64().unwrap_or(0) < 6 || h["top"].as_array().map_or(true, |t| t.is_empty()) || h["tool_result_carried"].as_i64().unwrap_or(0) < 5000 {
            return; // tiny tasks have nothing worth tuning
        }
        let share = (100.0 * h["tool_result_carried"].as_f64().unwrap_or(0.0) / u["prompt_tokens"].as_f64().unwrap_or(1.0).max(1.0)).round().min(100.0);
        uprintln!("{}", paint(&format!("    {share}% of the input tokens were earlier tool results re-sent. Biggest:"), MUTED));
        for r in h["top"].as_array().unwrap().iter().take(3) {
            let (t, c) = (r["tokens"].as_i64().unwrap_or(0), r["carried"].as_i64().unwrap_or(0));
            uprintln!("{}", paint(&format!("      {} {}: {t} tokens, re-sent {}x = {c}", r["tool"].as_str().unwrap_or(""), r["target"].as_str().unwrap_or(""), c / t.max(1)), MUTED));
        }
        if let Some(rep) = h["repeated"].as_array().filter(|r| !r.is_empty()) {
            uprintln!("{}", paint(&format!("    repeated: {}", rep.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("; ")), MUTED));
        }
    }

    fn print_report(&self, report: &Value, elapsed: f64) {
        let u = &report["usage"];
        self.tui.turn_summary(elapsed, u);
        let quiet = self.tui.quiet();
        let w = &u["cache_waste"];
        if w["misses"].as_i64().unwrap_or(0) > 0 && !quiet {
            uprintln!("{}", paint(&format!("    cache misses: {} tokens re-billed ({})", w["missed_tokens"], w["reasons"].as_array().map(|r| r.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")).unwrap_or_default()), MUTED));
        }
        if !quiet {
            self.print_heaviest(u);
        }
        if let Some(un) = report["grounding"]["unverified"].as_array().filter(|a| !a.is_empty()) {
            uprintln!("{}", paint(&format!("  ⚠ {} cited reference(s) could not be verified: {}", un.len(), un.iter().take(4).filter_map(Value::as_str).collect::<Vec<_>>().join("; ")), AMBER));
        }
        let status = report["status"].as_str().unwrap_or("");
        if status == "done_no_changes" {
            return;
        }
        let color = match status {
            "verified" => MINT,
            "unverified" => AMBER,
            _ => CORAL,
        };
        let checks = report["checks"].as_array().cloned().unwrap_or_default();
        let files = report["changed_files"].as_array().map_or(0, Vec::len);
        if quiet {
            let bad: Vec<&Value> = checks.iter().filter(|c| c["ok"] != true).collect();
            let review = report["review"]["verdict"].as_str().map(|v| format!("  ·  review {}", v.to_lowercase())).unwrap_or_default();
            let tail = format!("{}{review}", if !checks.is_empty() && bad.is_empty() { format!("  ·  {} checks ok", checks.len()) } else { String::new() });
            uprintln!("  {}  {}", paint(status, color), paint(&format!("{files} files{tail}  ·  /verify checks  ·  /undo reverts"), MUTED));
            for c in bad {
                uprintln!("    {} {}", paint("✗", CORAL), c["command"].as_str().unwrap_or(""));
            }
            if status == "unverified" {
                uprintln!("{}", paint(&format!("    {}", unverified_why(report["status_note"].as_str().unwrap_or(""))), MUTED));
            }
            return;
        }
        if report["undo"] == true {
            uprintln!("{}", paint("    /undo puts these files back", MUTED));
        }
        uprintln!("  {}  files changed: {files}  repairs: {}", paint(&format!("Result: {status}"), color), report["repairs"]);
        for c in &checks {
            let ok = c["ok"] == true;
            uprintln!("    {} {} ({}s)", paint(if ok { "✓" } else { "✗" }, if ok { MINT } else { CORAL }), c["command"].as_str().unwrap_or(""), c["seconds"]);
        }
        if status == "verified" && !checks.is_empty() && !checks.iter().any(|c| lit(r"test|spec|pytest|vitest|jest|playwright").is_match(c["command"].as_str().unwrap_or(""))) {
            uprintln!("{}", paint("    These checks (types/lint) don't exercise the changed behaviour, so \"verified\" does not mean the bug is fixed.", MUTED));
        }
        if let Some(v) = report["review"]["verdict"].as_str() {
            uprintln!("    review: {v}{}", if v == "PASS" { "" } else { " (see findings above)" });
        }
        if status == "unverified" {
            uprintln!("{}", paint(&format!("    {}", unverified_why(report["status_note"].as_str().unwrap_or(""))), MUTED));
        }
    }

    fn snapshot(&mut self) -> SessionSnapshot {
        let cwd = self.config.get().cwd.to_string_lossy().to_string();
        let a = self.agent();
        let sess = a.session.clone();
        let (msgs, usage) = {
            let cm = sess.context_manager.lock().unwrap();
            (cm.get_messages(), cm.total_usage.clone())
        };
        let first = msgs.iter().find(|m| m["role"] == "user" && m["content"].as_str().map_or(false, |c| !c.is_empty())).and_then(|m| m["content"].as_str()).unwrap_or("");
        let title: String = first.split("\n\n---\n").next().unwrap_or("").trim().replace('\n', " ").chars().take(70).collect(); // drop the playbooks appended to a request
        let todos = sess
            .tool_registry
            .get("todos")
            .and_then(|t| t.as_any().downcast_ref::<crate::tools::builtin::todo::TodosTool>().map(|t| t.items()))
            .map(|items| Value::Object(items.into_iter().map(|t| (t.id, json!({"content": t.content, "done": t.done}))).collect()))
            .unwrap_or(json!({}));
        SessionSnapshot {
            session_id: sess.session_id(),
            created_at: sess.created_at.lock().unwrap().clone(),
            updated_at: sess.updated_at.lock().unwrap().clone(),
            turn_count: sess.turns(),
            messages: msgs,
            total_usage: usage,
            cwd,
            title,
            todos,
        }
    }

    /// Written after every turn, so a crash or closed terminal still leaves something to resume.
    pub(crate) fn autosave(&mut self) {
        if self.agent.as_ref().map_or(false, |a| a.session.turns() > 0) {
            let snap = self.snapshot();
            PersistenceManager::new().save_session(&snap);
        }
    }

    /// Replace the live session with a saved one (messages, usage, trace, checklist).
    async fn restore_into_session(&mut self, snap: &SessionSnapshot, with_trace: bool) {
        let confirm = self.agent().session.approval_manager.confirmation_callback.lock().unwrap().clone();
        let session = Session::create(self.config.clone()).await;
        session.set_session_id(&snap.session_id);
        *session.created_at.lock().unwrap() = snap.created_at.clone();
        *session.updated_at.lock().unwrap() = snap.updated_at.clone();
        session.set_turns(snap.turn_count);
        {
            let mut cm = session.context_manager.lock().unwrap();
            cm.total_usage = snap.total_usage.clone();
            for m in &snap.messages {
                match m["role"].as_str().unwrap_or("") {
                    "user" => cm.add_user_message(m["content"].as_str().unwrap_or("")),
                    "assistant" => {
                        let item = crate::context::MessageItem::from_value(m);
                        cm.add_assistant_message(Some(&item.content), (!item.tool_calls.is_empty()).then(|| item.tool_calls.clone()), Some(&item.reasoning), Some(&item.reasoning_model));
                    }
                    "tool" => cm.add_tool_result(m["tool_call_id"].as_str().unwrap_or(""), m["content"].as_str().unwrap_or("")),
                    _ => {}
                }
            }
            cm.repair_orphans(); // a session saved mid-turn may end on an unanswered tool call
        }
        if with_trace {
            // the trace brings back the footer's cost, /cost and the cache stats; the context bar is the last model call's prompt
            let events = read_jsonl(&get_data_dir().join("traces").join(format!("{}.jsonl", snap.session_id)));
            let last = events.iter().rev().find(|e| e["event"] == "compaction" || (e["event"] == "model_call" && e["ok"] == true)).cloned();
            *session.tracer.events.lock().unwrap() = events;
            if let Some(l) = last.filter(|l| l["event"] == "model_call" && !l["tokens"].is_null()) {
                session.context_manager.lock().unwrap().set_latest_usage(TokenUsage::from_value(&l["tokens"]));
            }
        }
        if let (Some(todos), Some(map)) = (session.tool_registry.get("todos"), snap.todos.as_object().filter(|m| !m.is_empty())) {
            if let Some(t) = todos.as_any().downcast_ref::<crate::tools::builtin::todo::TodosTool>() {
                t.replace(map.iter().map(|(id, v)| crate::tools::builtin::todo::Todo { id: id.clone(), content: v["content"].as_str().unwrap_or("").into(), done: v["done"].as_bool().unwrap_or(false), active: false }).collect());
            }
        }
        // the new session starts with no one to ask; without this a resumed run refuses every edit
        *session.approval_manager.confirmation_callback.lock().unwrap() = confirm;
        self.agent().session.mcp_manager.shutdown().await;
        self.agent().session = session;
        self.publish_session();
        self.tui.reset();
    }

    pub(crate) async fn resume(&mut self, id: &str) -> bool {
        let Some(snap) = PersistenceManager::new().load_session(id) else { return false };
        self.restore_into_session(&snap, true).await;
        // draw the conversation again so the transcript shows what was said
        let requests: Vec<String> = self.agent().session.tracer.events.lock().unwrap().iter()
            .filter(|e| e["event"] == "task_start")
            .map(|e| e["request"].as_str().unwrap_or("").to_string())
            .collect();
        for m in &snap.messages {
            match m["role"].as_str().unwrap_or("") {
                "user" => {
                    let c = m["content"].as_str().unwrap_or("");
                    if c.starts_with(crate::context::manager::COMPACT_HEAD) {
                        uprintln!("{}", italic(&paint("  ✦ compacted · earlier messages are summarised for the model", MUTED)));
                    } else if !typed_by_person(c, &requests) {
                        // a nudge zerd sent the model, not something the person typed
                    } else {
                        self.tui.echo_user(c.split("\n\n---\n").next().unwrap_or(""));
                    }
                }
                "assistant" => {
                    let c = m["content"].as_str().unwrap_or("");
                    if !c.trim().is_empty() {
                        uprintln!("\n{}", super::md::render_block(c, true));
                    }
                }
                _ => {}
            }
        }
        if let Some(t) = self.agent().session.tool_registry.get("todos").and_then(|t| t.as_any().downcast_ref::<crate::tools::builtin::todo::TodosTool>().map(|t| t.snapshot())) {
            self.tui.set_todos(t);
        }
        true
    }

    async fn process_message(&mut self, message: &str) -> Option<String> {
        self.agent.as_ref()?;
        let started = Instant::now();
        let tui = self.tui.clone();
        let progress_tui = tui.clone();
        let reg = self.agent().session.tool_registry.clone();
        *reg.progress.lock().unwrap() = Some(Arc::new(move |id, info| progress_tui.agent_progress(id, &info)));
        tui.set_cwd(&self.config.get().cwd);
        tui.start_turn("Thinking");
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<AgentEvent>();
        let (mut final_response, mut report) = (None::<String>, None::<Value>);
        {
            let agent = self.agent.as_mut().unwrap();
            let run = agent.run(message, tx);
            tokio::pin!(run);
            let handle = |ev: AgentEvent, final_response: &mut Option<String>, report: &mut Option<Value>| match ev {
                AgentEvent::TextDelta(c) => tui.assistant_delta(&c),
                AgentEvent::TextComplete(c) => {
                    *final_response = Some(c);
                    tui.assistant_flush();
                }
                AgentEvent::Notice(m) => tui.notice(&m),
                AgentEvent::AgentError { error } => tui.error(&error),
                AgentEvent::ToolCallStart { call_id, name, arguments, .. } => tui.tool_call_start(&call_id, &name, &arguments),
                AgentEvent::ToolCallComplete { call_id, name, result } => {
                    let diff = result.diff.as_ref().map(|d| d.to_diff());
                    tui.tool_call_complete(&call_id, &name, result.success, &result.output, result.error.as_deref(), &result.metadata, diff.as_deref(), result.exit_code);
                    if name == "todos" {
                        if let Some(t) = reg.get("todos").and_then(|t| t.as_any().downcast_ref::<crate::tools::builtin::todo::TodosTool>().map(|t| t.snapshot())) {
                            tui.set_todos(t);
                        }
                    }
                }
                AgentEvent::AgentEnd { report: r, .. } => *report = Some(r),
                AgentEvent::AgentStart { .. } => {}
            };
            loop {
                tokio::select! {
                    _ = &mut run => break,
                    Some(ev) = rx.recv() => handle(ev, &mut final_response, &mut report),
                }
            }
            while let Ok(ev) = rx.try_recv() {
                handle(ev, &mut final_response, &mut report);
            }
        }
        tui.end_turn();
        self.last_report = report.clone();
        if let Some(r) = &report {
            self.print_report(r, started.elapsed().as_secs_f64());
        }
        let mgr = self.agent().session.approval_manager.clone();
        if mgr.policy() == ApprovalPolicy::Plan && final_response.as_deref().map_or(false, looks_like_plan) {
            if let Some(asker) = super::prompt::asker() {
                if asker.ask_choice("Carry out this plan? (leaves plan mode)".into(), vec!["Yes, go ahead".into(), "No, keep planning".into()]).await == 0 {
                    let back = *self.before_plan.lock().unwrap();
                    mgr.set_policy(back);
                    self.config.update(|c| c.approval = back);
                    uprintln!("{}", paint(&format!("Plan mode off, approval: {}", back.value()), MINT));
                    return Box::pin(self.process_message("The plan is approved. Carry it out now, step by step, and verify as you go.")).await;
                }
            }
        }
        final_response
    }

    /// /connect groq: ask for the key (hidden), keep it in zerd's .env, prove it works by listing the provider's
    /// models, and make those models Tab-completable in /model.
    async fn connect(&mut self, name: &str) {
        let cfg = self.config.get();
        let providers = crate::client::providers::all_providers(&cfg);
        let Some(p) = providers.get(name) else {
            if !name.is_empty() {
                uprintln!("{}", paint(&format!("Unknown provider: {name}"), CORAL));
            }
            uprintln!("Connect one with {}:", bold("/connect <name>"));
            for (n, q) in &providers {
                let state = if q.api_key().is_some() { paint("ready", MINT) } else { paint(&format!("needs {}", q.api_key_env.clone().unwrap_or_default()), MUTED) };
                uprintln!("  {n:<11} {state}  {}", paint(&q.base_url, MUTED));
            }
            uprintln!("{}", paint("Anything else that speaks the OpenAI or Anthropic format: add a [providers.<name>] table to config.toml.", MUTED));
            return;
        };
        if let Some(env) = &p.api_key_env {
            let note = if p.api_key().is_some() { " (Enter keeps the current one)" } else { "" };
            let prompt = format!("Paste your {} API key{note}: ", p.name);
            let key = match fullscreen::current() {
                Some(fs) => fs.ask_line(&prompt, true).await,
                None => tokio::task::spawn_blocking(move || read_secret_line(&prompt)).await.unwrap_or_default(),
            };
            if key.chars().any(char::is_whitespace) {
                uprintln!("{}", paint("That doesn't look like a key (it has spaces or line breaks). Nothing saved.", CORAL));
                return;
            }
            let (key, copies) = collapse_repeats(&key);
            if copies > 1 {
                uprintln!("{}", paint(&format!("That looked like the key pasted {copies} times; using one copy."), AMBER));
            }
            if !key.is_empty() {
                if let Ok(path) = save_env_key(env, &key) {
                    uprintln!("{}", paint(&format!("Saved {env} to {}", path.display()), MUTED));
                }
            } else if p.api_key().is_none() {
                uprintln!("{}", paint("Cancelled.", MUTED));
                return;
            }
        }
        match crate::client::providers::fetch_models(p).await {
            Err(e) if e.contains("401") || e.contains("403") => uprintln!("{}", paint(&format!("{} rejected that key ({e}). It is saved, but it won't work: run /connect {} again with the right one.", p.name, p.name), CORAL)),
            Err(e) => uprintln!("{}", paint(&format!("Couldn't list {}'s models ({}). If the key is right you can still pick one by name: /model {}:<model>", p.name, e.chars().take(160).collect::<String>(), p.name), AMBER)),
            Ok(ids) => {
                let _ = std::fs::create_dir_all(get_data_dir());
                let _ = std::fs::write(get_data_dir().join(format!("models-{}.json", p.name)), serde_json::to_string(&ids).unwrap_or_default());
                self.agent().session.client.registry.add_provider_models_list(&p.name, &ids);
                uprintln!("{} Switch with {} and Tab, e.g. /model {}:{}", paint(&format!("Connected {}: {} models.", p.name, ids.len()), MINT), bold(&format!("/model {}:", p.name)), p.name, ids.first().cloned().unwrap_or_else(|| "<model>".into()));
            }
        }
    }

    fn show_help(&self) {
        uprintln!("\n{}", bold_paint("Commands", LAVENDER));
        for (c, d) in COMMANDS {
            uprintln!("  {}  {}", bold_paint(&format!("{c:<13}"), ACCENT), paint(d, MUTED));
        }
        uprintln!("\n{}\n  @path attaches a file (Tab completes)\n  !cmd runs a shell command and shows the output to the model; !!cmd runs it privately\n  Ctrl+J inserts a newline · Ctrl+C interrupts the agent\n", bold_paint("Input", LAVENDER));
    }

    /// Returns false when the session should end.
    async fn handle_command(&mut self, command: &str) -> bool {
        let command = command.trim();
        let lower = command.to_lowercase();
        let name = lower.split_whitespace().next().unwrap_or("").to_string();
        // args keep their original case: model ids like Qwen/Qwen3.8-Max are case-sensitive
        let args = command.split_once(char::is_whitespace).map_or("", |x| x.1.trim()).to_string();
        let cfg = self.config.get();
        match name.as_str() {
            "/exit" | "/quit" => return false,
            "/help" => self.show_help(),
            "/compact" => {
                let before = self.agent().session.context_manager.lock().unwrap().context_used();
                self.tui.start_turn("Compacting");
                let ok = self.agent().compact_now().await;
                self.tui.end_turn();
                if !ok {
                    uprintln!("{}", paint("Nothing to compact yet", MUTED));
                } else {
                    self.autosave(); // otherwise quitting now and resuming brings the whole uncompacted history back
                    let sid = self.agent().session.session_id();
                    let after = self.agent().session.context_manager.lock().unwrap().latest_usage.total_tokens;
                    uprintln!("{}", paint(&format!("Context compacted: {before} -> ~{after} tokens"), MINT));
                    uprintln!("{}", paint(&format!("Summary: first message of {}\nReplaced messages: {}", get_data_dir().join("sessions").join(format!("{sid}.json")).display(), get_data_dir().join("transcripts").join(format!("{sid}.jsonl")).display()), MUTED));
                }
            }
            "/cost" => {
                let u = summarize_usage(&self.agent().session.tracer.snapshot(), &cfg.pricing);
                let w = &u["cache_waste"];
                uprintln!("  calls: {}   in: {} ({}% cached)   out: {}", u["calls"], u["prompt_tokens"], u["cache_hit_pct"], u["completion_tokens"]);
                uprintln!("  est cost: {}", u["est_cost_usd"].as_f64().map(|c| format!("${c}")).unwrap_or_else(|| "unknown (add a [pricing] section to config)".into()));
                self.print_heaviest(&u);
                if w["misses"].as_i64().unwrap_or(0) > 0 {
                    uprintln!("  cache misses: {} ({} tokens re-billed: {})", w["misses"], w["missed_tokens"], w["reasons"].as_array().map(|r| r.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")).unwrap_or_default());
                }
            }
            "/layout" => uprintln!("{}", crate::ui::layout::command(&args)),
            "/update" => {
                self.tui.start_turn("Checking for an update");
                let msg = crate::util::update::from_session().await;
                self.tui.end_turn();
                uprintln!("{}", paint(&format!("  {msg}"), MUTED));
            }
            "/loop" => {
                if args.is_empty() {
                    uprintln!("{}", paint("Usage: /loop <task>. zerd writes a brief file with the goal and what done means, then keeps working without asking until it is proven.", MUTED));
                } else {
                    // commands skip the normal attach step, so a dragged-in screenshot is turned into an [image:] line here
                    let (task, shots) = attach_images(&args, &cfg.cwd, &get_data_dir().join("images"));
                    if !shots.is_empty() {
                        uprintln!("{}", paint(&format!("  attached: {}", shots.join(", ")), MUTED));
                    }
                    self.run_loop(&task).await;
                }
            }
            "/verbose" => uprintln!("{}", paint(&format!("  output: {}", if self.tui.toggle_quiet() { "quiet" } else { "verbose" }), MUTED)),
            "/challenge" => {
                if args.is_empty() {
                    uprintln!("{}", paint("Usage: /challenge <your idea or plan>. A second model argues against it.", MUTED));
                } else {
                    self.tui.start_turn("Challenging");
                    let (objections, dropped, err) = crate::agent::challenge::challenge_request(&cfg, &args).await;
                    self.tui.end_turn();
                    if !err.is_empty() {
                        uprintln!("{}", paint(&format!("Challenger failed: {}", err.chars().take(200).collect::<String>()), CORAL));
                    } else {
                        let r = crate::agent::challenge::render(&objections, dropped);
                        uprintln!("{}", if r.is_empty() { paint("Nothing it could back with evidence.", MUTED) } else { r });
                    }
                }
            }
            "/skills" => {
                let skills = self.agent().session.skills.clone();
                for sk in &skills {
                    uprintln!("  {}{}  {}", paint(&sk.name, ACCENT), if sk.always { paint(" (always on)", MUTED) } else { String::new() }, paint(&sk.description, MUTED));
                }
                if skills.is_empty() {
                    uprintln!("{}", paint("  no skills loaded", MUTED));
                }
            }
            "/copy" => {
                let fs = fullscreen::current();
                let text = match &fs {
                    Some(f) if args.trim() == "all" => f.all_text(), // the whole conversation, not just the last reply
                    _ => self.tui.last_reply(),
                };
                if text.is_empty() {
                    uprintln!("{}", paint("  nothing to copy yet", MUTED));
                } else if fs.is_some() || copy_to_clipboard(&text) {
                    if fs.is_some() {
                        super::screen::copy_to_clipboard(&text);
                    }
                    uprintln!("{}", paint(&format!("  copied {} characters", text.chars().count()), MINT));
                } else {
                    uprintln!("{}", paint("  no clipboard tool found (pbcopy, wl-copy or xclip)", AMBER));
                }
            }
            "/jobs" => {
                let words: Vec<&str> = args.split_whitespace().collect();
                let kill = words.first() == Some(&"kill");
                let jobs = JOBS_LIST();
                let job = words.last().and_then(|w| w.parse::<usize>().ok()).and_then(|id| jobs.iter().find(|j| j.id == id).cloned());
                if let (true, Some(j)) = (kill, &job) {
                    crate::tools::builtin::shell::kill_job(j);
                    uprintln!("{}", paint(&format!("  stopped job {}", j.id), MUTED));
                } else if let Some(j) = &job {
                    uprintln!("{} {}\n{}", paint(&format!("job {}", j.id), ACCENT), paint(&j.command, MUTED), j.tail(40));
                } else {
                    if jobs.is_empty() {
                        uprintln!("{}", paint("  no background jobs", MUTED));
                    }
                    for j in jobs {
                        let state = match j.returncode() {
                            None => "running".to_string(),
                            Some(0) => "done".to_string(),
                            Some(c) => format!("failed ({c})"),
                        };
                        uprintln!("  {} {state:<12} {}", paint(&j.id.to_string(), ACCENT), paint(&format!("{:.0}s  {}", j.started.elapsed().as_secs_f64(), j.command), MUTED));
                    }
                }
            }
            "/expand" => self.tui.show_result(args.parse().unwrap_or(1)),
            "/clear" => {
                {
                    let a = self.agent();
                    a.session.context_manager.lock().unwrap().clear();
                    a.session.loop_detector.lock().unwrap().clear();
                }
                self.tui.reset();
                match fullscreen::current() {
                    Some(f) => f.clear(),
                    None => uprint!("\x1b[2J\x1b[3J\x1b[H"),
                }
                uprintln!("{}", paint("Conversation cleared ", MINT));
            }
            "/config" => {
                uprintln!("\n{}", bold("Current Configuration"));
                uprintln!("  Model: {}", cfg.model.name);
                uprintln!("  Temperature: {}", cfg.model.temperature.map(|t| t.to_string()).unwrap_or_else(|| "per model family".into()));
                uprintln!("  Approval: {}", cfg.approval.value());
                uprintln!("  Working Dir: {}", cfg.cwd.display());
                uprintln!("  Max Turns: {}", cfg.max_turns);
                uprintln!("  Hooks Enabled: {}", cfg.hooks_enabled);
            }
            "/model" => {
                if args.is_empty() {
                    uprintln!("Current model: {}", cfg.model.name);
                } else {
                    let (known, near) = self.agent().session.client.registry.validate(&args);
                    if !known {
                        uprintln!("{}", paint(&format!("Unknown model: {args}"), CORAL));
                        if !near.is_empty() {
                            uprintln!("Did you mean: {}", near.join(", "));
                        }
                    } else {
                        let a = args.clone();
                        self.config.update(|c| {
                            c.model.name = a;
                            c.model_pinned = true;
                        });
                        save_last_model(&args);
                        uprintln!("{}", paint(&format!("Model changed to: {args} "), MINT));
                    }
                }
            }
            "/connect" => self.connect(&args.to_lowercase()).await,
            "/approval" => {
                if args.is_empty() {
                    uprintln!("Current approval policy: {}", cfg.approval.value());
                } else if let Some(p) = ApprovalPolicy::parse(&args) {
                    self.config.update(|c| c.approval = p);
                    self.agent().session.approval_manager.set_policy(p); // the manager holds its own copy
                    uprintln!("{}", paint(&format!("Approval policy changed to: {args} "), MINT));
                } else {
                    uprintln!("{}", paint(&format!("Incorrect approval policy: {args} "), CORAL));
                    uprintln!("Valid options: on-request, on-failure, auto, auto-edit, never, yolo, plan");
                }
            }
            "/sandbox" => {
                use crate::safety::sandbox;
                let show = |mode: &str| match mode {
                    "on" => uprintln!("{}", paint("  sandbox: on (commands run inside the OS sandbox)", MINT)),
                    "off" => uprintln!("{}", paint("  sandbox: off (commands run unconfined, approvals still apply)  /sandbox on to re-enable", CORAL)),
                    _ => uprintln!("{}", paint(&format!("  sandbox: unavailable, {}", sandbox::unavailable_reason()), CORAL)),
                };
                match args.as_str() {
                    "" | "status" => show(sandbox::mode(&cfg)),
                    "on" | "off" | "remove" => {
                        let value = if args == "on" { "auto" } else { "off" };
                        self.config.update(|c| c.sandbox = value.into());
                        let saved = sandbox::save_setting(value).is_ok();
                        show(sandbox::mode(&self.config.get()));
                        if !saved {
                            uprintln!("{}", paint("  (could not save to config.toml, this applies to this session only)", MUTED));
                        }
                    }
                    _ => uprintln!("Usage: /sandbox [status|on|off|remove]"),
                }
            }
            "/prompt" => {
                use crate::project_prompt as pp;
                pp::ensure(&cfg.cwd);
                match args.as_str() {
                    "confirm on" | "confirm off" => {
                        let on = args.ends_with("on");
                        let saved = pp::set_confirm(&cfg.cwd, on);
                        uprintln!("{}", paint(&if saved { format!("  task confirmation: {}", if on { "on" } else { "off" }) } else { "  could not edit prompt.yaml".to_string() }, MINT));
                    }
                    _ => {
                        let on = pp::load(&cfg.cwd).map_or(true, |p| p.confirm_tasks);
                        uprintln!("  {}  task confirmation: {}", pp::path(&cfg.cwd).display(), if on { "on" } else { "off" });
                        uprintln!("{}", paint(&format!("  trace of answers: {}", pp::log_path(&cfg.cwd).display()), MUTED));
                    }
                }
            }
            "/effort" => {
                if matches!(args.as_str(), "low" | "medium" | "high" | "auto") {
                    let v = (args != "auto").then(|| args.clone());
                    self.config.update(|c| c.reasoning_effort = v);
                    uprintln!("{} {}", paint(&format!("  effort: {args}"), MINT), paint("(Codex models; others ignore it)", MUTED));
                } else {
                    uprintln!("  effort: {}  {}", cfg.reasoning_effort.clone().unwrap_or_else(|| "auto".into()), paint("/effort low|medium|high|auto", MUTED));
                }
            }
            "/agents" => {
                let owner = self.config.key();
                if let Some(id) = args.strip_prefix("stop").map(str::trim) {
                    let ok = crate::tools::subagents::stop(owner, id);
                    uprintln!("{}", paint(&if ok { format!("  stopped sub-agent {id}") } else { format!("  no running background sub-agent '{id}'") }, MUTED));
                    return true;
                }
                for (id, name, goal, secs) in crate::tools::subagents::running(owner) {
                    uprintln!("  {} {}  {}\n      {}", paint("running", MINT), bold(&name), paint(&format!("{id}, {secs}s  (/agents stop {id})"), MUTED), goal.chars().take(150).collect::<String>());
                }
                let files = crate::tools::subagents::load_agent_files(&cfg.cwd);
                for d in crate::tools::subagents::default_definitions(&cfg) {
                    let mode = if d.allowed_tools.as_ref().map_or(false, |t| !t.is_empty() && t.iter().all(|x| crate::tools::subagents::READONLY_TOOLS.contains(&x.as_str()))) { "read-only" } else { "can edit/run" };
                    let from = if files.iter().any(|f| f.name == d.name) { " (from an agent file)" } else { "" };
                    uprintln!("  {}  {}\n      {}", bold(&d.name), paint(&format!("{mode}, {} turns, {} model{from}", d.max_turns, d.role), MUTED), d.description.chars().take(150).collect::<String>());
                }
                uprintln!("{}", paint("Add your own: a markdown file with name/description/tools in .claude/agents/ or .zerd/agents/ (repo or home)", MUTED));
            }
            "/permissions" => {
                use crate::safety::rules::{drop_rule, load_rules, save_rule, KINDS};
                let (word, rule_text) = args.split_once(' ').map_or((args.as_str(), ""), |x| (x.0, x.1.trim()));
                let mgr = self.agent().session.approval_manager.clone();
                if KINDS.contains(&word) && !rule_text.is_empty() {
                    if let Ok(p) = save_rule(&cfg.cwd, word, rule_text) {
                        uprintln!("{}", paint(&format!("{word}: {rule_text}  (saved in {})", p.display()), MINT));
                    }
                    self.reload_rules();
                } else if word == "remove" && !rule_text.is_empty() {
                    let ok = drop_rule(&cfg.cwd, rule_text);
                    self.reload_rules();
                    uprintln!("{}", if ok { paint("Removed", MINT) } else { paint("No such rule in .zerd/permissions.json", MUTED) });
                } else {
                    let _ = load_rules;
                    let rules = mgr.rules_snapshot();
                    if rules.rules.is_empty() {
                        uprintln!("{}", paint("No rules. Add one: /permissions allow Bash(git diff:*) | deny Read(.env*) | ask Bash(git push:*)", MUTED));
                    }
                    for kind in KINDS {
                        for r in rules.rules.iter().filter(|r| r.kind == kind) {
                            uprintln!("  {kind:<6} {}  {}", r.text, paint(&home_short(Path::new(&r.source)), MUTED));
                        }
                    }
                    uprintln!("{}", paint(&format!("approval mode: {}. Deny wins over ask, ask over allow; /permissions remove <rule>", mgr.policy().value()), MUTED));
                }
            }
            "/plan" => {
                let mgr = self.agent().session.approval_manager.clone();
                if mgr.policy() == ApprovalPolicy::Plan {
                    let back = *self.before_plan.lock().unwrap();
                    mgr.set_policy(back);
                    self.config.update(|c| c.approval = back);
                    uprintln!("{}", paint(&format!("Plan mode off, approval: {}", back.value()), MINT));
                } else {
                    *self.before_plan.lock().unwrap() = mgr.policy();
                    mgr.set_policy(ApprovalPolicy::Plan);
                    self.config.update(|c| c.approval = ApprovalPolicy::Plan);
                    uprintln!("{}", paint("Plan mode on: zerd can read and search but changes nothing. /plan again to leave.", MINT));
                }
            }
            "/diff" => {
                let mut a = vec!["-C".to_string(), cfg.cwd.to_string_lossy().to_string(), "diff".into()];
                if args != "full" {
                    a.push("--stat".into());
                }
                let out = std::process::Command::new("git").args(&a).output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
                uprintln!("{}", if out.is_empty() { paint("No unstaged changes", MUTED) } else { out });
            }
            "/context" => {
                let a = self.agent();
                let window = a.session.client.registry.get(&cfg.model.name).context_length;
                let cm = a.session.context_manager.lock().unwrap();
                let (used, limit) = (cm.context_used(), cm.limit() as usize);
                let cleared = cm.messages.iter().filter(|m| m.pruned).count();
                let mut by_role: std::collections::HashMap<&str, usize> = Default::default();
                for m in &cm.messages {
                    *by_role.entry(m.role.as_str()).or_default() += m.token_count;
                }
                uprintln!("\n{}  {used} / {limit} tokens before compaction ({}%), window {window}", bold("Context"), used * 100 / limit.max(1));
                uprintln!("   system prompt  ~{} tokens", crate::util::text::estimate_tokens(cm.system_prompt()));
                uprintln!("   summary        {}", cm.summary.as_ref().map(|s| format!("~{} tokens", crate::util::text::estimate_tokens(s))).unwrap_or_else(|| "none yet".into()));
                uprintln!("   messages       {}: user ~{}, assistant ~{}, tool results ~{} tokens ({cleared} old results cleared)", cm.message_count(), by_role.get("user").unwrap_or(&0), by_role.get("assistant").unwrap_or(&0), by_role.get("tool").unwrap_or(&0));
            }
            "/hooks" => {
                if !cfg.hooks_enabled || cfg.hooks.is_empty() {
                    uprintln!("{}", paint("No hooks configured (see hooks in config.toml)", MUTED));
                }
                for h in cfg.hooks.iter().filter(|_| cfg.hooks_enabled) {
                    uprintln!("  • {}: {}{} -> {}", h.name, h.trigger.value(), h.matcher.as_ref().map(|m| format!(" [{m}]")).unwrap_or_default(), h.command.clone().unwrap_or_else(|| "script".into()));
                }
            }
            "/stats" => {
                uprintln!("\n{}", bold("Session Statistics "));
                if let Some(o) = self.agent().session.get_stats().as_object() {
                    for (k, v) in o {
                        uprintln!("   {k}: {v}");
                    }
                }
            }
            "/tools" => {
                let tools = self.agent().session.tool_registry.get_tools();
                uprintln!("\n{}", bold(&format!("Available tools ({}) ", tools.len())));
                for t in tools {
                    uprintln!("  • {}", t.name());
                }
            }
            "/mcp" => {
                let servers = self.agent().session.mcp_manager.get_all_servers();
                uprintln!("\n{}", bold(&format!("MCP Servers ({}) ", servers.len())));
                for s in servers {
                    let st = s["status"].as_str().unwrap_or("");
                    uprintln!("  • {}: {} ({} tools)", s["name"].as_str().unwrap_or(""), paint(st, if st == "connected" { MINT } else { CORAL }), s["tools"]);
                }
            }
            "/save" => {
                let snap = self.snapshot();
                PersistenceManager::new().save_session(&snap);
                uprintln!("{}", paint(&format!("Session saved: {}", snap.session_id), MINT));
            }
            "/sessions" => {
                uprintln!("\n{}", bold("Saved Sessions"));
                for s in PersistenceManager::new().list_sessions() {
                    let here = if s["cwd"] == cfg.cwd.to_string_lossy().as_ref() { paint(" (this folder)", MUTED) } else { String::new() };
                    uprintln!("  • {}  {} turns  {}{here}\n      {}", s["session_id"].as_str().unwrap_or(""), s["turn_count"], s["updated_at"].as_str().unwrap_or("").chars().take(16).collect::<String>(), paint(s["title"].as_str().unwrap_or(""), MUTED));
                }
            }
            "/resume" => {
                let pm = PersistenceManager::new();
                let sid = if args.is_empty() { pm.latest_for(&cfg.cwd.to_string_lossy()) } else { Some(args.clone()) };
                match sid {
                    None => uprintln!("{}", paint("No saved session for this folder. /sessions lists all.", MUTED)),
                    Some(id) => {
                        if self.resume(&id).await {
                            uprintln!("{}", paint(&format!("Resumed {id} ({} turns)", self.agent().session.turns()), MINT));
                        } else {
                            uprintln!("{}", paint(&format!("No session with id {id}"), CORAL));
                        }
                    }
                }
            }
            "/learn" | "/lessons" | "/forget" => {
                let mut learned = crate::agent::learn::Learned::new(&cfg.cwd);
                match name.as_str() {
                    "/learn" => {
                        let ok = !args.is_empty() && learned.add_lesson(&args, "user");
                        uprintln!("{}", if ok { paint("Learned, and saved to AGENTS.md. It applies from the next session (the prompt is fixed for this one).", MINT) } else { paint("Usage: /learn <a rule, at least a few words>", MUTED) });
                    }
                    "/forget" => {
                        let gone = args.parse::<usize>().ok().and_then(|n| learned.forget(n));
                        uprintln!("{}", match gone { Some(g) => paint(&format!("Forgot: {g}"), MINT), None => paint("Usage: /forget <number from /lessons>", MUTED) });
                    }
                    _ => {
                        let rows = learned.lessons();
                        for (i, t) in rows.iter().enumerate() {
                            uprintln!("  {}. {t}", i + 1);
                        }
                        if rows.is_empty() {
                            uprintln!("{}", paint("Nothing learned yet. Rejecting an action with feedback, or /learn <rule>, teaches zerd.", MUTED));
                        }
                    }
                }
            }
            "/verify" => {
                let files: Vec<String> = self.agent().undo_info.as_ref().and_then(|i| i["files"].as_array().cloned()).unwrap_or_default().iter().filter_map(|v| v.as_str().map(String::from)).collect();
                let checks = crate::agent::verify::select_checks(&cfg, &files);
                if files.is_empty() {
                    uprintln!("{}", paint("Nothing to verify: the last request changed no files.", MUTED));
                } else if checks.is_empty() {
                    uprintln!("{}", paint("No checks found for those files (add verify rules in .zerd/config.toml).", MUTED));
                } else {
                    uprintln!("{}", paint(&format!("Verifying: {}", checks.iter().map(|c| c.0.clone()).collect::<Vec<_>>().join(" && ")), MUTED));
                    let cfg_r = &cfg;
                    let results = futures::future::join_all(checks.iter().map(|(cmd, sub)| {
                        let dir = cfg_r.cwd.join(sub);
                        async move { crate::agent::verify::run_check(cmd, &dir, cfg_r).await }
                    }))
                    .await;
                    for r in &results {
                        if r.ok {
                            uprintln!("  {} {}", paint("✓", MINT), r.command);
                        } else {
                            uprintln!("  {} {}", paint("✗", CORAL), r.command);
                            uprintln!("{}", paint(&r.output_tail, MUTED));
                        }
                    }
                }
            }
            "/undo" => {
                let info = self.agent().undo_info.clone();
                match info {
                    None => uprintln!("{}", paint("Nothing to undo: the last request changed no files.", MUTED)),
                    Some(info) => {
                        let files: Vec<String> = info["files"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default();
                        let created: Vec<String> = info["created"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default();
                        let names = format!("{}{}", files.iter().take(6).cloned().collect::<Vec<_>>().join(", "), if files.len() > 6 { " ..." } else { "" });
                        if self.confirm_yes(&format!("Put back {} file(s) to how they were before the last request? ({names})", files.len())).await {
                            let (restored, deleted, skipped) = undo(&cfg.cwd, info["ref"].as_str().unwrap_or(""), &files, &created).await;
                            let a = self.agent();
                            a.undo_info = None;
                            a.checkpoints.pop();
                            a.session.tool_registry.epoch.fetch_add(1, std::sync::atomic::Ordering::SeqCst); // files on disk changed under earlier reads
                            uprintln!("{}", paint(&format!("Restored {}, removed {} new file(s).", restored.len(), deleted.len()), MINT));
                            if !skipped.is_empty() {
                                uprintln!("{}", paint(&format!("Left alone (no earlier copy): {}", skipped.join(", ")), AMBER));
                            }
                        }
                    }
                }
            }
            "/rewind" => {
                let marks = self.agent().checkpoints.clone();
                if marks.is_empty() {
                    uprintln!("{}", paint("Nothing to rewind: no request in this session has changed files.", MUTED));
                } else if let Ok(n) = args.parse::<usize>() {
                    let n = n.clamp(1, marks.len());
                    let chosen = &marks[marks.len() - n..];
                    let strs = |k: &str| -> Vec<String> { chosen.iter().flat_map(|m| m[k].as_array().cloned().unwrap_or_default()).filter_map(|v| v.as_str().map(String::from)).collect::<std::collections::BTreeSet<_>>().into_iter().collect() };
                    let (files, created) = (strs("files"), strs("created"));
                    let first_req = chosen[0]["request"].as_str().unwrap_or("").to_string();
                    if self.confirm_yes(&format!("Rewind {} file(s) to before: {first_req}?", files.len())).await {
                        let (restored, deleted, skipped) = undo(&cfg.cwd, chosen[0]["ref"].as_str().unwrap_or(""), &files, &created).await;
                        let a = self.agent();
                        a.checkpoints.truncate(marks.len() - n);
                        a.undo_info = None;
                        a.session.tool_registry.epoch.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        a.session.context_manager.lock().unwrap().add_user_message(&format!("[The user rewound the last {n} request(s): {} file(s) restored and {} new file(s) removed, so the working tree is back to how it was before \"{first_req}\". Earlier edits in this chat no longer exist on disk.]", restored.len(), deleted.len()));
                        uprintln!("{}", paint(&format!("Restored {}, removed {} new file(s).", restored.len(), deleted.len()), MINT));
                        if !skipped.is_empty() {
                            uprintln!("{}", paint(&format!("Left alone (no earlier copy): {}", skipped.join(", ")), AMBER));
                        }
                    }
                } else {
                    for (i, m) in marks.iter().rev().enumerate() {
                        uprintln!("  {}. {}  {}", i + 1, m["request"].as_str().unwrap_or(""), paint(&format!("({} file(s))", m["files"].as_array().map_or(0, Vec::len)), MUTED));
                    }
                    uprintln!("{}", paint("/rewind <n> puts the files back to how they were before request n (1 = the latest)", MUTED));
                }
            }
            "/checkpoint" => {
                let snap = self.snapshot();
                uprintln!("{}", paint(&format!("Checkpoint created: {}", PersistenceManager::new().save_checkpoint(&snap)), MINT));
            }
            "/restore" => {
                if args.is_empty() {
                    uprintln!("{}", paint("Usage: /restore <checkpoint_id> ", CORAL));
                } else {
                    match PersistenceManager::new().load_checkpoint(&args) {
                        None => uprintln!("{}", paint("Checkpoint does not exist ", CORAL)),
                        Some(snap) => {
                            self.restore_into_session(&snap, false).await;
                            uprintln!("{}", paint(&format!("Restored checkpoint {args} into session {}", snap.session_id), MINT));
                        }
                    }
                }
            }
            other => {
                let templates = crate::util::templates::load_commands(&crate::util::templates::prompt_dirs(Some(&cfg.cwd)));
                let key = &other[1..];
                if let Some((body, _)) = templates.get(key) {
                    let text = crate::util::templates::expand(body, &crate::util::templates::split_args(&args));
                    self.run_turn(&text).await;
                } else if let Some(sk) = self.agent().session.skills.iter().find(|k| k.name == key && !k.always).cloned() {
                    // /code-review the auth change: that playbook is loaded for this request, whatever the words say
                    self.agent().forced_skills = vec![sk.clone()];
                    let msg = if args.is_empty() { format!("Apply the {} skill to this project: {}", sk.name, sk.description) } else { args.clone() };
                    self.run_turn(&msg).await;
                } else {
                    uprintln!("{}", paint(&format!("Unknown command: {other}"), CORAL));
                }
            }
        }
        true
    }

    fn reload_rules(&mut self) {
        self.agent().session.approval_manager.reload_rules();
    }

    /// Before a task runs: make sure this project has a prompt.yaml, show the task back and ask if it is correct.
    /// Every answer goes to .zerd/confirmations.jsonl. Returns false when the user said no.
    async fn confirm_task(&self, task: &str) -> bool {
        use crate::project_prompt as pp;
        let cwd = self.config.get().cwd.clone();
        if pp::ensure(&cwd) {
            uprintln!("{}", paint("  created .zerd/prompt.yaml for this project: scope, rules and done-when commands. Edit it any time.", MUTED));
        }
        if !pp::load(&cwd).map_or(true, |p| p.confirm_tasks) {
            return true;
        }
        let Some(asker) = super::prompt::asker() else {
            pp::log_confirmation(&cwd, task, "skipped: no interactive prompt");
            return true;
        };
        let shown: String = task.chars().take(300).collect();
        let ok = asker.ask_choice(format!("Task: {shown}\nIs this correct, OK to go ahead?"), vec!["Yes".into(), "No".into()]).await == 0;
        pp::log_confirmation(&cwd, task, if ok { "yes" } else { "no" });
        if !ok {
            uprintln!("{}", paint("  not run. Say the task again with what to change.", MUTED));
        }
        ok
    }

    async fn confirm_yes(&self, question: &str) -> bool {
        match super::prompt::asker() {
            Some(a) => a.ask_choice(question.to_string(), vec!["Yes".into(), "No".into()]).await == 0,
            None => false,
        }
    }
}

#[allow(dead_code)]
fn _unused(_: PathBuf) {
    let _ = kill_jobs;
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_key_pasted_several_times_collapses_to_one_copy() {
        let k = "sk-abcdefghijklmnopqrstuvwxyz0123456789";
        assert_eq!(collapse_repeats(&k.repeat(3)), (k.to_string(), 3));
        assert_eq!(collapse_repeats(k), (k.to_string(), 1));
        assert_eq!(collapse_repeats("abab"), ("abab".to_string(), 1));
    }

    use super::*;

    #[test]
    fn dropped_screenshots_attach_and_are_not_commands() {
        let dir = tempfile::tempdir().unwrap();
        let shot = dir.path().join("Screenshot 2026-11-18 at 11.48.43\u{202f}AM.png");
        std::fs::write(&shot, b"png").unwrap();
        let typed = format!("{} fix this too", shot.display().to_string().replace(' ', "\\ "));
        assert!(starts_with_path(&typed, dir.path()));
        assert!(!starts_with_path("/help", dir.path()));
        let store = dir.path().join("store");
        let (text, names) = attach_images(&typed, dir.path(), &store);
        assert_eq!(names, vec![shot.file_name().unwrap().to_string_lossy().to_string()]);
        let line = text.lines().last().unwrap();
        assert!(line.starts_with("[image: ") && Path::new(&line[8..line.len() - 1]).is_file(), "{text}"); // a copy that outlives the temp file
        assert!(attach_images("see @Screenshot", dir.path(), &store).1.is_empty());
    }

    #[test]
    fn replay_skips_what_zerd_told_the_model() {
        let asked = vec!["fix the login bug".to_string(), "pls cont".to_string()];
        assert!(typed_by_person("fix the login bug\n\n---\n[Orientation: ...]", &asked)); // the request with zerd's notes after it
        assert!(typed_by_person("pls cont", &asked));
        assert!(typed_by_person("[the user added, while you were working:] /jobs", &asked));
        assert!(!typed_by_person("Budget check: 17 of 25 tool rounds used. Put every lookup...", &asked));
        assert!(!typed_by_person("[background job 8 `git clone ...` failed; last output:]\n...", &asked));
        assert!(typed_by_person("Budget check: 3 of 9", &[])); // no trace to go by: show it rather than lose your words
    }

    #[test]
    fn plans_have_steps() {
        let plan = format!("{}\n1. one\n2. two\n3. three", "word ".repeat(70));
        assert!(looks_like_plan(&plan));
        assert!(!looks_like_plan("1. one\n2. two\n3. three"));
        assert!(!looks_like_plan("just text"));
    }

    #[test]
    fn mentions_attach_files_but_not_secrets() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello").unwrap();
        std::fs::write(dir.path().join(".env"), "KEY=1").unwrap();
        let (text, attached) = expand_mentions("look at @a.txt and @.env and @missing.py", dir.path(), &[".env".to_string()]);
        assert_eq!(attached, vec!["a.txt"]);
        assert!(text.contains("<file path=\"a.txt\">\nhello\n</file>"));
        assert!(!text.contains("KEY=1"));
    }
}
