//! The agent loop: one user request in, a stream of events out. It talks to the model, runs the tools the model asks
//! for, keeps the context in bounds, and checks its own work before it answers.
use crate::agent::brevity::{interim_line, HOLD_CHARS, LONG_ANSWER_WORDS};
use crate::agent::challenge::{challenge_request, is_risky, render as render_objections};
use crate::agent::events::AgentEvent;
use crate::agent::grounding::{correction_prompt, has_file_code, shows_second_thoughts, verify_references, wants_change, REWRITE_PROMPT};
use crate::agent::learn::{judge, Learned};
use crate::agent::ledger;
use crate::agent::orient::{orient, repo_map, Orientation};
use crate::agent::review::{merge_reviews, review_changes, ReviewResult};
use crate::agent::review_fix;
use crate::agent::review_fix_verify::verify_fix;
use crate::agent::review_pipeline::{run_review_pipeline, Finding, PipelineEvent};
use crate::agent::root::pick_root;
use crate::agent::routing_policy::{classify, effort, keep_cached_model, length_hint, one_agent, is_reviewy, risky_change, typed};
use crate::agent::scope::{is_broad, SWEEP, SWEEP_CHECKPOINT};
use crate::agent::session::Session;
use crate::agent::skills::match_skills;
use crate::agent::task::{TaskState, TaskStatus};
use crate::agent::verify::{backup, changed_since, repair_prompt, run_check, select_checks, signature_breaks, signature_prompt, snapshot, split_preexisting, Snapshot};
use crate::client::response::{StreamEvent, TokenUsage, ToolCall};
use crate::config::{Config, ConfigHandle, HookTrigger};
use crate::prompts::create_loop_breaker_prompt;
use crate::safety::approval::ConfirmCallback;
use crate::tools::builtin::read_file::ReadFileTool;
use crate::tools::builtin::shell::{finished_jobs, kill_jobs};
use crate::tools::builtin::todo::TodosTool;
use crate::tools::{ToolKind, ToolResult};
use crate::util::cost::summarize_usage;
use crate::util::paths::resolve_path;
use crate::util::re::{lit, lit_i, Re};
use futures::StreamExt;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedSender;

pub type Tx = UnboundedSender<AgentEvent>;

const DOC_SUFFIX: &[&str] = &[".md", ".mdx", ".txt", ".rst"];
/// What the scout is sent ahead of the user's request. Without it the scout reads "fix the checkout bug" as its own
/// job and spends its turns trying to solve it, when all the main agent needs back is where to look.
const SCOUT_BRIEF: &str = "Locate the code for the request below; do not do the work and do not propose a design. Report: the files and functions that would have to change (file:line), who calls them (ask the graph), the tests that cover them, and anything that would surprise someone making the change. Stop as soon as you can name those.";
const REVIEW_NO_WORKERS: &str = "parallel_tasks runs coding workers that edit files in their own worktrees; a review only reads. Read the files yourself, or use subagent_investigator for a read-only question.";
const CUT_CALL: &str = "Not run: your reply hit the output token limit, so this call's arguments may be truncated. Re-issue it in smaller pieces: write a long file as a short first version and grow it with edits, or split one big edit into several.";

/// A tool call the model typed into its answer instead of making it (Qwen, GLM, DeepSeek and Hermes-style templates).
/// A shell command that proves something about the code: a typecheck, a lint, a test run, a build or a project script.
/// `npm run db:migrate` and `npx tsc --noEmit` count; `ls`, `cat` and `git status` do not.
fn check_command() -> &'static Re {
    static R: OnceLock<Re> = OnceLock::new();
    R.get_or_init(|| lit(r"\b(tsc|eslint|pytest|vitest|jest|mypy|ruff|py_compile|cargo (test|check|build|clippy)|go (test|vet|build)|(npm|pnpm|yarn|bun) (run [\w:-]+|test\b)|node --test|make( |$))"))
}

/// A command that runs the code rather than only checking it: `python3 retry_lab/client.py`, `node -e "..."`,
/// `npx tsx db/migrate.ts`, `curl localhost:3000/api/chat`. When it passes after the last edit it counts as proof,
/// but a failure is not held against the answer the way a failing test is: these are often experiments.
fn proof_command() -> &'static Re {
    static R: OnceLock<Re> = OnceLock::new();
    R.get_or_init(|| lit(r"\b(python3? (-c|\S+\.py)|node (-e|--check|\S+\.[cm]?js)|(tsx|ts-node|deno run|deno check|deno test) \S|bun \S+\.[cm]?[jt]sx?|(ba)?sh (-n )?\S+\.sh|php -l|ruby (-c|\S+\.rb)|curl )"))
}

/// Files an eslint command was handed but does not look at with a usual setup: `pnpm exec eslint app/page.tsx
/// app/globals.css` passes whatever is in globals.css. Returns a note naming them, or None.
fn unchecked_by_eslint(cmd: &str) -> Option<String> {
    if !cmd.split_whitespace().any(|w| w == "eslint" || w.ends_with("/eslint")) {
        return None;
    }
    let skipped: Vec<&str> = cmd.split_whitespace().map(|w| w.trim_matches(|c| c == '"' || c == '\'')).filter(|w| [".css", ".scss", ".less", ".html", ".md", ".mdx", ".json", ".yml", ".yaml"].iter().any(|s| w.ends_with(s))).collect();
    (!skipped.is_empty()).then(|| format!("\n[note: eslint does not check {} here, so a pass says nothing about {}. Their per-edit diagnostics are the check, or read the lines back.]", skipped.join(", "), if skipped.len() == 1 { "it" } else { "them" }))
}

fn text_call() -> &'static Re {
    static R: OnceLock<Re> = OnceLock::new();
    R.get_or_init(|| {
        lit(r#"<tool_call>|<function=\w|<invoke name=|<\|tool_call|\[TOOL_CALLS\]|<\|tool_calls_section_begin\|>|```(?:json)?\s*\{\s*"(?:name|tool|function)"\s*:\s*"(?:read_file|write_file|edit|shell|grep|glob|list_dir|todos)""#)
    })
}

/// "the prompt is too big", as each provider words it. A rate limit saying "too many tokens" is not one.
pub(crate) fn overflow() -> &'static Re {
    static R: OnceLock<Re> = OnceLock::new();
    R.get_or_init(|| {
        lit_i(r"context[_ ](length|window)|maximum context|prompt (is )?too long|prompt exceeds max length|request_too_large|too many tokens|exceeds? the (model|context)|input (is )?too long|maximum prompt length|reduce the length of the messages|exceeds the available context size|greater than the context length|exceeded model token limit|configured context size|range of input length should be|token limit exceeded|longer than the model'?s context length|maximum allowed input length|input token count.*exceeds|too large for model with")
    })
}
pub(crate) fn not_overflow() -> &'static Re {
    static R: OnceLock<Re> = OnceLock::new();
    R.get_or_init(|| lit_i(r"rate.?limit|too many requests|throttl|please wait"))
}

fn emit(tx: &Tx, ev: AgentEvent) {
    let _ = tx.send(ev);
}

/// What the user reads when background checks finish, e.g. "✓ background checks passed: cargo check -q" or the
/// failing command with the end of its output.
fn late_check_report(results: &[crate::agent::verify::CheckResult]) -> String {
    let bad: Vec<_> = results.iter().filter(|r| !r.ok).collect();
    if bad.is_empty() {
        return format!("  ✓ background checks passed: {}", results.iter().map(|r| r.command.as_str()).collect::<Vec<_>>().join(" && "));
    }
    let mut out = String::new();
    for r in bad {
        out.push_str(&format!("  ✗ background check failed: {}\n{}\n", r.command, crate::util::text::tail_chars(&r.output_tail, 1200)));
    }
    out + "  Your next message will carry this to the model, so \"fix it\" is enough."
}

fn notice(tx: &Tx, m: impl Into<String>) {
    emit(tx, AgentEvent::notice(m));
}

/// Pass a model stream through, but give up when the request's time budget is gone: a stalled connection must not
/// outlive the budget (retries and fallbacks each wait for their own read timeout).
/// How many times a change that is still in progress gets its token budget topped up before the hard stop.
const MAX_BUDGET_EXTENSIONS: i64 = 2;
/// How many actions may be skipped for want of an approver before the run gives up.
const MAX_UNASKED: usize = 3;
const OUT_OF_TIME: &str = "Time budget exhausted while waiting for the model";

pub(crate) async fn next_within(deadline: Instant, s: &mut futures::stream::BoxStream<'static, StreamEvent>) -> Option<StreamEvent> {
    let left = deadline.saturating_duration_since(Instant::now());
    if left.is_zero() {
        return Some(StreamEvent::Error(OUT_OF_TIME.into()));
    }
    match tokio::time::timeout(left, s.next()).await {
        Ok(ev) => ev,
        Err(_) => Some(StreamEvent::Error(OUT_OF_TIME.into())),
    }
}

pub struct Agent {
    pub config: ConfigHandle,
    pub session: Arc<Session>,
    pub task: Option<TaskState>,
    /// Typed while the agent works; lands after the current round of tool calls.
    pub steering: Arc<Mutex<Vec<String>>>,
    failed: bool,
    tier: String,
    escalated: bool,
    budget_hit: bool,
    compact_retry_turn: usize,
    pub(crate) last_role: String,
    last_compact_turn: Option<usize>,
    thrash: usize,
    overflow_retried: bool,
    pub(crate) last_model_call: Option<Instant>,
    /// Did the last request stop at the hard round limit?
    pub capped_last: bool,
    /// Tool rounds the model said this request needs (0 = it gave no number). The hard limit is a multiple of it.
    pub(crate) estimate: usize,
    over_noted: bool,
    calls_used: usize,
    calls_warned: bool,
    shown: Vec<Finding>,
    fix_run: bool,
    fix_brief: String,
    read_only: bool,
    /// Orient was tried and neither model could read the request.
    orient_down: bool,
    todos_reminded: String,
    pipelined: bool,
    repo_map: Option<String>,
    pipeline_answer: String,
    budget_warned: bool,
    /// how many times this request's token budget was topped up instead of cut off (see `agentic_loop`)
    budget_extensions: i64,
    uncached_streak: usize,
    usage_base: i64,
    /// size of the last prompt, and whether this provider has ever served part of one from its cache (see `forgive_rewarm`)
    last_prompt: i64,
    cache_seen: bool,
    /// Checks the model ran itself in the shell: the ones whose last run failed, and the ones that passed since the
    /// last edit. A finished answer with a red one is sent back once; green ones count as verification when the
    /// harness has no check of its own for this kind of project.
    red_checks: Vec<String>,
    green_checks: Vec<String>,
    /// Checks that were still running when the last request handed the prompt back. `late_stop` cancels them when a
    /// newer change makes their answer stale; `late_checks` holds their results until the model can be told.
    late_stop: Option<tokio::task::AbortHandle>,
    late_checks: Arc<Mutex<Vec<crate::agent::verify::CheckResult>>>,
    review_sec: f64,
    pub undo_info: Option<Value>,
    long_answer: usize,
    /// One entry per request that changed files, for /rewind.
    pub checkpoints: Vec<Value>,
    root_hint: Option<PathBuf>,
    scout_error: String,
    /// Actions this request wanted but could not get approved because nobody was there to ask (a script, a worker).
    unasked: Vec<String>,
    pub forced_skills: Vec<crate::agent::skills::Skill>,
    /// Names whose text is in the conversation; cleared on compaction.
    playbooks_sent: std::collections::HashSet<String>,
    last_request: String,
    last_answer: String,
    learning: Option<tokio::task::JoinHandle<String>>,
    /// The request while the review pipeline, scout or challenger runs, before it is in the history.
    unsent: String,
    touches: String,
    docs_nudged: bool,
    broad: bool,
    sweep_pushed: usize,
    cut_continues: usize,
    text_calls: usize,
    empty_replies: usize,
    /// Read-only calls already running because the model finished writing them mid-reply: call id -> (the call, its result).
    early: std::collections::HashMap<String, (ToolCall, tokio::task::JoinHandle<ToolResult>)>,
    plan_told: bool,
    todo_idle: usize,
    quiet_rounds: usize,
    edited: bool,
    fix_nudged: bool,
    wants_change: bool,
    is_review: bool,
    waited_base: f64,
}

/// What a one-off agent produced: its final texts, any error, how many tool calls a hook or rule refused, and the new
/// work it spent.
#[derive(Default, Debug)]
pub struct Collected {
    pub texts: Vec<String>,
    pub error: String,
    pub blocked: usize,
    pub spent: i64,
}

fn collect_event(c: &mut Collected, ev: AgentEvent) {
    match ev {
        AgentEvent::TextComplete(t) if !t.is_empty() => c.texts.push(t),
        AgentEvent::AgentError { error } => c.error = error,
        AgentEvent::ToolCallComplete { result, .. } if result.flag("blocked") => c.blocked += 1,
        AgentEvent::ToolCallStart { tokens, .. } => c.spent = tokens,
        _ => {}
    }
}

