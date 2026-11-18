//! Pick a model tier for a request without spending a model call.
//!   fast    - small, well-defined work (rename, typo, where is X, explain this)
//!   default - the configured main model
//!   strong  - debugging, concurrency, security, architecture, multi-file work
//! The classifier is deliberately simple and explainable (it returns its reason), and the agent escalates to `strong`
//! on evidence of trouble (a failed repair, a detected loop) rather than guessing up front.
use crate::util::re::{lit, lit_i, Re};
use std::sync::OnceLock;

fn strong() -> &'static Re {
    static R: OnceLock<Re> = OnceLock::new();
    R.get_or_init(|| lit_i(r"\b(debug\w*|root cause|race condition|concurren\w*|deadlock|refactor\w*|architect\w*|migrat\w*|security|vulnerab\w*|auth\w*|payment\w*|webhook\w*|performance|optimi[sz]\w*|intermittent|flaky|regression|why (does|is|are|do)|memory leak|transaction\w*|idempoten\w*|design|investigate)\b"))
}
fn fast() -> &'static Re {
    static R: OnceLock<Re> = OnceLock::new();
    R.get_or_init(|| lit_i(r"\b(rename|typo|comment|format\w*|lint|where (is|are)|find|list|show|what (does|is)|explain|read|open|print|bump|add (a )?(log|import)|remove (the )?(unused|log))\b"))
}
fn pathy() -> &'static Re {
    static R: OnceLock<Re> = OnceLock::new();
    R.get_or_init(|| lit(r"[\w.\-]+/[\w./\-]+|\b[\w\-]+\.(?:ts|tsx|js|jsx|py|go|rs|java|sql|prisma)\b"))
}
pub fn reviewy() -> &'static Re {
    static R: OnceLock<Re> = OnceLock::new();
    R.get_or_init(|| lit_i(r"\b(review|audit|critique)\b"))
}
/// Does the person's own text point at code or changes to check? The review pipeline needs this as well as the orient
/// model's "review": that model once filed "how can we make the content better so people read the site" as a review,
/// and the person got "Code review: No issues found" instead of an answer.
pub fn about_code_review(message: &str) -> bool {
    static R: OnceLock<Re> = OnceLock::new();
    R.get_or_init(|| lit_i(r"\b(review|audit|critique|diff|pr|pull request|changes?|commits?|code|bugs?|look over|check (?:it|this|my|the))\b")).is_match(typed(message))
}
/// "how do i make this better", "should we split it?": the person asked for advice, so an answer that proposes a step
/// and asks before taking it is the right answer, not one to send back with "just do it".
pub fn asks_how(message: &str) -> bool {
    static R: OnceLock<Re> = OnceLock::new();
    let t = typed(message).trim();
    t.ends_with('?') || R.get_or_init(|| lit_i(r"^(?:so\s+|well\s+|ok\s+)?(?:how|what|why|which|should|would|could|can|is|are|do|does)\b")).is_match(t)
}
/// Questions riding along with a work request, which the final answer has to cover. A paste, then "update the site and
/// write the post\n\nalso tell me how can i make my site better for recruiters" got the edits and no word on the
/// recruiters, three times in a row. Only the person's own short blocks after a paste are read, and a request that is
/// one question as a whole returns nothing (it is answered anyway).
pub fn side_questions(message: &str) -> Vec<String> {
    static Q: OnceLock<Re> = OnceLock::new();
    let q = Q.get_or_init(|| lit_i(r"^(?:and\s+|so\s+)?(?:tell me|how|what|why|which|should|could you tell|can you tell|explain|suggest|give me (?:ideas|advice|tips))\b"));
    let blocks: Vec<&str> = message.split("\n\n").map(str::trim).filter(|b| !b.is_empty()).collect();
    let own: Vec<&str> = if message.chars().count() < 600 { blocks } else {
        let mut tail: Vec<&str> = blocks.into_iter().rev().take_while(|b| b.chars().count() < 600).collect();
        tail.reverse();
        tail
    };
    // "write the post also tell me how ..." on one line is two asks, so "also" starts a new part too
    let mut parts: Vec<String> = vec![];
    for line in own.iter().flat_map(|b| b.lines()) {
        let low = line.to_ascii_lowercase();
        let mut at = 0;
        for (i, _) in low.match_indices("also ").filter(|(i, _)| *i == 0 || low.as_bytes()[i - 1] == b' ') {
            parts.push(line[at..i].to_string());
            at = i + 5;
        }
        parts.push(line[at..].to_string());
    }
    let parts: Vec<String> = parts.into_iter().map(|p| p.trim().trim_end_matches(|c| ",.;".contains(c)).to_string()).filter(|p| p.len() > 3).collect();
    if parts.len() < 2 {
        return vec![];
    }
    parts.into_iter().filter(|p| p.ends_with('?') || q.is_match(p)).collect()
}
/// What the user actually typed. `@spec.md` inlines the whole file after the text, so every keyword check (review? fix?
/// hard task? one agent?) must read this and not the full message: a spec that says "review" somewhere must not turn
/// "implement this plan" into a review run.
pub fn typed(message: &str) -> &str {
    let msg = ["\n\n<file path=\"", "\n\n<directory path=\""].iter().filter_map(|m| message.find(m)).min().map_or(message, |i| &message[..i]);
    // A big paste has no marker, but the person's own words are one short block next to it: "review this:" before the
    // paste, or "make a plan to build this" after it. Judging the whole text lets "we review submissions" inside a
    // pasted job post start a code review. Short first block means ask-then-paste; otherwise the ask is the last block.
    let blocks: Vec<&str> = msg.split("\n\n").map(str::trim).filter(|b| !b.is_empty()).collect();
    if msg.chars().count() < 600 || blocks.len() < 2 {
        return msg;
    }
    let (first, last) = (blocks[0], blocks[blocks.len() - 1]);
    if first.chars().count() < 200 { first } else { last }
}
pub fn is_reviewy(message: &str) -> bool {
    reviewy().is_match(typed(message))
}
/// "review it with one single agent", "a single reviewer", "without the pipeline": the main agent reviews it itself.
pub fn one_agent() -> &'static Re {
    static R: OnceLock<Re> = OnceLock::new();
    R.get_or_init(|| lit_i(r"\b(?:(?:one|1|a)\s+)?single[- ](?:agent|reviewer)s?\b|\b(?:one|1)\s+(?:agent|reviewer)\b|\b(?:no|without)\s+(?:the\s+)?(?:sub-?agents?|pipeline|fan-?out|parallel reviewers)\b"))
}
fn explainy() -> &'static Re {
    static R: OnceLock<Re> = OnceLock::new();
    R.get_or_init(|| lit_i(r"\b(explain|walk me through|how does|what does|summari[sz]e)\b"))
}

