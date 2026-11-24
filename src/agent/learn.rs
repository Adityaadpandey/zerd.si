//! What zerd learns about a repo between sessions, kept outside the repo (data dir). Only things that cannot be
//! hallucinated go in: lessons (what YOU told it) and edit history (request words -> the files a verified run changed).
//! Nothing the model merely claims is stored.
use crate::client::llm_client::LLMClient;
use crate::client::response::StreamEvent;
use crate::config::loader::get_data_dir;
use crate::util::paths::{atomic_write, resolve};
use crate::util::rank::tokenize;
use futures::StreamExt;
use serde_json::{json, Value};
use sha1::{Digest, Sha1};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const MAX_LESSONS: usize = 30;
const MAX_EDITS: usize = 200;
const PROMPT_CHARS: usize = 1500;
/// Share of the search query's words a past request must have in common.
const MIN_OVERLAP: f64 = 0.5;
pub const HEADING: &str = "## Learned by zerd";
const JUDGE_PROMPT: &str = "You watch a coding agent work with its user. Decide whether the user's NEW message teaches the agent something worth remembering for this repo.

Worth keeping:
- the user redirects or corrects the agent (\"no, not that\", \"I wanted X\", changing their mind, giving something other than what the agent did)
- the user confirms an approach was right (\"yes exactly like that\", \"perfect, keep doing it this way\")
- the user states a preference or convention, however it is phrased

Not worth keeping: a new unrelated task, a question, a one-off detail that will not matter next time, thanks, small talk.

Reply with ONE short rule in the imperative that would help next time, or nothing at all. Say what to do and, when the correction shows it, what not to do. Example:
  agent built a dark mode toggle, user: \"no, follow the system theme instead\" -> Dark mode follows the system theme; do not add a manual toggle.
Reply with only the rule, or the single word NONE.";

fn squash(s: &str, n: usize) -> String {
    crate::util::text::head_chars(&crate::util::text::squash(s), n)
}

/// Does the file already carry this rule, even if the judge worded it differently this time?
///
/// The judge paraphrases: "Continue the loop by reading X" and "Read X and continue the loop" are one rule, not
/// two. Comparing the words both have in common, and only calling it the same rule when nearly all of the new
/// rule's own words are already there, keeps AGENTS.md a list of rules instead of a list of restatements.
fn has_rule(body: &str, text: &str) -> bool {
    body.lines().filter(|l| l.trim_start().starts_with("- ")).any(|line| same_rule(line.trim_start().trim_start_matches("- "), text))
}

/// Are these two wordings of the same rule?
///
/// A paraphrase keeps the original's distinctive words and only rearranges the filler, so it agrees with the
/// original in both directions. Two rules that merely share common words ("this lesson is noise and goes" and
/// "this lesson is fine and stays") agree in neither direction: each has a word the other does not, and those
/// are the words that carry the meaning.
fn same_rule(have: &str, text: &str) -> bool {
    let words = |s: &str| -> Vec<String> {
        s.split(|c: char| !c.is_alphanumeric())
            .filter(|w| w.len() > 2)
            .map(|w| w.to_lowercase())
            .collect()
    };
    let new_words = words(text);
    let have_words = words(have);
    if new_words.is_empty() || have_words.is_empty() {
        return false;
    }
    let shared = new_words.iter().filter(|w| have_words.contains(w)).count();
    let back = have_words.iter().filter(|w| new_words.contains(w)).count();
    // the shorter rule must be mostly in the longer one: a paraphrase may be longer or shorter than the rule it
    // restates, so the two sides are not compared symmetrically. Two different rules share only the filler, so
    // even the shorter one is not mostly there.
    let (short, long) = if new_words.len() <= have_words.len() { (shared, new_words.len()) } else { (back, have_words.len()) };
    if short * 3 < long * 2 {
        return false;
    }
    // What tells a paraphrase from a different rule is which words differ. A paraphrase drops and adds ordinary
    // words ("reading", "completing") while keeping the ones that carry the meaning ("loop", "Progress", "DONE"),
    // so the words only one side has must be few. Two different rules differ in the meaningful words: "noise and
    // goes" against "fine and stays" keeps only the filler in common.
    let only_new = new_words.iter().filter(|w| !have_words.contains(w)).count();
    let only_have = have_words.iter().filter(|w| !new_words.contains(w)).count();
    only_new * 5 <= new_words.len() * 2 && only_have * 5 <= have_words.len() * 2
}

/// Ask a model whether `message` (the user's reply) is a correction, a confirmation or a preference worth keeping.
/// Returns the rule to save, or "" when there is nothing to learn. Never fails: learning must not hurt a run.
pub async fn judge(client: &Arc<LLMClient>, before: &str, answer: &str, message: &str) -> String {
    let ask = format!("The user asked: {}\nThe agent answered: {}\nThe user's NEW message: {}", squash(before, 400), squash(answer, 600), squash(message, 400));
    let mut out = String::new();
    let mut s = client.chat_completion(vec![json!({"role": "system", "content": JUDGE_PROMPT}), json!({"role": "user", "content": ask})], None, false, "summarize");
    while let Some(ev) = s.next().await {
        match ev {
            StreamEvent::MessageComplete { text: Some(t), .. } => out.push_str(&t),
            StreamEvent::Error(_) => return String::new(),
            _ => {}
        }
    }
    let rule = crate::util::text::squash(&out);
    let rule = rule.trim_matches(|c| c == '"' || c == '\'' || c == '`' || c == ' ').to_string();
    if rule.is_empty() || rule.to_uppercase().starts_with("NONE") || rule.chars().count() > 300 { String::new() } else { rule }
}

pub struct Learned {
    root: PathBuf,
    path: PathBuf,
    data: Value,
}

impl Learned {
    pub fn new(root: &Path) -> Learned {
        Self::with_base(root, &get_data_dir())
    }

    pub fn with_base(root: &Path, base: &Path) -> Learned {
        let root = resolve(root);
        let h = hex::encode(Sha1::digest(root.to_string_lossy().as_bytes()));
        let path = base.join("learned").join(format!("{}.json", &h[..16]));
        let mut data = json!({"lessons": [], "edits": []});
        if let Some(loaded) = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
            for k in ["lessons", "edits"] {
                data[k] = loaded.get(k).filter(|v| v.is_array()).cloned().unwrap_or(json!([]));
            }
        }
        Learned { root, path, data }
    }

    fn save(&self) {
        if let Some(d) = self.path.parent() {
            if std::fs::create_dir_all(d).is_ok() {
                let _ = atomic_write(&self.path, &serde_json::to_string_pretty(&self.data).unwrap_or_default()); // learning is a bonus: never fail a run over it
            }
        }
    }

    pub fn add_lesson(&mut self, text: &str, source: &str) -> bool {
        let text: String = crate::util::text::squash(text).chars().take(300).collect();
        let exists = self.data["lessons"].as_array().unwrap().iter().any(|l| l["text"].as_str().unwrap_or("").to_lowercase() == text.to_lowercase());
        if text.split_whitespace().count() < 4 || exists {
            return false;
        }
        // the store also holds rules the judge worded differently, so a paraphrase of one already learned is not a
        // new lesson: without this, every session that learns the same kind of rule adds another copy of it
        if self.data["lessons"].as_array().unwrap().iter().any(|l| same_rule(l["text"].as_str().unwrap_or(""), &text)) {
            return false;
        }
        let in_file = self.to_agents_md(&text);
        let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let lessons = self.data["lessons"].as_array_mut().unwrap();
        lessons.push(json!({"text": text, "source": source, "ts": ts, "in_file": in_file}));
        if lessons.len() > MAX_LESSONS {
            let drop = lessons.len() - MAX_LESSONS;
            lessons.drain(..drop);
        }
        self.save();
        true
    }

    /// Also write the lesson into the repo's AGENTS.md under '## Learned by zerd', so the team gets it too and the next
    /// session reads it as an ordinary instruction. False when the file can't be written.
    fn to_agents_md(&self, text: &str) -> bool {
        let f = self.root.join("AGENTS.md");
        let mut body = if f.is_file() { std::fs::read_to_string(&f).unwrap_or_default() } else { String::new() };
        if !body.contains(HEADING) {
            body = format!("{}{HEADING}\n", if body.trim().is_empty() { String::new() } else { format!("{}\n\n", body.trim_end_matches('\n')) });
        }
        // A rule the judge words slightly differently each time is the same rule: without this, AGENTS.md fills
        // with paraphrases of one instruction and every other rule in it gets harder to see.
        if has_rule(&body, text) {
            return false;
        }
        std::fs::write(&f, format!("{}\n- {text}\n", body.trim_end_matches('\n'))).is_ok()
    }

    fn out_of_agents_md(&self, text: &str) {
        let f = self.root.join("AGENTS.md");
        if let Ok(body) = std::fs::read_to_string(&f) {
            let _ = std::fs::write(&f, body.replacen(&format!("- {text}\n"), "", 1));
        }
    }

    pub fn forget(&mut self, index: usize) -> Option<String> {
        let lessons = self.data["lessons"].as_array_mut().unwrap();
        if index >= 1 && index <= lessons.len() {
            let gone = lessons.remove(index - 1);
            let text = gone["text"].as_str().unwrap_or("").to_string();
            if gone["in_file"].as_bool().unwrap_or(false) {
                self.out_of_agents_md(&text);
            }
            self.save();
            return Some(text);
        }
        None
    }

    pub fn edit_count(&self) -> usize {
        self.data["edits"].as_array().map_or(0, Vec::len)
    }

    pub fn store_path(&self) -> &Path {
        &self.path
    }

    pub fn lessons(&self) -> Vec<String> {
        self.data["lessons"].as_array().unwrap().iter().map(|l| l["text"].as_str().unwrap_or("").to_string()).collect()
    }

    pub fn prompt_section(&self) -> Option<String> {
        let (mut out, mut used) = (vec![], 0usize);
        for l in self.data["lessons"].as_array().unwrap().iter().rev() {
            // newest first, so old ones fall off the cap
            let text = l["text"].as_str().unwrap_or("");
            if l["in_file"].as_bool().unwrap_or(false) {
                continue; // already in AGENTS.md, which the prompt carries
            }
            if used + text.chars().count() > PROMPT_CHARS {
                break;
            }
            out.push(format!("- {text}"));
            used += text.chars().count();
        }
        (!out.is_empty()).then(|| format!("# Learned in this repo\n\nThe user taught you these; follow them:\n{}", out.join("\n")))
    }

    /// Call only for runs that ended verified; `changed` are repo-relative paths.
    pub fn record_run(&mut self, request: &str, changed: &[String]) {
        let mut seen = HashSet::new();
        let terms: Vec<String> = tokenize(request).into_iter().filter(|t| seen.insert(t.clone())).take(40).collect(); // first-said words carry the subject of a long brief
        let files: Vec<&String> = changed.iter().filter(|f| self.root.join(f).is_file()).take(8).collect();
        let dup = self.data["edits"].as_array().unwrap().iter().any(|e| e["terms"] == json!(terms) && e["files"] == json!(files));
        if terms.len() < 3 || files.is_empty() || dup {
            return;
        }
        let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let edits = self.data["edits"].as_array_mut().unwrap();
        edits.push(json!({"terms": terms, "files": files, "ts": ts}));
        if edits.len() > MAX_EDITS {
            let drop = edits.len() - MAX_EDITS;
            edits.drain(..drop);
        }
        self.save();
    }

    /// File -> boost (0..1) from past verified runs whose request resembled this query.
    pub fn prior(&self, query: &str) -> HashMap<String, f64> {
        let q: HashSet<String> = tokenize(query).into_iter().collect();
        let mut boosts: HashMap<String, f64> = HashMap::new();
        if q.is_empty() {
            return boosts;
        }
        for e in self.data["edits"].as_array().unwrap() {
            let t: HashSet<String> = e["terms"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
            // share of the query's words, not of both: a 40-word brief would otherwise drown a 5-word search
            let overlap = q.intersection(&t).count() as f64 / q.len() as f64;
            if overlap >= MIN_OVERLAP {
                for f in e["files"].as_array().into_iter().flatten().filter_map(Value::as_str) {
                    let b = boosts.entry(f.to_string()).or_insert(0.0);
                    *b = (*b + overlap).min(1.0);
                }
            }
        }
        boosts
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lessons_and_priors() {
        let dir = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        std::fs::write(repo.path().join("a.py"), "x").unwrap();
        let mut l = Learned::with_base(repo.path(), dir.path());
        assert!(!l.add_lesson("too short", "user"));
        assert!(l.add_lesson("Always run the linter before committing code", "user"));
        assert!(!l.add_lesson("always run the linter before committing code", "user"));
        assert!(std::fs::read_to_string(repo.path().join("AGENTS.md")).unwrap().contains("- Always run the linter"));
        l.record_run("fix the description blank lines on the event page", &["a.py".into()]);
        let l2 = Learned::with_base(repo.path(), dir.path());
        assert_eq!(l2.lessons().len(), 1);
        assert!(l2.prior("event description blank lines").contains_key("a.py"));
        let mut l2 = l2;
        assert!(l2.forget(1).is_some());
        assert!(!std::fs::read_to_string(repo.path().join("AGENTS.md")).unwrap().contains("Always run"));
    }

    #[test]
    fn a_rule_worded_differently_is_not_written_twice() {
        let dir = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        let mut l = Learned::with_base(repo.path(), dir.path());
        assert!(l.add_lesson("Continue the loop by reading .zerd/loop/x.md, completing the next task, and updating Progress", "user"));
        // the judge paraphrases the same rule next time: it is not a new lesson, and AGENTS.md must not grow a
        // second copy of it
        assert!(!l.add_lesson("Continue the loop by reading .zerd/loop/x.md, completing the next task, and updating Progress with what remains", "user"));
        let body = std::fs::read_to_string(repo.path().join("AGENTS.md")).unwrap();
        assert_eq!(body.lines().filter(|l| l.trim_start().starts_with("- ")).count(), 1, "one rule, one line: {body}");
    }

    #[test]
    fn a_paraphrase_that_rewords_more_is_still_the_same_rule() {
        // the judge can drop the file path and say the same thing in other words: still one rule
        let kept = "Continue the loop by reading `.zerd/loop/20261124-212302.md`, completing and proving each checklist item with its exact command, recording progress and remaining work under `# Progress`, and setting `STATUS: DONE` only after every “Done when” item is verified by running its command.";
        let reworded = "Continue the loop from its current state; update `# Progress` with completed work and remaining items, and set `STATUS: DONE` only after every “Done when” criterion passes by running its command.";
        assert!(same_rule(kept, reworded), "a reworded rule is the same rule");
        // but two rules that differ in the words that carry the meaning are two rules
        assert!(!same_rule("this lesson is noise and goes", "this lesson is fine and stays"));
    }

    #[test]
    fn a_different_rule_is_still_written() {
        let dir = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        let mut l = Learned::with_base(repo.path(), dir.path());
        assert!(l.add_lesson("Always run the linter before committing code", "user"));
        assert!(l.add_lesson("Keep the changelog updated for every user-facing change", "user"));
        let body = std::fs::read_to_string(repo.path().join("AGENTS.md")).unwrap();
        assert_eq!(body.lines().filter(|l| l.trim_start().starts_with("- ")).count(), 2, "{body}");
    }

    #[test]
    fn a_paraphrase_of_a_rule_already_learned_is_not_a_new_lesson() {
        // The store outlives the process, so a rule learned by an older session must stop a paraphrase from
        // being learned again, even when AGENTS.md was never written.
        let dir = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        let mut l = Learned::with_base(repo.path(), dir.path());
        assert!(l.add_lesson("Always run the linter before committing code", "user"));
        // same rule, different words
        assert!(!l.add_lesson("Always run the linter before you commit any code", "user"));
        // a different rule is still learned
        assert!(l.add_lesson("Keep the changelog updated for every user-facing change", "user"));
        let lessons = l.data["lessons"].as_array().unwrap();
        assert_eq!(lessons.len(), 2, "the paraphrase must not become a lesson");
    }
}