/// Run a one-off agent over `prompt` and collect what it said. For helpers like the challenger and the reviewers.
pub fn run_collect(cfg: Config, prompt: &str) -> futures::future::BoxFuture<'static, Collected> {
    // boxed: a reviewer is itself an agent, so this call is recursive
    let prompt = prompt.to_string();
    Box::pin(async move { run_collect_inner(cfg, &prompt).await })
}

async fn run_collect_inner(cfg: Config, prompt: &str) -> Collected {
    let mut agent = Agent::new(cfg, None).await;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut c = Collected::default();
    {
        let run = agent.run(prompt, tx);
        tokio::pin!(run);
        loop {
            tokio::select! {
                _ = &mut run => break,
                Some(ev) = rx.recv() => collect_event(&mut c, ev),
            }
        }
        while let Ok(ev) = rx.try_recv() {
            collect_event(&mut c, ev);
        }
    }
    c.spent = agent.spent_tokens();
    agent.close().await;
    c
}

pub async fn run_text(cfg: Config, prompt: &str) -> (Vec<String>, String) {
    let c = run_collect(cfg, prompt).await;
    (c.texts, c.error)
}

impl Agent {
    pub async fn new(config: Config, confirm: Option<ConfirmCallback>) -> Agent {
        let handle = ConfigHandle::new(config);
        let session = Session::create(handle.clone()).await;
        *session.approval_manager.confirmation_callback.lock().unwrap() = confirm;
        Agent::with_session(handle, session)
    }

    pub fn with_session(config: ConfigHandle, session: Arc<Session>) -> Agent {
        Agent {
            config,
            session,
            task: None,
            steering: Arc::new(Mutex::new(vec![])),
            failed: false,
            tier: "default".into(),
            escalated: false,
            budget_hit: false,
            compact_retry_turn: 0,
            last_role: "default".into(),
            last_compact_turn: None,
            thrash: 0,
            overflow_retried: false,
            last_model_call: None,
            capped_last: false,
            estimate: 0,
            over_noted: false,
            calls_used: 0,
            calls_warned: false,
            shown: vec![],
            fix_run: false,
            fix_brief: String::new(),
            read_only: false,
            orient_down: false,
            todos_reminded: String::new(),
            pipelined: false,
            repo_map: None,
            pipeline_answer: String::new(),
            budget_warned: false,
            budget_extensions: 0,
            uncached_streak: 0,
            usage_base: 0,
            last_prompt: 0,
            cache_seen: false,
            red_checks: vec![],
            green_checks: vec![],
            late_stop: None,
            late_checks: Default::default(),
            review_sec: 0.0,
            undo_info: None,
            long_answer: 0,
            checkpoints: vec![],
            root_hint: None,
            scout_error: String::new(),
            unasked: vec![],
            forced_skills: vec![],
            playbooks_sent: Default::default(),
            last_request: String::new(),
            last_answer: String::new(),
            learning: None,
            unsent: String::new(),
            touches: String::new(),
            docs_nudged: false,
            broad: false,
            sweep_pushed: 0,
            cut_continues: 0,
            text_calls: 0,
            empty_replies: 0,
            early: Default::default(),
            plan_told: false,
            todo_idle: 0,
            quiet_rounds: 0,
            edited: false,
            fix_nudged: false,
            wants_change: false,
            is_review: false,
            waited_base: 0.0,
        }
    }

    pub async fn close(&mut self) {
        kill_jobs();
        self.session.mcp_manager.shutdown().await;
    }

    /// Ctrl+C. A request cut off while the review pipeline was still running never reached the history, so the next
    /// "pls cont" found nothing to continue. Keep it, marked as interrupted, so the model can pick it back up.
    /// /compact: summarise now, whatever the size.
    pub async fn compact_now(&mut self) -> bool {
        self.compact(true).await
    }

    pub fn on_interrupt(&mut self) {
        let mut cm = self.session.context_manager.lock().unwrap();
        if !self.unsent.is_empty() {
            cm.add_user_message(&format!("{}\n\n[Interrupted by the user before you answered (the review, scout or challenger was still running). Nothing was changed.]", self.unsent));
            self.unsent.clear();
        }
        cm.repair_orphans(); // providers reject unanswered tool calls
    }

    /// Work from the repo itself. The folder zerd started in stays reachable for other repos next to it.
    fn adopt_root(&mut self, root: Option<PathBuf>) -> bool {
        let cfg = self.config.get();
        let Some(root) = root.filter(|r| *r != cfg.cwd) else { return false };
        let old = crate::util::paths::resolve(&cfg.cwd).to_string_lossy().to_string();
        self.config.update(|c| {
            c.allowed_dirs.push(old);
            c.cwd = root.clone();
        });
        self.root_hint = Some(root);
        true
    }

    fn add_user(&self, text: &str) {
        self.session.context_manager.lock().unwrap().add_user_message(text);
    }

    /// Harness context the model should act on but not reply to, marked so it never reads as the user talking.
    fn remind(&self, text: &str) {
        self.add_user(&format!("<system-reminder>\n{text}\n</system-reminder>"));
    }

    /// Record that the loop had to step in ("empty_reply", "text_call", ...). `zerd insights` counts these per model,
    /// which is how we learn which family needs a prompt note or a compat flag.
    fn nudged(&self, kind: &str) {
        self.session.tracer.emit("nudge", json!({"kind": kind}));
    }

    fn spent(&self) -> i64 {
        let cm = self.session.context_manager.lock().unwrap();
        let u = &cm.total_usage;
        u.prompt_tokens - u.cached_tokens + u.completion_tokens
    }

    /// The budget is there to stop a request that keeps piling on new work, not to bill the same context twice.
    /// When the provider's cache goes cold (we cleared old tool results, or the user came back after a coffee), the
    /// whole prompt is charged as uncached even though almost none of it is new. Only the part the prompt grew by
    /// counts; the rest is taken off this request's bill.
    /// Example: the last prompt was 150k, we prune, the next is 130k with nothing cached. Growth is zero, so none of
    /// those 130k count. Before this, that single call ate almost half of a 300k budget and the run got cut off.
    /// A provider that never caches is charged in full as before: there the resend is the real cost every time.
    fn forgive_rewarm(&mut self, u: &TokenUsage) {
        self.cache_seen = self.cache_seen || u.cached_tokens > 0;
        let uncached = u.prompt_tokens - u.cached_tokens;
        let growth = (u.prompt_tokens - self.last_prompt).max(0);
        if self.cache_seen && self.last_prompt > 0 && uncached > growth {
            self.usage_base += uncached - growth;
        }
        self.last_prompt = u.prompt_tokens;
    }

    pub fn spent_tokens(&self) -> i64 {
        self.spent()
    }

    fn todos(&self) -> Option<Arc<dyn crate::tools::Tool>> {
        self.session.tool_registry.get("todos")
    }

    fn unfinished_todos(&self) -> Vec<String> {
        self.todos().and_then(|t| t.as_any().downcast_ref::<TodosTool>().map(|t| t.unfinished())).unwrap_or_default()
    }

    fn drain_steering(&self) -> Vec<String> {
        std::mem::take(&mut *self.steering.lock().unwrap())
    }

    /// Routing role for the next model call: a fixed role for sub-agents and reviewers, otherwise the request's tier,
    /// escalated to `strong` after a failed repair or a loop (if a strong chain is configured).
    fn role(&self, task: &TaskState) -> String {
        let cfg = self.config.get();
        if cfg.model_role != "default" {
            return cfg.model_role.clone();
        }
        if (task.repairs >= 1 || self.escalated) && cfg.routing.contains_key("strong") {
            return "strong".into();
        }
        self.tier.clone()
    }

    /// The hard stop on tool rounds. With the model's own estimate it is a multiple of it (a runaway catcher, see
    /// `ledger::backstop`), still never above the configured budget; without one it is the configured budget.
    /// Sub-agents are not capped here (they have turn and token budgets).
    pub(crate) fn call_limit(&self) -> usize {
        let cfg = self.config.get();
        let budget = if self.read_only { cfg.read_call_budget } else { cfg.write_call_budget };
        if cfg.model_role != "default" {
            return 0;
        }
        match (self.estimate, budget) {
            (0, b) => b,
            (e, 0) => ledger::backstop(e),
            (e, b) => ledger::backstop(e).min(b),
        }
    }