/// (tier, reason)
pub fn classify(request: &str) -> (&'static str, String) {
    let text = request.trim();
    if let Some((s, e)) = strong().find(text) {
        return ("strong", format!("'{}' suggests a hard task", text[s..e].to_lowercase()));
    }
    let n = text.chars().count();
    if n > 700 {
        return ("strong", "long, detailed request".into());
    }
    let mut paths = 0;
    let mut at = 0;
    while let Some((_, e)) = pathy().find(&text[at..]) {
        paths += 1;
        at += e.max(1);
    }
    if paths >= 4 {
        return ("strong", "touches several files".into());
    }
    if n < 200 && fast().is_match(text) && !lit_i(r"(?s)\band\b.*\band\b").is_match(text) {
        return ("fast", "small, well-defined request".into());
    }
    ("default", "ordinary task".into())
}

/// A path where a mistake is expensive: auth, money, secrets, permissions, schema migrations. Matched on whole path
/// pieces so "policy" in routing_policy.rs or "author" in author.ts doesn't count.
fn risky_path() -> &'static Re {
    static R: OnceLock<Re> = OnceLock::new();
    R.get_or_init(|| lit_i(r"(?:^|[/_.\-])(?:auth|authn|authz|authentication|authorization|oauth|login|passwords?|secrets?|credentials?|payments?|billing|checkout|stripe|migrations?|migrate|permissions?|rbac|acl|iam)(?:[/_.\-]|$)"))
}
/// Tests and docs can't break production, so they don't make a change risky.
fn test_or_doc() -> &'static Re {
    static R: OnceLock<Re> = OnceLock::new();
    R.get_or_init(|| lit_i(r"(?:^|/)(?:tests?|__tests__|specs?|docs?)/|(?:^|/)test_|_test\.|\.(?:test|spec)\.|\.md$"))
}

