//! The review half of an unattended receipt. A run with nobody watching still deserves a second pair of eyes on a risky
//! diff, and the receipt must say what that review found (or why there was none), never just a blank.
//!
//! Example: the agent adds `POST /bookings/rsvp` in `routes/v1/booking.route.ts` above the auth middleware. The route
//! file is risky, so an independent reviewer reads the diff and the receipt gets
//! `{"status": "ran", "verdict": "ISSUES", "findings": [{"severity": "unrated", "file": "booking.route.ts", "line": 31, "summary": "..."}]}`.
use crate::agent::review::review_changes;
use crate::agent::routing_policy::risky_change;
use crate::config::Config;
use serde_json::{json, Value};
use std::path::Path;
use std::time::Duration;

/// Many files, or many lines, is risky on its own: nobody reads 500 changed lines as carefully as 20.
const BIG_FILES: usize = 8;
const BIG_LINES: usize = 300;

fn is_test_or_doc(f: &str) -> bool {
    f.ends_with(".md") || f.contains(".test.") || f.contains(".spec.") || f.contains("/test/") || f.contains("/tests/") || f.contains("__tests__") || f.starts_with("test_")
}

/// Why this diff is worth an independent review, or None when it is small and boring.
/// `src/auth/session.ts` -> "auth"; `routes/v1/booking.route.ts` -> "routes"; a 14 file change -> "large change".
// ponytail: path words and size only; reading the diff to see "mounted before the auth middleware" would need a parser
pub fn risk(files: &[String], changed_lines: usize) -> Option<String> {
    if let Some((file, why)) = risky_change(files) {
        return Some(format!("{file} touches {why}"));
    }
    for f in files.iter().filter(|f| !is_test_or_doc(f)) {
        let hit = f.split(|c: char| !c.is_ascii_alphanumeric()).map(str::to_lowercase).find(|w| matches!(w.as_str(), "middleware" | "middlewares" | "route" | "routes" | "router" | "guard" | "guards" | "webhook" | "webhooks"));
        if let Some(w) = hit {
            return Some(format!("{f} is {w} code, where a public or unguarded endpoint can slip in"));
        }
    }
    if files.len() >= BIG_FILES || changed_lines >= BIG_LINES {
        return Some(format!("large change ({} files, {} lines)", files.len(), changed_lines));
    }
    None
}

/// `1. apps/x/a.ts:31 what is wrong | evidence: ...` becomes {severity, file, line, summary}. A leading HIGH or MEDIUM word
/// is kept as the severity; the reviewer's own format has none, so most rows are "unrated".
fn parse_findings(text: &str) -> Vec<Value> {
    text.lines()
        .filter_map(|l| {
            let l = l.trim();
            let (num, rest) = l.split_once(|c| c == '.' || c == ')')?;
            if num.is_empty() || !num.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            let mut rest = rest.trim();
            let mut severity = "unrated".to_string();
            if let Some((w, tail)) = rest.split_once(char::is_whitespace) {
                if matches!(w.trim_matches(|c| c == '[' || c == ']' || c == ':').to_uppercase().as_str(), "CRITICAL" | "HIGH" | "MEDIUM" | "LOW") {
                    severity = w.trim_matches(|c| c == '[' || c == ']' || c == ':').to_lowercase();
                    rest = tail.trim();
                }
            }
            let (loc, summary) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
            let summary = summary.split(" | evidence:").next().unwrap_or("").trim();
            let (file, line) = match loc.rsplit_once(':') {
                Some((f, n)) if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) => (f, n.parse::<u64>().ok()),
                _ => (loc, None),
            };
            Some(json!({"severity": severity, "file": file, "line": line, "summary": summary}))
        })
        .collect()
}

/// Turns a reviewer answer into the receipt shape. UNPARSED means we got text but no VERDICT line: say so, don't guess.
pub fn shape(verdict: &str, findings: &str) -> Value {
    match verdict {
        "PASS" => json!({"status": "ran", "verdict": "PASS", "findings": []}),
        "ISSUES" => json!({"status": "ran", "verdict": "ISSUES", "findings": parse_findings(findings), "raw": findings.chars().take(1500).collect::<String>()}),
        _ => json!({"status": "unparsed", "reason": if findings.trim().is_empty() { "the reviewer gave no VERDICT line".to_string() } else { findings.chars().take(300).collect::<String>() }}),
    }
}

/// Is this review result one that lets a run count as complete? Only a review that ran (or was not needed) does.
pub fn lets_complete(review: &Value) -> bool {
    matches!(review["status"].as_str(), Some("ran" | "not_needed"))
}

/// Added plus deleted lines of the change. Tracked files come from `git diff --numstat`, new files are counted on disk.
fn changed_lines(cwd: &Path, files: &[String]) -> usize {
    let out = std::process::Command::new("git").args(["diff", "--numstat", "HEAD"]).current_dir(cwd).output().ok().filter(|o| o.status.success());
    let text = out.map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
    let mut counted = std::collections::HashMap::new();
    for l in text.lines() {
        let mut p = l.split('\t');
        let (a, d, f) = (p.next().unwrap_or("0"), p.next().unwrap_or("0"), p.next().unwrap_or(""));
        counted.insert(f.to_string(), a.parse::<usize>().unwrap_or(0) + d.parse::<usize>().unwrap_or(0));
    }
    files.iter().map(|f| counted.get(f).copied().unwrap_or_else(|| std::fs::read_to_string(cwd.join(f)).map(|t| t.lines().count()).unwrap_or(0))).sum()
}