    pub async fn run(&mut self, message: &str, tx: Tx) {
        let ask = typed(message);
        let cfg0 = self.config.get();
        let hook_context = self.session.hook_system.trigger_before_agent(message).await;
        let root = pick_root(&cfg0.cwd, message, self.root_hint.as_deref(), &[]);
        if self.adopt_root(root) {
            notice(&tx, format!("Working in {} (the repo this request is about)", self.config.get().cwd.display()));
        }
        emit(&tx, AgentEvent::AgentStart { message: message.into() });
        self.unsent = message.into();
        if self.learning.as_ref().map_or(false, |h| h.is_finished()) {
            if let Some(h) = self.learning.take() {
                if let Ok(rule) = h.await {
                    if !rule.is_empty() {
                        notice(&tx, format!("Learned: {rule} (saved to AGENTS.md; /forget removes it)"));
                    }
                }
            }
        }
        let cfg = self.config.get();
        if cfg.auto_learn && !self.last_request.is_empty() && self.learning.is_none() {
            // in the background, so the reply is not held up
            let (client, before, answer, msg, cwd) = (self.session.client.clone(), self.last_request.clone(), self.last_answer.clone(), message.to_string(), cfg.cwd.clone());
            self.learning = Some(tokio::spawn(async move {
                let rule = judge(&client, &before, &answer, &msg).await;
                if !rule.is_empty() && Learned::new(&cwd).add_lesson(&rule, "auto") { rule } else { String::new() }
            }));
        }
        self.last_request = message.into();
        // The small model reads the whole request first and says what job it is. Words are only the fallback for when it
        // is down, because "we review submissions" inside a pasted job post is not a request to review code.
        let oriented = self.orient(message).await;
        let reviewing = oriented.as_ref().map_or_else(|| is_reviewy(ask), |o| o.intent == "review" && crate::agent::routing_policy::about_code_review(ask));
        let forced = std::mem::take(&mut self.forced_skills); // set by a typed /skill-name, for this request only
        let mut matched = forced.clone();
        matched.extend(match_skills(&self.session.skills, ask, 4).into_iter().filter(|p| !forced.iter().any(|f| f.name == p.name) && (reviewing || p.name != "code-review")));
        // a playbook already in the conversation is named, not pasted again
        let playbooks: Vec<_> = matched.iter().filter(|p| !self.playbooks_sent.contains(&p.name)).cloned().collect();
        let again: Vec<String> = matched.iter().filter(|p| self.playbooks_sent.contains(&p.name)).map(|p| p.name.clone()).collect();
        self.playbooks_sent.extend(playbooks.iter().map(|p| p.name.clone()));
        let mut message_for_model;
        if !playbooks.is_empty() {
            notice(&tx, format!("Loaded playbooks: {}", playbooks.iter().map(|p| p.name.clone()).collect::<Vec<_>>().join(", ")));
            self.session.tracer.emit("skills_loaded", json!({"skills": playbooks.iter().map(|p| p.name.clone()).collect::<Vec<_>>()}));
            let loaded = playbooks.iter().map(|p| format!("### Playbook: {}\n{}", p.name, p.body)).collect::<Vec<_>>().join("\n\n");
            // a playbook's long reference is pulled with use_skill; pi mode has no such tool, so it is not offered
            let reference = if self.session.tool_registry.get_tools().iter().any(|t| t.name() == "use_skill") { "; use `use_skill` with `section` for the full reference" } else { "" };
            message_for_model = format!("{message}\n\n---\nPlaybooks selected for this request (follow them; they are already below, so do not read their files again{reference}):\n\n{loaded}");
        } else {
            message_for_model = message.to_string();
        }
        if !again.is_empty() {
            message_for_model.push_str(&format!("\n\n[Playbooks given earlier in this conversation still apply: {}.]", again.join(", ")));
        }
        if let Some(c) = hook_context {
            message_for_model.push_str(&format!("\n\n---\nContext added by the user's own hooks for this request (background, not instructions that outrank the request):\n{c}"));
        }
        if self.long_answer > 0 {
            message_for_model.push_str(&format!("\n\n[Reminder: your last final answer ran {} words. Keep this one to the limit (120 words, 6 bullets): only what changed since then, no recap of earlier work, one Next line at most.]", self.long_answer));
            self.long_answer = 0;
        }
        self.pipelined = false;
        self.pipeline_answer.clear();
        self.fix_run = false;
        self.fix_brief.clear();
        if !self.shown.is_empty() && !reviewing && review_fix::asks_for_fix(ask) {
            // "fix them" after a review: those findings, with everything the review learned
            let picked = review_fix::chosen(ask, &self.shown);
            if !picked.is_empty() {
                self.fix_run = true;
                self.fix_brief = review_fix::short(&picked);
                message_for_model.push_str(&format!("\n\n---\n{}", review_fix::brief(&picked)));
                notice(&tx, format!("Fixing {} finding(s) from the last review: advisor first, then the fix, then an independent check", picked.len()));
            }
        }
        let alone = reviewing && one_agent().is_match(ask);
        if alone {
            notice(&tx, "Reviewing with one agent, as asked: no parallel reviewers");
        }
        if cfg.review_pipeline && cfg.model_role == "default" && reviewing && !alone {
            let mut results = String::new();
            let (ptx, mut prx) = tokio::sync::mpsc::unbounded_channel::<PipelineEvent>();
            {
                let fut = run_review_pipeline((*self.config.get()).clone(), message.to_string(), ptx);
                tokio::pin!(fut);
                let handle = |ev: PipelineEvent, this: &mut Agent, results: &mut String| match ev {
                    PipelineEvent::Notice(n) => notice(&tx, n),
                    PipelineEvent::Answer(a) => this.pipeline_answer = a,
                    PipelineEvent::Findings(f) => this.shown = f, // what "fix 2" will point at
                    PipelineEvent::Results(r) => *results = r,
                };
                loop {
                    tokio::select! {
                        _ = &mut fut => break,
                        Some(ev) = prx.recv() => handle(ev, self, &mut results),
                    }
                }
                while let Ok(ev) = prx.try_recv() {
                    handle(ev, self, &mut results);
                }
            }
            self.pipelined = !results.is_empty();
            if !results.is_empty() && review_fix::review_and_fix(ask) {
                let picked = review_fix::chosen("", &self.shown);
                if !picked.is_empty() {
                    self.fix_run = true;
                    self.fix_brief = review_fix::short(&picked);
                    message_for_model.push_str(&format!("\n\n---\n{}", review_fix::brief(&picked)));
                    notice(&tx, format!("Fixing the {} proven finding(s): advisor first, then the fix, then an independent check", picked.len()));
                }
            }
            if !results.is_empty() {
                message_for_model.push_str(&format!(
                    "\n\n---\nReview results. Each claim below was checked by a separate agent that read the code and tried to refute it. Answer from them with no headings and none of the bracket tags: confirmed items as `severity file:line  what is wrong -> fix`; unverified items the same with (unverified) after the severity, at most MEDIUM; rejected items are not findings and are not mentioned. One finding per line. If the user also asked for a fix, fix the confirmed items first, then check once. If nothing is confirmed or unverified, say: No issues found.\n{results}"
                ));
            }
        }
        self.scout_error.clear();
        self.unasked.clear();
        let scout = self.scout(message).await;
        if !self.scout_error.is_empty() {
            notice(&tx, format!("Scout failed, continuing without it: {}", crate::util::text::head_chars(&self.scout_error, 160)));
        }
        if !scout.is_empty() {
            notice(&tx, "Scout finished; starting from its findings");
            message_for_model.push_str(&format!("\n\n---\nScout report (a cheaper read-only agent already searched; start from it, do not repeat its searches, and confirm any line you rely on before editing):\n{scout}"));
        }
        if cfg.auto_challenge && is_risky(ask) {
            notice(&tx, "Challenger is questioning this request first");
            let (objections, dropped, err) = challenge_request(&cfg, message).await;
            if !err.is_empty() {
                notice(&tx, format!("Challenger failed, continuing without it: {}", crate::util::text::head_chars(&err, 160)));
            } else if !objections.is_empty() {
                let notes = render_objections(&objections, dropped);
                notice(&tx, format!("Challenger objections:\n{notes}"));
                message_for_model.push_str(&format!("\n\n---\nChallenger objections (a second model argued against this request). [verified] ones are facts to deal with; [guess] ones are suspicions to check with your tools, never to repeat as fact. Before editing, say which objections you accept and which you reject and why, and tell the user about any real loss in their approach:\n{notes}"));
            } else {
                notice(&tx, "Challenger found nothing it could back with evidence");
            }
        }
        self.tidy_if_cache_went_cold(&tx).await;
        let wants = oriented.as_ref().map_or_else(|| self.orient_down || wants_change(ask), |o| o.intent == "change"); // both models failed to read it: no read-only cap, the main model decides
        let difficulty: &str = oriented.as_ref().map_or_else(|| classify(ask).0, |o| o.tier);
        if let Some(o) = &oriented {
            let w = o.hint();
            if !w.is_empty() {
                message_for_model.push_str(&format!("\n\n{w}"));
            }
            if o.rounds > 0 {
                // rides on this message only, so the cached prefix before it is untouched
                message_for_model.push_str(&format!("\n\n[Target: about {} tool rounds. A round is one turn, however many calls it holds, so batch independent lookups. Finishing within it earns points and overshooting costs them; if the work truly needs more, say how many and why, then carry on.]", o.rounds));
            }
        }
        if self.session.tool_registry.get("graph").is_some() {
            if let Some(map) = crate::util::symgraph::task_map(&cfg.cwd, message) {
                message_for_model.push_str(&format!("\n\n{map}"));
            }
        }
        self.touches = if wants { oriented.as_ref().map(|o| o.touches.clone()).unwrap_or_default() } else { String::new() };
        self.docs_nudged = false;
        self.session.client.set_auto_effort(effort(difficulty));
        let hint = length_hint(ask, difficulty, wants, reviewing);
        if !hint.is_empty() {
            message_for_model.push_str(&format!("\n\n[{hint}]"));
        }
        if wants {
            for q in crate::agent::routing_policy::side_questions(message) {
                message_for_model.push_str(&format!("\n\n[The person also asked: \"{q}\". Do the work, then answer this in the final reply with specific points from what you read. It is part of the request, so the short answer format must not drop it, and answering it is not a reason to edit more files.]"));
            }
        }
        self.broad = wants && is_broad(ask);
        if self.broad {
            message_for_model.push_str(&format!("\n\n[{SWEEP}]"));
        }
        self.sweep_pushed = 0;
        self.cut_continues = 0; // replies cut off at the output limit that were asked to carry on
        self.text_calls = 0; // replies that wrote a tool call as text instead of making it
        self.empty_replies = 0;
        self.plan_told = false;
        self.todo_idle = 0; // tool rounds since the todo list was last touched
        self.add_user(&message_for_model);
        self.unsent.clear();

        self.escalated = false;
        self.budget_hit = false;
        self.budget_warned = false;
        self.budget_extensions = 0;
        self.red_checks.clear();
        self.green_checks.clear();
        self.overflow_retried = false;
        self.calls_used = 0;
        self.calls_warned = false;
        self.quiet_rounds = 0; // rounds in a row where the model said nothing, only called tools
        self.edited = false;
        self.fix_nudged = false;
        self.capped_last = false;
        self.estimate = oriented.as_ref().map_or(0, |o| o.rounds);
        self.over_noted = false;
        self.read_only = !wants;
        self.wants_change = wants;
        self.is_review = reviewing;
        self.usage_base = self.spent(); // budget is per request
        self.review_sec = 0.0;
        self.tier = "default".into();
        let cfg = self.config.get();
        if cfg.auto_route && !cfg.model_pinned && cfg.model_role == "default" && (cfg.routing.contains_key("fast") || cfg.routing.contains_key("strong")) {
            let (tier, why) = match &oriented {
                Some(o) => (o.tier, "read by the fast model".to_string()),
                None => classify(ask),
            };
            let ctx = self.session.context_manager.lock().unwrap().latest_usage.total_tokens;
            let kept = keep_cached_model(tier, ctx);
            let mut tier = tier;
            if kept != tier {
                self.session.tracer.emit("route_kept", json!({"wanted": tier, "kept": kept, "reason": "prompt cache"}));
                tier = kept;
            }
            if let Some(chain) = cfg.routing.get(tier).and_then(|c| c.first()) {
                self.session.tracer.emit("route", json!({"tier": tier, "reason": why, "model": chain}));
                notice(&tx, format!("Routing to the {tier} model ({why}): {chain}"));
                self.tier = tier.into();
            } else {
                self.tier = "default".into();
            }
        }

        let mut task = TaskState::new(message, self.session.tracer.clone(), cfg.max_seconds, cfg.max_repairs);
        task.sub_agent = cfg.sub_agent;
        let registry = self.session.tool_registry.clone();
        let advisor_on = registry.get_tools().iter().any(|t| t.name() == "advisor");
        if advisor_on {
            if let Some(t) = registry.get("advisor") {
                if let Some(a) = t.as_any().downcast_ref::<crate::tools::builtin::advisor::AdvisorTool>() {
                    a.calls.store(0, std::sync::atomic::Ordering::SeqCst); // the limit is per request
                    a.forced.store(self.fix_run, std::sync::atomic::Ordering::SeqCst);
                }
            }
        }
        *registry.gate.lock().unwrap() = if self.fix_run && advisor_on { Some(review_fix::edit_gate(registry.clone())) } else { None }; // a fix run may not edit before the advisor has been consulted
        if self.is_review && self.read_only && registry.gate.lock().unwrap().is_none() {
            *registry.gate.lock().unwrap() = Some(Arc::new(|name: &str| (name == "parallel_tasks").then(|| REVIEW_NO_WORKERS.to_string())));
        }
        let base_writes = registry.writes.load(std::sync::atomic::Ordering::SeqCst);
        let mut advisor_nudges = 0;
        self.waited_base = self.session.approval_manager.waited();
        let before = snapshot(&self.config.get().cwd).await;
        let backup_ref = if before.is_some() { backup(&self.config.get().cwd).await } else { None };
        let mut before: Option<Snapshot> = before;
        let mut baseline: std::collections::BTreeMap<String, crate::agent::verify::CheckResult> = Default::default(); // clean-tree result per failing check
        let mut final_response: Option<String> = None;
        let (mut nudged, mut signatures_checked, mut reviews, mut grounded_once, mut rewrote, mut stop_blocks, mut pushed_once, mut red_pushed, mut proof_asked) = (false, false, 0usize, false, false, 0usize, false, false, false);
        let mut sent_back = self.fix_brief.clone(); // findings the author was just asked to fix: the next review checks that fix

        loop {
            self.failed = false;
            let mut fail_reason = String::new();
            // events from the loop are forwarded as they happen; the final text is read from TEXT_COMPLETE
            let (itx, mut irx) = tokio::sync::mpsc::unbounded_channel::<AgentEvent>();
            {
                let fut = self.agentic_loop(&mut task, itx);
                tokio::pin!(fut);
                loop {
                    tokio::select! {
                        _ = &mut fut => break,
                        Some(ev) = irx.recv() => { if let AgentEvent::TextComplete(c) = &ev { final_response = Some(c.clone()); } if let AgentEvent::AgentError { error } = &ev { fail_reason = error.clone(); } emit(&tx, ev); }
                    }
                }
                while let Ok(ev) = irx.try_recv() {
                    if let AgentEvent::TextComplete(c) = &ev {
                        final_response = Some(c.clone());
                    }
                    if let AgentEvent::AgentError { error } = &ev {
                        fail_reason = error.clone();
                    }
                    emit(&tx, ev);
                }
            }
            let cwd = self.config.get().cwd.clone();
            if self.failed || self.budget_hit {
                // a run cut short may still have edited files; the report must say which
                task.changed_files = changed_since(before.as_ref(), snapshot(&cwd).await.as_ref());
            }
            if self.failed {
                task.transition(TaskStatus::Failed, if fail_reason.is_empty() { "agent loop aborted" } else { &fail_reason }); // the trace says why, not just that it failed
                break;
            }
            let cfg = self.config.get();
            if self.budget_hit {
                // the checks cost no model tokens, so a run that ran out of budget mid-change still finds out if it broke the tree
                let checks = if task.changed_files.is_empty() { vec![] } else { select_checks(&cfg, &task.changed_files) };
                let mut results = vec![];
                for (cmd, sub) in &checks {
                    results.push(run_check(cmd, &cwd.join(sub), &cfg).await);
                }
                task.verification = results.iter().map(|r| r.to_value()).collect();
                if let Some(bad) = results.iter().find(|r| !r.ok) {
                    notice(&tx, format!("Stopped on the budget and the tree fails {}; the change is incomplete", bad.command));
                    task.transition(TaskStatus::Verifying, "checks after budget stop");
                    task.transition(TaskStatus::VerificationFailed, &bad.command);
                } else if !task.changed_files.is_empty() {
                    task.transition(TaskStatus::Unverified, "token budget reached");
                } else if self.wants_change && !self.read_only && !cfg.sub_agent {
                    // asked for a change and ran out before making one: calling that "unverified" hid a quarter of a week's cut-offs
                    task.transition(TaskStatus::Failed, "token budget reached");
                } else {
                    // a question or a review that had to answer early: nothing changed, and the answer says it was cut short
                    task.transition(TaskStatus::DoneNoChanges, "token budget reached");
                }
                break;
            }

            if self.wants_change && !self.is_review && !self.capped_last && !red_pushed && !self.red_checks.is_empty() {
                // "Done!" right after `npm run db:migrate` exited 1 makes the user find the failure and paste it back
                red_pushed = true;
                self.nudged("red_check");
                notice(&tx, format!("The answer came while `{}` was still failing; sending it back", self.red_checks[0]));
                self.add_user(&format!(
                    "You ended, but the last run of these commands failed: {}. Fix the cause and run them again until they pass. If one cannot pass here (it needs a database, a key or a service you cannot reach), say so plainly in the answer: which command, what it printed, and what the user has to do.",
                    self.red_checks.iter().map(|c| format!("`{c}`")).collect::<Vec<_>>().join(", ")
                ));
                continue;
            }

            if let Some(fr) = &final_response {
                // "say the word and I will..." after a clear request makes the user type "ok do it"; send it back once
                if self.wants_change && !self.is_review && !self.capped_last && !pushed_once && seeks_permission(fr) && !crate::agent::routing_policy::asks_how(&self.last_request) {
                    pushed_once = true;
                    self.nudged("asked_permission");
                    notice(&tx, "The answer asked permission for a clear step; sending it back to do it");
                    self.add_user("You ended by asking whether to go ahead. The request is clear: do that step now with your tools, then report in the short format. Ask only if it needs a choice the user alone can make, and say which choice.");
                    continue;
                }
            }
            if let (Some(fr), true) = (final_response.clone(), cfg.check_references) {
                let (n_refs, problems) = verify_references(&fr, &cwd).await;
                task.grounding = Some(json!({"checked": n_refs, "unverified": problems.iter().map(|p| format!("{} ({})", p.ref_, p.why)).collect::<Vec<_>>()}));
                self.session.tracer.emit("grounding", json!({"task": task.id, "checked": n_refs, "unverified": problems.len()}));
                // after a call-cap wrap-up there are no tools left to re-check with, so a retry would only pay for a second full-context answer
                if !problems.is_empty() && !grounded_once && !self.capped_last {
                    grounded_once = true;
                    notice(&tx, format!("{} cited reference(s) could not be verified; asking for a correction", problems.len()));
                    self.add_user(&correction_prompt(&problems));
                    continue;
                }
            }

            if let Some(fr) = final_response.clone() {
                if !rewrote && !self.pipelined && !cfg.sub_agent && shows_second_thoughts(message, &fr) {
                    rewrote = true;
                    notice(&tx, "The review argues with itself; asking for a clean rewrite");
                    self.add_user(REWRITE_PROMPT);
                    // a plain tool-free answer: re-entering the loop would hit the call cap and ask for a "next step" summary instead
                    let (itx, mut irx) = tokio::sync::mpsc::unbounded_channel::<AgentEvent>();
                    {
                        let fut = self.answer_without_tools(&task, itx);
                        tokio::pin!(fut);
                        loop {
                            tokio::select! {
                                _ = &mut fut => break,
                                Some(ev) = irx.recv() => { if let AgentEvent::TextComplete(c) = &ev { final_response = Some(c.clone()); } emit(&tx, ev); }
                            }
                        }
                        while let Ok(ev) = irx.try_recv() {
                            if let AgentEvent::TextComplete(c) = &ev {
                                final_response = Some(c.clone());
                            }
                            emit(&tx, ev);
                        }
                    }
                }
            }

            if before.is_none() {
                // started above the repo and the request did not name it: the files touched tell
                let targets: Vec<String> = self.session.tracer.snapshot().iter().filter(|e| e["event"] == "tool_call").filter_map(|e| e["target"].as_str().map(String::from)).collect();
                let root = pick_root(&cwd, message, self.root_hint.as_deref(), &targets);
                if self.adopt_root(root) {
                    notice(&tx, format!("Working in {} (where the files you touched live)", self.config.get().cwd.display()));
                    before = Some(Snapshot::new()); // nothing known from before: every changed file counts as this request's
                }
            }
            let cwd = self.config.get().cwd.clone();
            task.changed_files = changed_since(before.as_ref(), snapshot(&cwd).await.as_ref());
            if self.touches == "code" && !self.docs_nudged && !task.changed_files.is_empty() && task.changed_files.iter().all(|f| DOC_SUFFIX.iter().any(|s| f.to_lowercase().ends_with(s))) {
                // a bug was reported and only notes changed: the notes do not fix it
                self.docs_nudged = true;
                notice(&tx, "Only documentation changed for a code change; asking for the code");
                self.add_user(&format!(
                    "Only {} changed, but this request needs the code itself changed. A note in a doc does not fix a bug. Change the code that causes it, or say plainly in one sentence what stopped you.",
                    task.changed_files.iter().take(4).cloned().collect::<Vec<_>>().join(", ")
                ));
                continue;
            }
            let checks = select_checks(&cfg, &task.changed_files);
            // outside git we can't tell what changed, so only verify when asked to
            if task.changed_files.is_empty() && before.is_some() {
                if !nudged && final_response.is_some() && self.wants_change && !asks_user(final_response.as_deref().unwrap_or("")) && (has_file_code(final_response.as_deref().unwrap_or("")) || self.touches == "code") {
                    // models sometimes paste the solution instead of writing it
                    nudged = true;
                    notice(&tx, "Reply contained code but no file changed; asking the model to apply it");
                    self.add_user("You showed code but no file on disk changed. If the task needs changes, apply them now with your tools. If no change is needed, say so in one sentence.");
                    continue;
                }
                if stop_blocks < 2 {
                    if let Some(held) = self.session.hook_system.trigger_stop(message, final_response.as_deref().unwrap_or("")).await {
                        stop_blocks += 1;
                        notice(&tx, "A stop hook says the work is not finished; sending it back");
                        self.add_user(&held);
                        continue;
                    }
                }
                task.transition(TaskStatus::DoneNoChanges, "");
                break;
            }
            let only_docs = task.changed_files.iter().all(|f| DOC_SUFFIX.iter().any(|s| f.to_lowercase().ends_with(s)));
            if checks.is_empty() && cfg.auto_verify && !only_docs && self.green_checks.is_empty() && !proof_asked && self.wants_change && !self.is_review && !self.capped_last && !cfg.sub_agent && !task.expired() {
                // nothing of ours can check these files (a .ts file with no tsc installed, a .sql migration, a shell
                // script), so the one who wrote the change shows it works, once
                proof_asked = true;
                self.nudged("no_check");
                notice(&tx, "No automatic check covers these files; asking for one run that proves the change");
                self.add_user(&format!(
                    "zerd has no automatic check for {}. Before you finish, run one command that proves the change works: run the script, import the module, call the function, hit the endpoint, or a typecheck or syntax check if that is all there is. Then give the short answer again with that command and its result in one line. If nothing can be run here, say why in one sentence instead.",
                    task.changed_files.iter().take(4).cloned().collect::<Vec<_>>().join(", ")
                ));
                continue;
            }
            let mut failures = vec![];
            let mut checks_running = false;
            if !checks.is_empty() {
                task.transition(TaskStatus::Verifying, &format!("{} check(s)", checks.len()));
                notice(&tx, format!("Verifying: {}", checks.iter().map(|c| c.0.clone()).collect::<Vec<_>>().join(" && ")));
                if let Some(old) = self.late_stop.take() {
                    old.abort(); // still checking the previous change: this run checks the newer tree
                }
                // the checks run as their own task so they can outlive this request when they are slow
                let (jobs, dir, cfg_c) = (checks.clone(), cwd.clone(), cfg.clone());
                let mut work = tokio::spawn(async move {
                    let cfg_r = &cfg_c;
                    futures::future::join_all(jobs.iter().map(|(cmd, sub)| {
                        let d = dir.join(sub);
                        async move { run_check(cmd, &d, cfg_r).await }
                    }))
                    .await
                });
                // a fix run judges its own work by the checks, so it always waits for them
                let wait = if self.fix_run { None } else { cfg.verify_wait_sec.map(Duration::from_secs) };
                let waited = self
                    .unless_steered(async {
                        match wait {
                            Some(d) => tokio::time::timeout(d, &mut work).await.ok(),
                            None => Some((&mut work).await),
                        }
                    })
                    .await;
                let results = match waited {
                    None => {
                        work.abort();
                        self.back_to_work(&mut task, &tx);
                        continue;
                    }
                    Some(None) => {
                        notice(&tx, format!("Checks are taking longer than {}s; they carry on in the background and report when done", wait.map_or(0, |d| d.as_secs())));
                        self.finish_checks_later(work);
                        checks_running = true;
                        vec![]
                    }
                    Some(Some(done)) => done.unwrap_or_default(),
                };
                task.verification = results.iter().map(|r| r.to_value()).collect();
                let failing: Vec<_> = results.into_iter().zip(checks.iter()).filter(|(r, _)| !r.ok).map(|(r, (_, sub))| (r, cwd.join(sub))).collect();
                failures = failing.iter().map(|(r, _)| r.clone()).collect();
                if let (false, Some(r)) = (failing.is_empty(), backup_ref.as_deref()) {
                    // is it the run's doing at all? the same check on the pre-run files tells
                    let created: Vec<String> = task.changed_files.iter().filter(|f| !before.as_ref().map_or(false, |b| b.contains_key(*f))).cloned().collect();
                    let (new, old) = split_preexisting(&cwd, r, &task.changed_files, &created, failing, &mut baseline, &cfg).await;
                    for o in &old {
                        notice(&tx, format!("{} was already failing before your change; leaving it alone", o.command));
                        if let Some(v) = task.verification.iter_mut().find(|v| v["command"] == o.command.as_str()) {
                            v["preexisting"] = json!(true);
                        }
                    }
                    failures = new;
                }
            }

            if !failures.is_empty() {
                if task.repairs >= task.max_repairs || task.expired() {
                    task.transition(TaskStatus::VerificationFailed, &failures[0].command);
                    break;
                }
                task.transition(TaskStatus::Repairing, &failures[0].command);
                notice(&tx, format!("Verification failed ({}); repair attempt {}/{}", failures[0].command, task.repairs, task.max_repairs));
                self.add_user(&repair_prompt(&failures, &task.changed_files));
                continue;
            }

            if !signatures_checked && !task.expired() {
                // once per request: checks pass even when a public signature broke for callers outside the repo
                signatures_checked = true;
                let breaks = signature_breaks(&cwd, &task.changed_files, message).await;
                if !breaks.is_empty() {
                    self.session.tracer.emit("signature_breaks", json!({"task": task.id, "breaks": breaks.iter().take(10).collect::<Vec<_>>()}));
                    if task.status == TaskStatus::Verifying {
                        task.transition(TaskStatus::Running, "public signature changed");
                    }
                    notice(&tx, format!("The change breaks {} public signature(s) ({}); asking to keep old calls working", breaks.len(), breaks[0]));
                    self.add_user(&signature_prompt(&breaks));
                    continue;
                }
            }

            if self.fix_run && advisor_on && advisor_nudges < 2 && !task.expired() && review_fix::needs_final_advice(&registry, base_writes) {
                advisor_nudges += 1;
                notice(&tx, "The fix changed code since the advisor last looked; asking for a final consultation");
                self.add_user(review_fix::ADVISOR_LAST);
                continue;
            }

            // checks passed (or none exist): an independent reviewer reads the real diff
            let max_reviews = if self.fix_run { cfg.max_reviews.max(3) } else { cfg.max_reviews }; // a fix gets review, repair, re-review
            // with review off, a change to auth, payments, secrets or migrations is still worth a second look
            let risky = if cfg.auto_review || self.fix_run || !cfg.review_risky { None } else { risky_change(&task.changed_files) };
            if (cfg.auto_review || self.fix_run || risky.is_some()) && reviews < max_reviews && !task.expired() {
                reviews += 1;
                match (&risky, reviews) {
                    (Some((file, why)), 1) => notice(&tx, format!("{file} touches {why}, where a mistake is expensive; an independent reviewer is reading the diff...")),
                    _ => notice(&tx, "Independent review of the diff..."),
                }
                let t_review = Instant::now();
                let fixing = if sent_back.is_empty() { None } else { Some(sent_back.clone()) };
                let fr = final_response.clone().unwrap_or_default();
                let mut res = self.unless_steered(review_changes(&cfg, message, &task.changed_files, &task.verification, &fr, fixing.as_deref(), None, "review")).await;
                if let Some(r) = &res {
                    if cfg.second_reviewer && matches!(r.verdict.as_str(), "PASS" | "ISSUES") {
                        notice(&tx, "Second reviewer looking for what the first missed...");
                        let prior = if r.findings.is_empty() { "VERDICT: PASS".to_string() } else { r.findings.clone() };
                        let second = self.unless_steered(review_changes(&cfg, message, &task.changed_files, &task.verification, &fr, None, Some(&prior), "review2")).await;
                        res = second.map(|s| merge_reviews(r, &s));
                    }
                }
                let Some(mut res) = res else {
                    self.review_sec += t_review.elapsed().as_secs_f64();
                    reviews -= 1; // this review was of a diff that is about to change; the final one still gets its turn
                    self.back_to_work(&mut task, &tx);
                    continue;
                };
                // a review that is about to send the work back is not the one to prove: the fix changes first, then it is proven once
                let will_repair = res.verdict == "ISSUES" && reviews < max_reviews && task.repairs < task.max_repairs;
                if self.fix_run && !will_repair && !task.expired() {
                    notice(&tx, "A fresh verifier is trying to prove the fix works (it runs the checks and the findings' triggers in a copy)");
                    let checks_text = task.verification.iter().map(|c| format!("{}={}", c["command"].as_str().unwrap_or(""), if c["ok"] == true { "ok" } else { "FAILED" })).collect::<Vec<_>>().join(", ");
                    let fv = verify_fix(&cfg, &cfg.review_pipeline_role, &cfg.cwd, &self.fix_brief, &fr, &task.changed_files, &checks_text).await;
                    if fv.ran {
                        notice(&tx, format!("Fix verifier: {} ({})", if fv.passed { "PASS" } else { "FAIL" }, fv.summary()));
                        if !fv.passed {
                            res = ReviewResult { verdict: "ISSUES".into(), findings: format!("{}A fresh verifier ran the fix and found:\n{}", if res.verdict == "ISSUES" { format!("{}\n\n", res.findings) } else { String::new() }, fv.feedback()), blocked: res.blocked };
                        }
                    }
                }
                self.review_sec += t_review.elapsed().as_secs_f64();
                if res.blocked > 0 {
                    notice(&tx, format!("The reviewer had {} tool call(s) blocked by hooks or permission rules, so its check was narrower than usual", res.blocked));
                }
                task.review = Some(json!({"verdict": res.verdict, "findings": res.findings}));
                self.session.tracer.emit("review", json!({"task": task.id, "verdict": res.verdict, "findings": crate::util::text::head_chars(&res.findings, 1500)}));
                if res.verdict == "ISSUES" && reviews >= max_reviews {
                    // the fix itself was reviewed and still has problems: another blind round costs ~50 calls, so the user decides
                    notice(&tx, format!("The review of the fix still found problems (left unfixed):\n{}", res.findings));
                } else if res.verdict == "ISSUES" && task.repairs < task.max_repairs {
                    sent_back = res.findings.clone();
                    task.transition(TaskStatus::Repairing, "review findings");
                    notice(&tx, "Reviewer found issues; sending them back for a fix");
                    self.add_user(&format!(
                        "An independent reviewer examined your diff and reported these problems:\n\n{}\n\nFix every finding that is real and sits in code this diff added or changed, whatever its severity: a dead control or a blank page that your change caused is a regression, not a nit. Only defer a finding about code you did not touch, and list it. If a finding is wrong, say why in one sentence and leave the code alone. After the fix, run what proves it (the page, request or test the finding names), not only the typecheck.",
                        res.findings
                    ));
                    continue;
                }
            }

            if stop_blocks < 2 && !task.expired() {
                if let Some(held) = self.session.hook_system.trigger_stop(message, final_response.as_deref().unwrap_or("")).await {
                    stop_blocks += 1;
                    notice(&tx, "A stop hook says the work is not finished; sending it back");
                    self.add_user(&held);
                    continue;
                }
            }

            if checks_running {
                task.transition(TaskStatus::Unverified, "checks still running in the background");
            } else if task.verification.iter().any(|v| v["preexisting"] == true) {
                task.transition(TaskStatus::Unverified, "a check was already failing before the change");
            } else if !checks.is_empty() {
                task.transition(TaskStatus::Verified, "");
            } else if !self.green_checks.is_empty() && self.red_checks.is_empty() && !task.changed_files.is_empty() {
                // no check of our own for this project, but the model ran its own after the last edit and they passed
                task.transition(TaskStatus::Verifying, "checks the model ran");
                task.transition(TaskStatus::Verified, &self.green_checks.join("; "));
            } else if only_docs {
                task.transition(TaskStatus::Unverified, "only docs changed, nothing to run");
            } else {
                task.transition(TaskStatus::Unverified, "no verification commands");
            }
            break;
        }

        let cwd = self.config.get().cwd.clone();
        if task.status == TaskStatus::Verified && !task.changed_files.is_empty() && task.review.as_ref().map_or(true, |r| r["verdict"] != "ISSUES") {
            Learned::new(&cwd).record_run(message, &task.changed_files); // which files fixed what, for next time
        }
        if let (Some(r), false) = (&backup_ref, task.changed_files.is_empty()) {
            let created: Vec<String> = task.changed_files.iter().filter(|f| !before.as_ref().map_or(false, |b| b.contains_key(*f))).cloned().collect();
            let info = json!({"ref": r, "files": task.changed_files, "created": created});
            let mut cp = info.clone();
            cp["request"] = json!(crate::util::text::head_chars(message, 80));
            self.undo_info = Some(info);
            self.checkpoints.push(cp);
            if self.checkpoints.len() > 20 {
                self.checkpoints.remove(0);
            }
        }
        if self.estimate > 0 {
            let cut = self.capped_last || self.budget_hit || task.status == TaskStatus::Failed;
            let intent = if self.is_review { "review" } else if self.wants_change { "change" } else { "ask" };
            let pts = ledger::record(&crate::config::loader::get_data_dir(), intent, self.estimate, self.calls_used, cut);
            notice(&tx, ledger::verdict(self.estimate, self.calls_used, cut, pts));
            self.session.tracer.emit("calibration", json!({"estimate": self.estimate, "actual": self.calls_used, "cut": cut, "points": pts}));
        }
        self.last_answer = final_response.clone().unwrap_or_default();
        if let Some(fr) = &final_response {
            if fr.split_whitespace().count() > LONG_ANSWER_WORDS && !self.is_review {
                self.long_answer = fr.split_whitespace().count();
            }
        }
        self.session.hook_system.trigger_after_agent(message, final_response.as_deref().unwrap_or("")).await;
        let mut report = task.report();
        report["undo"] = json!(self.undo_info.is_some());
        let events = self.session.tracer.snapshot();
        report["usage"] = summarize_usage(&events, &self.config.get().pricing);
        let sum = |ev: &str, ok_only: bool| -> f64 { events.iter().filter(|e| e["event"] == ev && (!ok_only || e["ok"] == true)).map(|e| e["sec"].as_f64().unwrap_or(0.0)).sum() };
        let r1 = |x: f64| (x * 10.0).round() / 10.0;
        report["timings"] = json!({
            "wall_sec": r1(task.started.elapsed().as_secs_f64()),
            "model_sec": r1(sum("model_call", true)),
            "tool_sec": r1(sum("tool_call", false)), // parallel calls overlap, so this can exceed wall
            "verify_sec": r1(task.verification.iter().map(|c| c["seconds"].as_f64().unwrap_or(0.0)).sum()),
            "review_sec": r1(self.review_sec),
        });
        {
            // nothing is collected unless the user opted in; see util/telemetry.rs for the exact record
            let cfg = self.config.get();
            let extra = json!({
                "model": cfg.model.name,
                "intent": if self.is_review { "review" } else if self.wants_change { "change" } else { "ask" },
                "estimate": self.estimate,
                "rounds": self.calls_used,
                "checks": task.verification.len(),
                "checks_failed": task.verification.iter().filter(|c| c["ok"] != true).count(),
                "window": self.session.context_manager.lock().unwrap().context_window,
                "top_ups": self.budget_extensions,
                "approval": cfg.approval.value(),
                "pi_mode": cfg.pi_mode as i64,
                "sub_agent": cfg.sub_agent as i64,
            });
            crate::util::telemetry::send_if_enabled(&crate::config::loader::get_data_dir(), &events, extra);
        }
        if !self.unasked.is_empty() {
            // said by zerd itself, so the user hears it even when the model's answer forgets to
            report["needs_approval"] = json!(self.unasked);
            if task.status != TaskStatus::Failed {
                notice(&tx, format!("Skipped {} step(s) that needed approval with nobody to ask: {}. Re-run with --approval auto to allow them.", self.unasked.len(), self.unasked.join("; ")));
            }
        }
        self.task = Some(task);
        emit(&tx, AgentEvent::AgentEnd { response: final_response, report });
    }