/// The first changed file on a high-risk path, with the word that flagged it: `src/auth/session.ts` gives
/// `("src/auth/session.ts", "auth")`. Such a change gets an independent review even when `auto_review` is off.
pub fn risky_change(files: &[String]) -> Option<(String, String)> {
    files.iter().filter(|f| !test_or_doc().is_match(f)).find_map(|f| {
        let (s, e) = risky_path().find(f)?;
        Some((f.clone(), f[s..e].trim_matches(|c| "/_.-".contains(c)).to_lowercase()))
    })
}

/// Tokens of context worth keeping cached.
const CACHE_WORTH_KEEPING: i64 = 20_000;

/// Switching model resets the provider's prompt cache, so the next call re-bills the whole context at full price. Past
/// CACHE_WORTH_KEEPING a request stays on the model in use; only evidence of trouble (a failed repair, a loop) escalates.
pub fn keep_cached_model(tier: &'static str, context_tokens: i64) -> &'static str {
    if tier != "default" && context_tokens >= CACHE_WORTH_KEEPING { "default" } else { tier }
}

/// fast work needs little thinking, hard work gets the most; "default" leaves the model's own setting alone.
pub fn effort(tier: &str) -> Option<&'static str> {
    match tier {
        "fast" => Some("low"),
        "strong" => Some("high"),
        _ => None,
    }
}

/// One line appended to the user's message telling the model how long the answer should be.
pub fn length_hint(request: &str, tier: &str, will_change: bool, reviewing: bool) -> String {
    if reviewing {
        return "Answer with at most 6 findings, each `severity path:line  what is wrong -> fix`; if there is no defect, only: No issues found.".into();
    }
    if explainy().is_match(request) && !will_change {
        return "Answer in under 300 words, with file:line references.".into();
    }
    if tier == "fast" && will_change {
        return "When done, reply in 1-2 sentences: what changed.".into();
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_question_inside_a_work_request_is_kept() {
        let paste = "# Speculative RAG\n\n".to_string() + &"The README body. ".repeat(80);
        let msg = format!("{paste}\n\nthis is the readme of speculative rag we have update the things over the places pls also if there has been no blog of this write that up too\n\nalso tell me how can i make my site more better for the recruiter to actuall get it");
        assert_eq!(side_questions(&msg), ["tell me how can i make my site more better for the recruiter to actuall get it"]);
        assert!(side_questions("how do i make this better").is_empty()); // the whole request is the question
        assert!(side_questions("make the status line smaller and commit").is_empty());
    }

    #[test]
    fn content_advice_is_not_a_code_review() {
        assert!(!about_code_review("well i am asking about the content how can we make it better so people genuinely read the site"));
        assert!(asks_how("how do i make this better like people actually look at me") && asks_how("split it?"));
        assert!(!asks_how("add a resume link to the hero") && !asks_how("make the status line smaller"));
        assert!(about_code_review("review the change i have") && about_code_review("look over my diff") && about_code_review("is this PR ok"));
    }
    #[test]
    fn tiers() {
        assert_eq!(classify("debug the flaky test").0, "strong");
        assert_eq!(classify("rename foo to bar").0, "fast");
        assert_eq!(classify("add a button to the page").0, "default");
        assert_eq!(keep_cached_model("strong", 30_000), "default");
        assert!(one_agent().is_match("review it with one single agent"));
    }

    #[test]
    fn risky_paths() {
        let risky = |f: &str| risky_change(&[f.to_string()]).map(|(_, why)| why);
        assert_eq!(risky("src/auth/session.ts").as_deref(), Some("auth"));
        assert_eq!(risky("db/migrations/0042_users.sql").as_deref(), Some("migrations"));
        assert_eq!(risky("app/payment_service.py").as_deref(), Some("payment"));
        assert_eq!(risky("config/secrets.yaml").as_deref(), Some("secrets"));
        // a word inside a longer name, or a test or doc about the risky thing, is not risky
        assert_eq!(risky("src/agent/routing_policy.rs"), None);
        assert_eq!(risky("src/author.ts"), None);
        assert_eq!(risky("src/william.py"), None);
        assert_eq!(risky("tests/test_auth.py"), None);
        assert_eq!(risky("src/auth/login.test.ts"), None);
        assert_eq!(risky("docs/auth.md"), None);
        // the first risky file is the one named, whatever comes before it
        let files = vec!["README.md".to_string(), "src/billing/invoice.rs".to_string()];
        assert_eq!(risky_change(&files), Some(("src/billing/invoice.rs".into(), "billing".into())));
    }
}