/// Files the run changed, from the agent's report when it has one, otherwise from git (a run cut off by the clock has no report).
fn changed_files(cwd: &Path, report: Option<&Value>) -> Vec<String> {
    if let Some(a) = report.and_then(|r| r["changed_files"].as_array()) {
        return a.iter().filter_map(|v| v.as_str().map(String::from)).collect();
    }
    let out = std::process::Command::new("git").args(["status", "--porcelain", "-uall"]).current_dir(cwd).output().ok();
    out.map(|o| String::from_utf8_lossy(&o.stdout).lines().filter_map(|l| l.get(3..).map(|p| p.trim().to_string())).collect()).unwrap_or_default()
}

/// The `review` value for the receipt. `skip` is a reason the review must not run (task turned it off, a cap already hit).
/// Never returns null: the answer is always one of ran, unparsed, not_run or not_needed.
pub async fn for_receipt(config: &Config, issue: &str, report: Option<&Value>, skip: Option<&str>) -> Value {
    let files = changed_files(&config.cwd, report);
    if files.is_empty() {
        return json!({"status": "not_needed", "reason": "no files changed"});
    }
    // the agent's own loop may already have had the diff reviewed: keep that result instead of paying for a second one
    if let Some(v) = report.and_then(|r| r["review"]["verdict"].as_str()) {
        return shape(v, report.and_then(|r| r["review"]["findings"].as_str()).unwrap_or(""));
    }
    let lines = changed_lines(&config.cwd, &files);
    let Some(why) = risk(&files, lines) else {
        return json!({"status": "not_needed", "reason": format!("small change outside auth, payments, routes and middleware ({} files, {} lines)", files.len(), lines)});
    };
    if let Some(reason) = skip {
        return json!({"status": "not_run", "reason": format!("{reason} (this change is risky: {why})")});
    }
    let checks: Vec<Value> = report.and_then(|r| r["checks"].as_array()).cloned().unwrap_or_default();
    let claims = report.and_then(|r| r["response"].as_str()).unwrap_or("");
    let run = review_changes(config, issue, &files, &checks, claims, None, None, "review");
    match tokio::time::timeout(Duration::from_secs(360), run).await {
        Ok(res) => {
            let mut v = shape(&res.verdict, &res.findings);
            v["why"] = json!(why);
            v
        }
        Err(_) => json!({"status": "not_run", "reason": format!("the review timed out (this change is risky: {why})")}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn risk_flags_auth_routes_middleware_and_size() {
        assert!(risk(&s(&["apps/server/src/auth/session.ts"]), 10).unwrap().contains("auth"));
        assert!(risk(&s(&["apps/server/src/routes/v1/booking.route.ts"]), 10).unwrap().contains("route"));
        assert!(risk(&s(&["apps/server/src/middleware/rate-limit.middleware.ts"]), 10).unwrap().contains("middleware"));
        assert!(risk(&s(&["a.ts"]), 400).unwrap().contains("large change"));
        let many: Vec<String> = (0..8).map(|i| format!("src/f{i}.ts")).collect();
        assert!(risk(&many, 10).unwrap().contains("8 files"));
    }

    #[test]
    fn risk_ignores_small_plain_changes_tests_and_docs() {
        assert_eq!(risk(&s(&["src/util/format.ts"]), 20), None);
        assert_eq!(risk(&s(&["docs/routes.md", "src/routes/a.test.ts"]), 20), None, "tests and docs never make a change risky");
        assert_eq!(risk(&s(&["src/author.ts", "src/routing_policy.rs"]), 20), None, "whole words only: 'author' is not 'auth'");
    }

    #[test]
    fn issues_are_parsed_into_rows() {
        let v = shape("ISSUES", "1. apps/x/a.ts:31 the route is public | evidence: mounted above authMiddleware\n2. HIGH b.ts no limit on size | evidence: none");
        assert_eq!(v["status"], "ran");
        assert_eq!(v["findings"][0]["file"], "apps/x/a.ts");
        assert_eq!(v["findings"][0]["line"], 31);
        assert_eq!(v["findings"][0]["severity"], "unrated");
        assert_eq!(v["findings"][0]["summary"], "the route is public");
        assert_eq!(v["findings"][1]["severity"], "high");
        assert_eq!(shape("PASS", "")["findings"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn an_unparsed_review_says_so_and_blocks_complete() {
        let v = shape("UNPARSED", "reviewer produced no answer");
        assert_eq!(v["status"], "unparsed");
        assert_eq!(v["reason"], "reviewer produced no answer");
        assert!(!lets_complete(&v));
        assert!(!lets_complete(&json!({"status": "not_run", "reason": "x"})));
        assert!(lets_complete(&shape("PASS", "")));
        assert!(lets_complete(&json!({"status": "not_needed", "reason": "x"})));
        assert_eq!(shape("UNPARSED", "")["reason"], "the reviewer gave no VERDICT line");
    }
}