    /// Runs a check or a review, but drops it the moment the user types a steering note, so the note is worked on now
    /// instead of after a minutes-long typecheck. Returns None when dropped.
    /// Slow checks keep running after the request has answered. When they finish the user is told straight away, and
    /// a failure waits in `late_checks` for the model's next turn.
    fn finish_checks_later(&mut self, work: tokio::task::JoinHandle<Vec<crate::agent::verify::CheckResult>>) {
        self.late_stop = Some(work.abort_handle());
        let waiting = self.late_checks.clone();
        tokio::spawn(async move {
            let Ok(results) = work.await else { return }; // cancelled: a newer change is being checked instead
            if let Some(ui) = crate::ui::prompt::asker() {
                ui.say(late_check_report(&results));
            }
            *waiting.lock().unwrap() = results;
        });
    }

    async fn unless_steered<F: std::future::Future>(&self, work: F) -> Option<F::Output> {
        tokio::pin!(work);
        loop {
            tokio::select! {
                out = &mut work => return Some(out),
                _ = tokio::time::sleep(Duration::from_millis(250)) => {
                    if !self.steering.lock().unwrap().is_empty() { return None; }
                }
            }
        }
    }

    fn back_to_work(&mut self, task: &mut TaskState, tx: &Tx) {
        if task.status == TaskStatus::Verifying {
            task.transition(TaskStatus::Running, "user steered");
        }
        notice(tx, "Checks paused for your note; the whole diff is checked once it is done");
        for note in self.drain_steering() {
            self.add_user(&format!("[the user added, while you were working; it steers the task in progress, not a new task:] {note}"));
            notice(tx, format!("steering received: {}", crate::util::text::head_chars(&note, 80)));
        }
    }

    async fn agentic_loop(&mut self, task: &mut TaskState, tx: Tx) {
        let cfg = self.config.get();
        if self.pipelined && self.read_only {
            // the reviewer and verifier already read the code, and each finding is already one line: no model call, no tool loop to wander in
            let text = self.pipeline_answer.clone();
            self.session.context_manager.lock().unwrap().add_assistant_message(Some(&text), None, None, None);
            emit(&tx, AgentEvent::TextDelta(text.clone()));
            emit(&tx, AgentEvent::TextComplete(text));
            return;
        }
        let max_turns = cfg.max_turns;
        let mut loop_warnings = 0;
        for _turn in 0..max_turns {
            let cfg = self.config.get();
            // minutes spent on an approval prompt are the user's, not the agent's: they never use up the budget
            task.paused = self.session.approval_manager.waited() - self.waited_base;
            if task.expired() {
                // out of time: answer from what is known instead of failing with nothing to show
                self.wrap_up(task, &tx, &format!("Time budget reached ({:.0}s)", cfg.max_seconds), "The time budget is spent").await;
                return;
            }
            let limit = self.call_limit();
            if limit > 0 && self.calls_used >= limit {
                self.wrap_up(task, &tx, &format!("Call budget reached ({limit} tool rounds)"), &format!("The {limit}-round limit for this request is reached")).await;
                self.budget_hit = false; // the answer was produced; this is not a token-budget failure
                self.capped_last = true;
                return;
            }
            if self.estimate > 0 && self.calls_used > self.estimate && !self.over_noted {
                self.over_noted = true;
                self.add_user(&format!("You are past your estimate of {} rounds ({} used). Keep going only if the work needs it: say in one line what is taking longer, then batch what is left into parallel calls.", self.estimate, self.calls_used));
            }
            if limit > 0 && self.calls_used >= ((limit as f64 * 0.7) as usize).max(1) && !self.calls_warned {
                self.calls_warned = true;
                let warning = if self.is_review {
                    // unread candidates turn into guessed HIGHs at the cap: spend what is left on the best two
                    format!("Budget check: {} of {limit} tool rounds used. Pick the two candidates most likely to be real defects and use the remaining rounds, in parallel, to read the code that proves or refutes each. Drop the rest: an unread candidate is not a finding. Keep any notes to 5 lines (candidate, file:line, checked or not; no severity).", self.calls_used)
                } else if !self.read_only {
                    format!("Budget check: {} of {limit} tool rounds used. Fix the HIGH items first, batch the remaining edits and checks into parallel calls, and list anything you will not get to so the user can decide.", self.calls_used)
                } else {
                    format!("Budget check: {} of {limit} tool rounds used. Put every lookup you still need into your next turn as parallel calls, then answer. Before reading more, jot short working notes (candidate, file:line, checked or not; no severity, nothing called verified until you read the code that proves it): old tool results get cleared, your own text stays. If the task asks for an output file, write it now, then refine it.", self.calls_used)
                };
                self.add_user(&warning);
            }
            if self.quiet_rounds >= 8 {
                // thinking lasts one request and old tool results get cleared: only written text survives
                self.quiet_rounds = 0;
                self.add_user(if self.broad { SWEEP_CHECKPOINT } else { "Checkpoint: you have looked things up for several rounds without writing anything down. Before the next lookup, jot working notes in at most 3 lines (candidate, file:line, checked or not; no severity) and what you will check next. If you have verified a defect, fix it now." });
            } else if self.wants_change && !self.broad && !self.edited && !self.fix_nudged && self.calls_used >= 15 {
                self.fix_nudged = true;
                self.add_user(&format!("{} rounds in and nothing is changed yet. If you have a verified finding, name your root cause in one line and make the smallest fix for it (ask first only if it adds or restyles something the user will see; a guard, a prop or a handler change needs no asking). Do not add new files or components just to have a change. If you have no cause, say what you traced and why it holds.", self.calls_used));
            }
            // What sub-agents and parallel workers spent is part of this request's bill, so it joins the total that
            // /cost and the report show. It does not use up this agent's token budget: that budget guards this
            // agent's own context from running away, and each sub-agent already has limits of its own.
            if let Some(u) = crate::tools::subagents::take_usage(self.config.key()) {
                self.session.context_manager.lock().unwrap().add_usage(&u);
                self.usage_base += u.prompt_tokens - u.cached_tokens + u.completion_tokens;
            }
            let used = self.spent() - self.usage_base;
            let mut budget = cfg.max_total_tokens * (1 + self.budget_extensions);
            if used > budget && self.edited && !self.read_only && self.budget_extensions < MAX_BUDGET_EXTENSIONS {
                // Mid-change, a text-only stop leaves half-applied edits and makes the user type "continue".
                // Fold the context down and give the same budget again; the hard stop is still (1 + extensions) x budget.
                // Example: a 300k budget runs as 300k, then 600k, then 900k, and only then wraps up.
                self.budget_extensions += 1;
                budget = cfg.max_total_tokens * (1 + self.budget_extensions);
                self.budget_warned = false;
                notice(&tx, format!("Token budget topped up ({} of {MAX_BUDGET_EXTENSIONS}): folding the context and carrying on with the change", self.budget_extensions));
                if !self.compact(true).await {
                    self.session.context_manager.lock().unwrap().prune_tool_outputs();
                }
                self.add_user("The token budget was topped up so the change can finish. Carry on from where you stopped: do not recap earlier work, and keep to the remaining steps.");
            }
            if used > budget {
                self.wrap_up(task, &tx, &format!("Token budget reached ({budget})"), "Token budget is spent").await;
                return;
            }
            // the warning is for the last stretch of the whole allowance, so it counts from the topped-up budget
            if used as f64 > 0.7 * budget as f64 && !self.budget_warned {
                self.budget_warned = true;
                self.add_user("Budget check: about 70% of the token budget is used. Stop broad searching. Jot short working notes (candidate, file:line, checked or not; no severity), since old tool results get cleared. If the task asks for an output file, write it now. Read only what you must, then answer or make the change.");
            }
            for job in finished_jobs() {
                let state = if job.returncode() == Some(0) { "finished".to_string() } else { format!("failed (exit {})", job.returncode().unwrap_or(-1)) };
                notice(&tx, format!("Background job {} {state} in {:.0}s: {}", job.id, job.started.elapsed().as_secs_f64(), job.command));
                self.add_user(&format!("[background job {} `{}` {state}; last output:]\n{}", job.id, job.command, job.tail(15)));
            }
            let late: Vec<_> = std::mem::take(&mut *self.late_checks.lock().unwrap()).into_iter().filter(|r| !r.ok).collect();
            if !late.is_empty() {
                // the user was already told when they finished; the model hears it at its next turn
                self.add_user(&format!(
                    "[checks on your previous change finished after you had answered, and failed. They may have been failing before that change, so look before you fix:]\n{}",
                    late.iter().map(|r| format!("`{}`\n{}", r.command, crate::util::text::tail_chars(&r.output_tail, 1500))).collect::<Vec<_>>().join("\n\n")
                ));
            }
            for (id, name, report) in crate::tools::subagents::finished(self.config.key()) {
                notice(&tx, format!("Background sub-agent {name} finished"));
                self.add_user(&format!("[background sub-agent '{name}' ({id}) finished; check its report like any evidence before relying on it:]\n{report}"));
            }
            self.remind_each_turn();
            self.session.increment_turn();
            let mut response_text = String::new();

            // check for context overflow, against the model actually in use
            {
                let window = self.session.client.registry.get(&cfg.model.name).context_length;
                self.session.context_manager.lock().unwrap().context_window = window;
            }
            let (needs, turn_count) = (self.session.context_manager.lock().unwrap().needs_compression(), self.session.turns());
            if needs && turn_count >= self.compact_retry_turn {
                let refilled = self.last_compact_turn.map_or(false, |t| turn_count - t <= 3);
                self.thrash = if refilled { self.thrash + 1 } else { 0 };
                if self.thrash >= 3 {
                    // Claude Code's breaker: what is kept (the tail, restored files) already fills the window, so another
                    // summary just pays for itself and changes nothing. Shed tool output and stop summarising for a while.
                    self.thrash = 0;
                    self.compact_retry_turn = turn_count + 10;
                    self.session.context_manager.lock().unwrap().prune_tool_outputs();
                    notice(&tx, "The context fills up again right after each compaction; pruned old tool output instead");
                } else if self.compact(true).await {
                    self.last_compact_turn = Some(turn_count);
                    notice(&tx, "Context compacted (recent messages kept)");
                } else {
                    // the summarizer failed: shed old tool output instead, and don't pay for another failing summary call on every turn
                    self.compact_retry_turn = turn_count + 3;
                    self.session.context_manager.lock().unwrap().prune_tool_outputs();
                    notice(&tx, "Could not summarise the context; pruned old tool output instead");
                }
            }

            let schemas = self.session.tool_registry.get_schemas();
            let role = self.role(task);
            self.last_role = role.clone();

            // early reads from a reply whose calls were then not run (cut off, rejected): their results are not wanted
            self.early.drain().for_each(|(_, (_, run))| run.abort());
            let mut tool_calls: Vec<ToolCall> = vec![];
            let (mut usage, mut cut, mut reasoning): (Option<TokenUsage>, bool, Option<String>) = (None, false, None);
            let (mut failed, mut overflowed) = (false, false);
            let (mut held, mut streaming) = (String::new(), false); // text waits until we know whether tool calls follow (then it shows as one line)

            let messages = self.session.context_manager.lock().unwrap().get_messages();
            let mut stream = self.session.client.chat_completion(messages, if schemas.is_empty() { None } else { Some(schemas) }, true, &role);
            let deadline = Instant::now() + Duration::from_secs_f64(task.remaining());
            while let Some(event) = next_within(deadline, &mut stream).await {
                match event {
                    StreamEvent::TextDelta(content) => {
                        response_text.push_str(&content);
                        if streaming {
                            emit(&tx, AgentEvent::TextDelta(content));
                        } else {
                            held.push_str(&content);
                            if held.chars().count() > HOLD_CHARS {
                                streaming = true;
                                emit(&tx, AgentEvent::TextDelta(std::mem::take(&mut held)));
                            }
                        }
                    }
                    StreamEvent::Notice(n) => notice(&tx, n),
                    StreamEvent::ToolCallComplete(tc) => {
                        // "read a.rs, read b.rs, grep x" in one reply: a.rs is being read while b.rs is still being written.
                        // Only while every call so far is read-only, so nothing ever runs ahead of an edit or a command.
                        let fixed = self.session.tool_registry.repair(&tc);
                        tool_calls.push(tc);
                        if cfg.early_tools && tool_calls.iter().all(|c| self.is_parallel_safe(&self.session.tool_registry.repair(c))) {
                            let (session, cwd, call) = (self.session.clone(), cfg.cwd.clone(), fixed.clone());
                            let run = tokio::spawn(async move { session.tool_registry.invoke(&call.name, Value::Object(call.arguments.clone()), &cwd, &session.hook_system, Some(&session.approval_manager), &call.call_id).await });
                            self.early.insert(fixed.call_id.clone(), (fixed, run));
                        }
                    }
                    StreamEvent::Error(err) => {
                        if overflow().is_match(&err) && !not_overflow().is_match(&err) && !self.overflow_retried {
                            overflowed = true; // the provider says the prompt is too big: compact and ask again, once
                            break;
                        }
                        if err.starts_with(OUT_OF_TIME) {
                            // the clock ran out in the middle of a model call. What was read so far is still in context,
                            // so turn it into an answer, the same as when the clock runs out between calls
                            drop(stream);
                            self.wrap_up(task, &tx, &format!("Time budget reached ({:.0}s)", cfg.max_seconds), "The time budget is spent").await;
                            return;
                        }
                        failed = true;
                        self.failed = true;
                        self.session.hook_system.fire(HookTrigger::OnError, &[("error", err.clone())]).await;
                        emit(&tx, AgentEvent::AgentError { error: if err.is_empty() { "Unknown error occurred.".into() } else { err } });
                        break;
                    }
                    StreamEvent::MessageComplete { usage: u, finish_reason, reasoning: r, .. } => {
                        usage = u;
                        cut = matches!(finish_reason.as_deref(), Some("length" | "max_tokens" | "max_output_tokens"));
                        reasoning = r;
                    }
                    _ => {}
                }
            }
            drop(stream);

            let tool_calls: Vec<ToolCall> = tool_calls.iter().map(|tc| self.session.tool_registry.repair(tc)).collect(); // `Read` -> read_file, old_str -> old_string
            if !held.is_empty() && !overflowed {
                emit(&tx, AgentEvent::TextDelta(if tool_calls.is_empty() { held.clone() } else { interim_line(&held) }));
            }

            if overflowed {
                self.overflow_retried = true;
                self.nudged("overflow");
                notice(&tx, "The model refused the prompt as too long; compacting and retrying");
                if self.compact(true).await {
                    continue;
                }
                self.session.context_manager.lock().unwrap().prune_tool_outputs();
                notice(&tx, "Could not summarise; pruned old tool output and retrying");
                continue;
            }

            if failed {
                return; // a failed request must not leave a half-written assistant turn in history
            }

            // a cut reply's calls are never run, and their half-written arguments (sometimes megabytes of runaway text) would
            // sit in the history and block every later request, so only their names are kept
            let calls_json: Option<Vec<Value>> = if tool_calls.is_empty() {
                None
            } else {
                Some(tool_calls.iter().map(|tc| json!({"id": tc.call_id, "type": "function", "function": {"name": tc.name, "arguments": if cut { "{}".to_string() } else { Value::Object(tc.arguments.clone()).to_string() }}})).collect())
            };
            {
                let model = self.session.client.last_model();
                self.session.context_manager.lock().unwrap().add_assistant_message((!response_text.is_empty()).then_some(response_text.as_str()), calls_json, reasoning.as_deref(), model.as_deref());
            }
            if !response_text.is_empty() {
                emit(&tx, AgentEvent::TextComplete(response_text.clone()));
                self.session.loop_detector.lock().unwrap().record_response(&response_text);
            }

            self.quiet_rounds = if response_text.is_empty() { self.quiet_rounds + 1 } else { 0 };
            self.todo_idle = if tool_calls.iter().any(|tc| tc.name == "todos") { 0 } else { self.todo_idle + 1 };
            let edits = tool_calls.iter().any(|tc| tc.name == "edit" || tc.name == "write_file");
            self.edited = self.edited || edits;
            self.calls_used += 1; // one round trip however many calls it holds: reading 5 files at once costs what reading 1 does
            if edits {
                self.read_only = false; // it turned out to be a change after all: the read cap no longer applies
            }
            if tool_calls.is_empty() {
                if let Some(u) = &usage {
                    {
                        let mut cm = self.session.context_manager.lock().unwrap();
                        cm.set_latest_usage(u.clone());
                        cm.add_usage(u);
                    }
                    self.forgive_rewarm(u);
                }
                if text_call().is_match(&response_text) && self.text_calls < 2 {
                    self.text_calls += 1;
                    self.nudged("text_call");
                    self.remind("Your last reply wrote a tool call as text. Text is not run, so nothing happened. Make the call through the tool-calling interface.");
                    continue;
                }
                if response_text.trim().is_empty() && !cut && self.empty_replies < 2 {
                    // thinking with nothing after it: common on open models, and the user would see a blank answer
                    self.empty_replies += 1;
                    self.nudged("empty_reply");
                    self.remind("Your last reply had no text and no tool call, so the user saw nothing. If work is left, make the next tool call now. If you are done, write the answer as plain text.");
                    continue;
                }
                if cut && self.cut_continues < 2 {
                    self.cut_continues += 1;
                    self.nudged("cut_off");
                    self.add_user("Your last reply was cut off by the output token limit. Continue exactly where it stopped, without repeating what you already wrote or apologising; if you were inside a code block or table, reopen it first. Keep what remains short, or do it in smaller pieces through tool calls.");
                    continue;
                }
                let left = if self.broad { self.unfinished_todos() } else { vec![] };
                if !left.is_empty() && self.sweep_pushed < 2 {
                    // a sweep that ends with open steps stops after the first few files, which is what this guards against
                    self.sweep_pushed += 1;
                    self.nudged("stopped_early");
                    self.add_user(&format!("{} step(s) on your list are still open: {}. Finish them (fix, or mark done with a one-line reason it is fine) before you answer.", left.len(), left.iter().take(6).cloned().collect::<Vec<_>>().join("; ")));
                    continue;
                }
                let waiting = crate::tools::subagents::pending(self.config.key());
                if waiting > 0 {
                    // the model has nothing more to do itself, but work it started is still out: its reports are part of the answer
                    notice(&tx, format!("Waiting for {waiting} background sub-agent(s)"));
                    crate::tools::subagents::wait_for_report(self.config.key()).await;
                    continue;
                }
                self.prune(false, false);
                return;
            }

            let mut results: Vec<(String, String, bool)> = vec![];
            if cut {
                // the last call's arguments are probably truncated: running it would write half a file, and "not valid JSON"
                // made the model resend the same oversized call. Run none of them and say why.
                self.nudged("cut_call");
                notice(&tx, "The reply hit the output token limit; its tool calls were not run");
                results = tool_calls.iter().map(|tc| (tc.call_id.clone(), CUT_CALL.to_string(), true)).collect();
            } else {
                self.run_tool_calls(&tool_calls, &tx, &mut results).await;
            }
            {
                let mut cm = self.session.context_manager.lock().unwrap();
                for (id, content, _) in &results {
                    cm.add_tool_result(id, content);
                }
            }
            for note in self.drain_steering() {
                self.add_user(&format!("[the user added, while you were working:] {note}"));
                notice(&tx, format!("steering received: {}", crate::util::text::head_chars(&note, 80)));
            }
            if self.unasked.len() >= MAX_UNASKED {
                // one refusal is a step to skip; three means the task cannot be done without the approval, and
                // going on would only burn the budget on more of the same
                self.failed = true;
                emit(&tx, AgentEvent::AgentError { error: format!("Stopped: this task needs approval and the run cannot ask. Waiting on: {}. Re-run with --approval auto-edit (edits) or --approval auto (edits and commands), or start zerd interactively.", self.unasked.join("; ")) });
                return;
            }

            let churn = self.session.loop_detector.lock().unwrap().take_churn();
            if let Some(c) = churn {
                self.nudged("edit_churn");
                self.add_user(&format!("{c}. Stop and check: re-read the file and run its check to see where it stands. If the edits are not converging, write the design in two lines (what this file should end up doing, and why the earlier edits missed it) before the next edit, or tell the user what is blocking you."));
            }
            let looped = self.session.loop_detector.lock().unwrap().check_for_loop();
            if let Some(err) = looped {
                loop_warnings += 1;
                self.nudged("loop");
                self.escalated = true; // stuck: bring in the strong model
                if loop_warnings >= 3 {
                    // the model was told twice and kept going; stop burning budget
                    self.failed = true;
                    emit(&tx, AgentEvent::AgentError { error: format!("Stopped: {err}") });
                    return;
                }
                self.add_user(&create_loop_breaker_prompt(&err));
            }

            if let Some(u) = &usage {
                {
                    let mut cm = self.session.context_manager.lock().unwrap();
                    cm.set_latest_usage(u.clone());
                    cm.add_usage(u);
                }
                self.forgive_rewarm(u);
                // big prompts that never hit the cache are re-billed in full every call
                self.uncached_streak = if u.prompt_tokens > 15_000 && u.cached_tokens == 0 { self.uncached_streak + 1 } else { 0 };
                if self.uncached_streak == 3 {
                    notice(&tx, format!("{} is not reusing its prompt cache (3 large calls, 0 cached tokens). Every call re-bills the whole context; a Claude or deepseek model is much cheaper here.", cfg.model.name));
                }
            }

            self.last_model_call = Some(Instant::now());
            self.prune(true, false);
        }
        self.wrap_up(task, &tx, &format!("Turn limit reached ({max_turns})"), "Turn limit reached").await;
    }

    /// What the request is, how hard, and where to look, read by the fast model next to a map of this repo.
    async fn orient(&mut self, message: &str) -> Option<Orientation> {
        self.orient_down = false;
        let cfg = self.config.get();
        // With no small model set up, the main model reads the request itself. Skipping the step instead (as before)
        // meant no round estimate and an empty calibration ledger for anyone who never wrote a [routing] table.
        let role = if cfg.routing.contains_key("fast") { "fast" } else if cfg.routing.contains_key("summarize") { "summarize" } else { "default" };
        if !(cfg.auto_orient && cfg.model_role == "default" && !cfg.sub_agent && message.trim().chars().count() >= 20 && !message.starts_with('/')) {
            return None;
        }
        if self.repo_map.is_none() {
            let (cwd, forbidden) = (cfg.cwd.clone(), cfg.forbidden_paths.clone());
            self.repo_map = Some(tokio::task::spawn_blocking(move || repo_map(&cwd, &forbidden)).await.unwrap_or_default());
        }
        let t0 = Instant::now();
        let map = self.repo_map.clone().unwrap_or_default();
        let track = ledger::track_record(&crate::config::loader::get_data_dir());
        let mut found = orient(&self.session.client, role, message, &map, &track).await;
        if found.is_none() {
            found = orient(&self.session.client, "default", message, &map, &track).await; // the helper chain is down (bad key, quota): the main model can read a request too
        }
        let mut f = json!({"ok": found.is_some(), "sec": (t0.elapsed().as_secs_f64() * 100.0).round() / 100.0});
        if let Some(o) = &found {
            f["intent"] = json!(o.intent);
            f["tier"] = json!(o.tier);
            f["rounds"] = json!(o.rounds);
            f["where"] = json!(o.where_);
            f["queries"] = json!(o.queries);
        }
        self.session.tracer.emit("orient", f);
        self.orient_down = found.is_none();
        found
    }

    /// Search in a throwaway read-only context so the main (pricier, ever-growing) context starts from findings
    /// instead of 30 grep results.
    async fn scout(&mut self, message: &str) -> String {
        let cfg = self.config.get();
        let reg = self.session.tool_registry.clone();
        if !(cfg.auto_scout && cfg.model_role == "default" && reg.get("subagent_investigator").is_some()) {
            return String::new();
        }
        if message.chars().count() < 80 || classify(typed(message)).0 == "fast" {
            return String::new();
        }
        self.session.tracer.emit("scout", json!({"goal": crate::util::text::head_chars(message, 200)}));
        let res = reg.invoke("subagent_investigator", json!({"goal": format!("{SCOUT_BRIEF}\n\nThe request:\n{message}")}), &cfg.cwd, &self.session.hook_system, Some(&self.session.approval_manager), "").await;
        if !res.success {
            self.scout_error = res.error.unwrap_or_else(|| "unknown error".into()).lines().next().unwrap_or("").to_string();
            return String::new();
        }
        let out = crate::util::text::head_chars(&res.output, 6000);
        self.session.tracer.emit("scout_done", json!({"report": out})); // the only copy until the session is saved
        out
    }

    /// Out of budget or turns: one last tool-free call, so the work done so far becomes an answer (or a verdict, for a
    /// reviewer) instead of a bare failure.
    async fn wrap_up(&mut self, task: &TaskState, tx: &Tx, notice_text: &str, why: &str) {
        self.budget_hit = true;
        notice(tx, format!("{notice_text}; wrapping up with what is known"));
        let ask = if self.is_review {
            "Answer now with the review itself: only defects you verified by reading the code, as `severity file:line  what is wrong -> fix`, at most 6. Leave out anything you retracted, could not verify, or that is redundancy or style; a guess is `(unverified)` and at most MEDIUM. At most one closing line, for a check you did not run."
        } else {
            "Answer now from what you have already read, in at most 6 bullets: what is done, what is still open (file:line), and the one next step. This text is all that survives into the next request, so name files and open items, and leave out anything already finished and anything you only guessed. No preamble. If you were asked for a specific reply format, use it."
        };
        self.add_user(&format!("{why}. No more tool calls. {ask}"));
        self.answer_without_tools(task, tx.clone()).await;
    }

    /// One last model call with no tools, answering from what is already in context.
    async fn answer_without_tools(&mut self, task: &TaskState, tx: Tx) {
        let mut text = String::new();
        let messages = self.session.context_manager.lock().unwrap().get_messages();
        let mut s = self.session.client.chat_completion(messages, None, true, &self.role(task));
        // the budget is already spent when this runs, so nothing else would stop a model that has gone silent:
        // a minute without a single event ends it with whatever text arrived
        while let Ok(Some(ev)) = tokio::time::timeout(Duration::from_secs(60), s.next()).await {
            match ev {
                StreamEvent::TextDelta(c) => {
                    text.push_str(&c);
                    emit(&tx, AgentEvent::TextDelta(c));
                }
                StreamEvent::MessageComplete { usage: Some(u), .. } => self.session.context_manager.lock().unwrap().add_usage(&u),
                _ => {}
            }
        }
        self.session.context_manager.lock().unwrap().add_assistant_message((!text.is_empty()).then_some(text.as_str()), None, None, None);
        if !text.is_empty() {
            emit(&tx, AgentEvent::TextComplete(text));
        }
    }

    /// The first call after a pause is billed in full because the provider's prompt cache expired. Rewriting history
    /// costs nothing extra at that moment, so shrink it now: clear old tool output, and summarise if the context is still big.
    async fn tidy_if_cache_went_cold(&mut self, tx: &Tx) {
        let cfg = self.config.get();
        let Some(last) = self.last_model_call else { return };
        if cfg.pi_mode || last.elapsed().as_secs() < cfg.cache_ttl_seconds {
            return;
        }
        self.last_model_call = None;
        let before = self.session.context_manager.lock().unwrap().latest_usage.total_tokens;
        self.prune(true, true);
        if before as usize > cfg.idle_compact_at_tokens && self.compact(false).await {
            // the cache is cold: the trimmed copy is cheaper
            notice(tx, format!("Cache had expired: compacted {}k of context before the first call", before / 1000));
        }
    }

    /// Put the current text of the files the model was just working in back after a compaction. Read from disk now, so
    /// an edit made before the compaction shows as it is, not as it was first read.
    fn restore_files(&self, paths: &[String]) {
        const FILE_CHARS: usize = 40_000;
        const FILES_CHARS: usize = 150_000;
        let cwd = self.config.get().cwd.clone();
        let (mut parts, mut room) = (vec![], FILES_CHARS as i64);
        for rel in paths {
            let path = resolve_path(&cwd, rel);
            let Ok(text) = std::fs::read_to_string(&path) else { continue }; // deleted, a folder, or binary: the summary still names it
            if room <= 0 {
                break;
            }
            let cap = (FILE_CHARS as i64).min(room) as usize;
            let n = text.chars().count();
            let body = if n <= cap { text } else { format!("{}\n[cut at {cap} of {n} chars; read_file with offset for the rest]", text.chars().take(cap).collect::<String>()) };
            room -= body.chars().count() as i64;
            parts.push(format!("## {rel}\n{body}"));
        }
        if !parts.is_empty() {
            self.add_user(&format!("Files you were working in before the compaction, as they are on disk now (no need to read them again):\n\n{}", parts.join("\n\n")));
        }
    }

    /// Small facts that changed since the last call, as Claude Code attaches them. Appended after the history, so the
    /// cached prefix (system prompt, earlier turns) is untouched.
    fn remind_each_turn(&mut self) {
        let cwd = self.config.get().cwd.clone();
        let changed = self.session.tool_registry.changed_on_disk();
        if !changed.is_empty() {
            let names = changed.iter().take(8).map(|p| format!("`{}`", crate::util::paths::rel_display(Path::new(p.split(" (deleted)").next().unwrap_or(p)), &cwd))).collect::<Vec<_>>().join(", ");
            self.remind(&format!("Changed on disk since you last read or wrote them: {names} (by the user, a formatter or a command you ran). That is usually deliberate: take it as the current state rather than reverting it, and re-read before editing."));
        }
        if !self.plan_told && self.session.approval_manager.policy() == crate::config::ApprovalPolicy::Plan {
            // told up front, once per request: a model that learns it only from a refused edit tends to retry the edit
            self.plan_told = true;
            self.remind("Plan mode is on. You must not change anything: no edits, no new files, no commands that modify state (installs, commits, migrations). Reading, searching and read-only commands are fine. Work in this order: 1) read the code this touches and how the repo already does the nearest thing; 2) ask the user only what the code cannot answer; 3) end your turn with the plan itself: the files to change in order and what changes in each, what could break, and how to verify. Keep it short enough to scan and concrete enough to carry out. The user leaves plan mode with /plan when they approve; do not start the work before that.");
        }
        let open = self.unfinished_todos();
        if !open.is_empty() && self.todo_idle >= 10 {
            self.todo_idle = 0;
            self.remind(&format!("Your task list has not been touched in 10 rounds. Still open: {}. Tick what is done with `todos` alongside your next real call, and drop steps that no longer apply.", open.iter().take(8).cloned().collect::<Vec<_>>().join("; ")));
        }
    }

    /// Old results get cleared and history gets folded, but the task list is small and still the plan: put it back after
    /// a clear so the agent does not rediscover what is left to do.
    pub(crate) fn remind_todos(&mut self) {
        let text = self.todos().and_then(|t| t.as_any().downcast_ref::<TodosTool>().map(|t| t.render())).unwrap_or_default();
        if !text.is_empty() && (text.contains("[ ]") || text.contains("[~]")) && text != self.todos_reminded {
            self.todos_reminded = text.clone();
            // the list is only as current as the last `todos set`: say so, or a list nobody ticked reads as ten open steps
            // what was edited is the best hint for which open steps are really finished
            let mut files: Vec<String> = vec![];
            for e in self.session.tracer.snapshot().iter().filter(|e| e["event"] == "tool_call" && matches!(e["tool"].as_str(), Some("edit" | "write_file"))) {
                if let Some(t) = e["target"].as_str() {
                    if !files.iter().any(|f| f == t) {
                        files.push(t.to_string());
                    }
                }
            }
            let edited = if files.is_empty() { String::new() } else { format!("\nFiles you edited so far: {}. A step that these files already cover is done.", files.iter().take(12).cloned().collect::<Vec<_>>().join(", ")) };
            self.add_user(&format!("Task list as last set (old tool results were cleared). Steps you already finished but did not tick are still shown open: tick them with `todos` first, then carry on with the rest.{edited}\n{text}"));
        }
    }

    pub(crate) fn prune(&mut self, remind: bool, force: bool) {
        let cfg = self.config.get();
        // Clearing results early only hurts: the model re-reads what it just lost, and the rewrite throws away the
        // provider's cache. Claude Code waits until the context is genuinely filling up; so do we.
        let (used, limit) = {
            let cm = self.session.context_manager.lock().unwrap();
            (cm.latest_usage.total_tokens as f64, cm.limit())
        };
        if !force && used < (0.75 * limit).min(cfg.prune_at_tokens as f64) {
            return;
        }
        let (n, info) = {
            let mut cm = self.session.context_manager.lock().unwrap();
            let n = cm.prune_tool_outputs();
            (n, cm.last_prune.clone())
        };
        if n > 0 {
            if remind {
                self.remind_todos();
            }
            // only the files whose text was cleared are forgotten; files still in context are not sent again
            if let Some(reader) = self.session.tool_registry.get("read_file") {
                if let Some(r) = reader.as_any().downcast_ref::<ReadFileTool>() {
                    for rel in &info.read_paths {
                        r.forget(&resolve_path(&cfg.cwd, rel));
                    }
                }
            }
            // folder rules (apps/web/AGENTS.md) rode in on an old result that may be gone now: attach them again next time
            self.session.tool_registry.forget_instructions();
            self.session.tracer.emit("prune", json!({"results": info.results, "tokens_cleared": info.tokens_cleared, "prompt_tokens": info.prompt_tokens, "read_paths": info.read_paths.len()}));
        }
    }

    pub(crate) async fn compact(&mut self, reuse_cache: bool) -> bool {
        let cfg = self.config.get();
        let before = self.session.context_manager.lock().unwrap().latest_usage.total_tokens;
        self.session.hook_system.fire(HookTrigger::PreCompact, &[("tokens", before.to_string())]).await;
        // with no cheaper summarizer configured, the summary is asked of the same model on the same prefix the loop just
        // sent, so the provider serves it from cache
        let fork = (reuse_cache && !cfg.routing.contains_key("summarize")).then(|| (self.session.tool_registry.get_schemas(), self.last_role.clone()));
        let (summary, usage, cut) = self.session.chat_compactor.compress(&self.session.context_manager, cfg.keep_recent_tokens, fork).await;
        let Some(summary) = summary else { return false };
        let recent = self.session.context_manager.lock().unwrap().recent_files(cut, 5);
        self.session.tool_registry.epoch.fetch_add(1, std::sync::atomic::Ordering::SeqCst); // earlier reads are no longer in context
        self.session.tool_registry.forget_instructions(); // and neither are the folder rules that came with them
        self.playbooks_sent.clear(); // the summary does not carry playbook text: send it again next time
        let transcript = crate::config::loader::get_data_dir().join("transcripts").join(format!("{}.jsonl", self.session.session_id()));
        let notes = crate::agent::notes::read(&cfg.cwd);
        self.session.context_manager.lock().unwrap().apply_compaction(&summary, cut, Some(&transcript), Some(notes));
        self.restore_files(&recent);
        self.remind_todos();
        let after = {
            let mut cm = self.session.context_manager.lock().unwrap();
            // the summarizer's usage is the size of the summary request, not of the context now. Estimate the real size
            // (restored files included) until the next model call reports it.
            let after = cm.estimated_tokens() as i64;
            cm.set_latest_usage(TokenUsage { prompt_tokens: after, total_tokens: after, ..Default::default() });
            if let Some(u) = &usage {
                cm.add_usage(u);
            }
            after
        };
        self.session.hook_system.fire(HookTrigger::PostCompact, &[("tokens_before", before.to_string()), ("summary", summary.clone())]).await;
        self.session.tracer.emit("compaction", json!({"cut": cut, "tokens_before": before, "tokens_after": after, "summary_tokens": usage.as_ref().map(|u| u.completion_tokens)}));
        true
    }

    fn is_parallel_safe(&self, call: &ToolCall) -> bool {
        self.session.tool_registry.get(&call.name).map_or(false, |t| t.kind() == ToolKind::Read && !t.is_mutating(&Value::Object(call.arguments.clone())))
    }

    /// Run calls in model order. Runs of read-only calls (e.g. five greps in a row) execute concurrently; anything that
    /// writes or shells out acts as a barrier and runs alone, so edits never race with reads.
    async fn run_tool_calls(&mut self, calls: &[ToolCall], tx: &Tx, out: &mut Vec<(String, String, bool)>) {
        crate::util::search::hold_listing(true);
        let mut i = 0;
        while i < calls.len() {
            let mut j = i + 1;
            if self.is_parallel_safe(&calls[i]) {
                while j < calls.len() && self.is_parallel_safe(&calls[j]) {
                    j += 1;
                }
            }
            let batch = &calls[i..j];
            i = j;
            let tokens = self.spent();
            for tc in batch {
                emit(tx, AgentEvent::ToolCallStart { call_id: tc.call_id.clone(), name: tc.name.clone(), arguments: tc.arguments.clone(), tokens });
                self.session.loop_detector.lock().unwrap().record_tool_call(&tc.name, &tc.arguments);
            }
            let cwd = self.config.get().cwd.clone();
            let session = self.session.clone();
            let futs = batch.iter().map(|tc| {
                let (session, cwd) = (session.clone(), cwd.clone());
                // already running since mid-reply, if it is still the same call
                let started = self.early.remove(&tc.call_id).and_then(|(call, run)| if call.name == tc.name && call.arguments == tc.arguments { Some(run) } else { run.abort(); None });
                async move {
                    if let Some(Ok(done)) = match started { Some(run) => Some(run.await), None => None } {
                        return done;
                    }
                    session.tool_registry.invoke(&tc.name, Value::Object(tc.arguments.clone()), &cwd, &session.hook_system, Some(&session.approval_manager), &tc.call_id).await
                }
            });
            let results: Vec<ToolResult> = futures::future::join_all(futs).await;
            // a pass from before an edit proves nothing about the code as it is now
            let edits = batch.iter().any(|tc| tc.name == "edit" || tc.name == "write_file");
            if edits {
                self.green_checks.clear();
            }
            for (tc, result) in batch.iter().zip(results) {
                let cmd = tc.arguments.get("command").and_then(|c| c.as_str()).unwrap_or("").trim().to_string();
                if result.flag("no_approver") {
                    // remembered as the user would type it: "`npm run db:migrate`" or "write_file db/schema.sql"
                    let what = if cmd.is_empty() { format!("{} {}", tc.name, tc.arguments.get("path").and_then(|p| p.as_str()).unwrap_or("")) } else { format!("`{}`", crate::util::text::head_chars(&cmd, 120)) };
                    self.unasked.push(what.trim().to_string());
                }
                let mut result = result;
                if let Some(note) = unchecked_by_eslint(&cmd) {
                    result.output.push_str(&note); // or "eslint passed" gets reported as if globals.css was checked
                }
                if tc.name == "shell" && check_command().is_match(&cmd) {
                    // ponytail: matched on the exact command text, so a failing `npm test -- -t foo` is not cleared by a
                    // later plain `npm test`. That costs one extra nudge at most; compare by tool and target if it gets noisy.
                    self.red_checks.retain(|c| c != &cmd);
                    self.green_checks.retain(|c| c != &cmd);
                    if !result.success {
                        self.red_checks.push(cmd);
                    } else if !edits {
                        self.green_checks.push(cmd);
                    }
                } else if tc.name == "shell" && result.success && !edits && proof_command().is_match(&cmd) && !self.green_checks.contains(&cmd) {
                    self.green_checks.push(cmd);
                }
                out.push((tc.call_id.clone(), result.to_model_output(), !result.success));
                emit(tx, AgentEvent::ToolCallComplete { call_id: tc.call_id.clone(), name: tc.name.clone(), result });
            }
        }
        crate::util::search::hold_listing(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn patterns() {
        assert!(text_call().is_match("<tool_call>{\"name\": \"x\"}"));
        assert!(text_call().is_match("```json\n{\"name\": \"read_file\""));
        assert!(!text_call().is_match("plain answer"));
        assert!(overflow().is_match("This model's maximum context length is 8192 tokens"));
        assert!(not_overflow().is_match("rate limit: too many tokens per minute"));
    }

    #[test]
    fn eslint_handed_css_says_it_did_not_check_it() {
        let note = unchecked_by_eslint("pnpm exec eslint --quiet app/page.tsx app/globals.css content/work.ts").unwrap();
        assert!(note.contains("app/globals.css") && !note.contains("page.tsx"), "{note}");
        assert!(unchecked_by_eslint("pnpm exec eslint --quiet app/page.tsx").is_none());
        assert!(unchecked_by_eslint("cat app/globals.css").is_none());
    }
}

/// True when the reply ends by asking the user something, e.g. "Which scope do you want?".
/// Such a turn is waiting on the user, so "no file changed" is expected, not a miss.
/// True when the reply ends by asking leave to continue, e.g. "Say the word and I'll measure it."
fn seeks_permission(reply: &str) -> bool {
    let tail: String = reply.trim_end().chars().rev().take(300).collect::<Vec<_>>().into_iter().rev().collect::<String>().to_lowercase();
    ["say the word", "should i ", "shall i ", "want me to", "would you like me", "tell me if you'd rather", "tell me if you would rather", "let me know if you want me"].iter().any(|p| tail.contains(p))
}

fn asks_user(reply: &str) -> bool {
    let tail: String = reply.trim_end().chars().rev().take(400).collect::<Vec<_>>().into_iter().rev().collect();
    tail.contains('?')
}

#[cfg(test)]
mod asks_user_tests {
    use super::{asks_user, seeks_permission};
    #[test]
    fn permission_phrases_are_caught_only_at_the_end() {
        assert!(seeks_permission("Width is off.\n\n**Next:** Say the word and I'll measure it."));
        assert!(seeks_permission("Done with A. Should I also do B?"));
        assert!(!seeks_permission("Fixed the width. Table is 896px, matches the original."));
        assert!(!seeks_permission(&format!("You said say the word once. {}", "x".repeat(400))));
    }
    #[test]
    fn question_tail_counts() {
        assert!(asks_user("Found the schema.\n\nWhich scope do you want first?"));
        assert!(!asks_user("Here is the code:\n```rust\nfn main() {}\n```"));
    }
}
