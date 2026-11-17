# Building a coding agent harness that adapts to the project

This is a reworked version of a widely shared "world-class harness" guide. The original had good instincts (treat the
codebase as a graph, scale checks to risk, learn from corrections). It also had numbers and code that break in practice.
This version keeps the instincts, fixes the parts that don't hold up, and adds what the original left out. The last section
lists where zerd already does each part and what it still lacks.

The rule behind every section: **anything the harness can decide in code shouldn't be left to the model.** Code is free,
repeatable and traceable. A model call is none of those.

---

## 1. Read the project, don't fill in a questionnaire

The original starts with a four-question form (codebase type, team size, task mix, risk). People fill it in once and it goes
stale. Most of the answers are already in the repo:

| Question | Where the answer already is |
|---|---|
| Stack and commands | `package.json`, `Cargo.toml`, `pyproject.toml`, `Makefile`, CI config |
| How risky is a change | the paths it touches (`auth/`, `migrations/`, `billing/`), not a project-wide label |
| Team size | `git shortlog -sn --since=90.days` |
| Task mix | the requests in past sessions |

Ask the user only for what the repo can't say, like "never touch `vendor/`" or "this table is shared with the billing team".
Keep it in a file they own (`AGENTS.md`, a profile) so it can be read, edited and committed.

**What changed from the original:** risk is per change, not per project. A typo fix in a payments repo is low risk. A
migration in an internal tool is not.

---

## 2. The codebase is a graph, but build it live

The original idea is right: find your place in the code through the graph instead of reading files front to back. Two parts
of its code need fixing.

**An import graph is not a call graph.** "Who imports `db/client.py`" is not "who calls `charge()`". For a bug fix you want
callers of the function, so build a **symbol graph** (tree-sitter gives definitions and references for most languages in a
few hundred lines) and answer `callers`, `callees`, `impact` and `path` from it. Keep the import graph for "which test
files cover this module".

**Don't precompute and store the transitive impact of every file.** That's quadratic in size, it goes stale the moment
someone edits a file, and an agent working from a stale graph is worse off than one with no graph. Build it from the working
tree, cache it by file modification time, and run the BFS when it's asked for.

Use the graph in two places:

1. **As a tool the model can call.** `callers parse_invoice` returns 6 lines where reading three files costs 900.
2. **As feedback after an edit.** When an edit changes a function's signature, attach the call sites to the edit result.
   The model learns its blast radius at the moment it can still act on it, and you don't pay up front for files it never
   needed.

Traversal by task type is a useful prompt hint, not something to hard-code: bug fixes walk up from the failing line
(callers), features walk down from the entry point (callees), refactors map both directions before touching anything.

---

## 3. Spend tokens like they cost money

### Budgets: count rounds, not tokens per task type

The original's table (10K tokens for a bug fix, 28K for a feature) can't work in an agent loop. A system prompt plus tool
definitions alone are often over 10K, and every round re-sends the conversation. A two-file fix is often 15 to 30 model
calls.

What works:

- Before the task, the model **estimates how many rounds** (model turns) it needs.
- At the end, score the estimate against what was used, and show the model its recent record the next time it estimates
  ("you ran over by 1.8x on average"). Estimates drift toward honest without training anything.
- Put a **hard stop** at a multiple of the estimate (3x, with a floor), plus a token ceiling for the whole session.
- When a run hits the stop, make one last tool-free call that turns what it learned into an answer. Never end silently.

### Context: send the smallest thing that answers the question

- Read by symbol (`read_file(symbol="charge_customer")`) or line range, not whole files.
- Remember what the model has already seen. A second read of an unchanged file returns one line; a changed file returns a
  diff.
- Cap big tool output, archive the full text, and let the model pull ranges back by id.
- When the context fills, prune old tool results first (cheap, keeps a note of what was cleared), and only then summarize.

### Caching: keep the prefix stable

Prompt caching only pays if the start of the prompt stays byte-for-byte the same. The original says "mark the system prompt
cacheable" and stops there. What actually breaks the cache:

- A timestamp, a random id or a "current files" list near the top of the prompt.
- **Switching models mid-session.** The new model has an empty cache and re-bills the whole context. Past ~20K tokens of
  context, staying on the current model is usually cheaper than moving to a "better" one for the next step.
- Summarizing with a different model than the one that just ran. Summarize with the same model on the same prefix and the
  provider serves most of it from cache.

### Routing: decide in code first

The original's table is right in spirit: schema checks, syntax, type errors and "does this break callers" are compilers,
parsers and graph queries, so they cost nothing. For the model calls that remain:

- Classify the request with plain rules (keywords, length, number of paths named) and **log the reason**. A routing
  decision nobody can explain can't be tuned.
- Escalate on **evidence of trouble** (a failed repair, a detected loop), not on a guess up front.

---

## 4. Instructions: intent, constraints, and how to check

The original's advice holds: give the goal, the constraints and the "done when", not twenty implementation steps. Two
corrections.

**Strong wording isn't the enemy, unexplained wording is.** "Never run migrations against production" is fine. "NEVER EVER
DO THIS!!!" with no reason invites the model to over-apply it everywhere. Give the reason in one clause and let the model
judge the edges.

**Instructions are the weakest enforcement.** For each rule, ask whether it can be something stronger:

| Rule | Stronger form |
|---|---|
| "Don't edit `vendor/`" | a guard rule that refuses the write |
| "Run tests before you finish" | the harness runs them after every edit |
| "Use `Result`, not exceptions" | a lint rule |
| "Don't drop public parameters" | a signature check against `HEAD` |

What's left in `AGENTS.md` is what genuinely needs judgment: conventions with an example, files that need extra care and why,
and what to ask about.

Domain terms (the original's "anthology") belong in the same file when they matter: `settlement date = T+2 business days`.
Write one only when someone gets it wrong, not upfront.

---

## 5. Isolation: snapshots, and worktrees only when two agents share a repo

The original's decision tree is close. Two changes.

- **"Every agent action is a commit" pollutes the user's history.** Snapshot the working tree to a private ref
  (`refs/<harness>/backup`) before a run instead. Undo is exact, `git log` stays the user's own, and the user commits when
  they choose. Committing per logical step is a habit for the human's branch, not something the harness imposes.
- **Worktrees are for concurrency.** Parallel workers that write files each get their own checkout. Tell each worker its own
  path, or it will reach for the original repo's absolute paths. A single agent on one task doesn't need one.

Auto-merge doesn't apply to a terminal agent working in your checkout. Drop it unless you run headless agents that open PRs.

---

## 6. Verification, scaled to the change

This is where the original had the right idea and the weakest code. Its risk classifier runs regexes over the **diff
text**: `role` matches `console.role`, `iam` matches `william`, `model` matches nearly every file. Everything comes out
HIGH, so the tiers mean nothing.

Classify by **path**, on word boundaries, and ignore tests and docs:

```text
src/auth/session.ts        -> high   (auth)
db/migrations/0042.sql     -> high   (migrations)
src/billing/invoice.py     -> high   (billing)
src/routing_policy.rs      -> low    ("policy" alone is too broad to flag)
tests/test_auth.py         -> low    (a test can't break production)
```

Then scale what runs:

| Every change | Plus, on a high-risk path |
|---|---|
| Checks for the **changed files only**: the tests that import them, the type checker for that package, the linter | An independent reviewer reads the diff, even when review is off by default |
| Per-edit diagnostics (syntax, types) attached to the edit result | Security lens in the reviewer |
| Signature check against `HEAD` | The user decides on anything the reviewer still flags |

Three things the original misses, and they matter more than the tiers:

1. **Whose failure is it?** When a check fails, run it again on the pre-run copy of the files. If it fails there too, the
   agent didn't break it. Say so and leave it alone instead of sending the agent to "fix" code it never touched.
2. **A claim is checked against the disk.** Every `path:line` in the final answer is looked up. A file that doesn't exist is
   a hallucination you can catch for free.
3. **The status says what was proven.** `verified`, `unverified` (and why: no check covers it, a check was already failing,
   still running), `verification_failed`, `failed`. Never a bare "done".

---

## 7. Learning from corrections without forms

The original logs each human override with a six-way error taxonomy and a weekly ten-minute review. Nobody fills in the form
after the first week.

- **Capture automatically.** After each user message, a cheap background call asks: did this redirect the agent ("no, use
  the helper"), confirm an approach ("yes, like that"), or state a preference ("always pnpm")? If so, write one rule.
  Rejecting a tool call with a reason is a lesson immediately.
- **Store rules where the team can see them**, in the repo's `AGENTS.md`, capped and deduplicated, with a way to remove one.
- **Learn only from verified work.** A run that ended `verified` records which files fixed which kind of request, so search
  ranks them higher next time. That's the original's "traversal history" in a form that can't learn from a wrong fix.
- **Replace the weekly ritual with a report built from traces**: failure rates, malformed tool calls, loops and nudges per
  model. Tune from what happened, not from a memory of one bad session.

---

## 8. Evals: small, real, and run on every harness change

The original's staging is sound. Fix the details:

- Start with **10 to 20 real tasks with hidden tests**: a bug fix, a multi-file feature, a refactor that must keep callers
  working, a destructive request the agent should refuse. Pass/fail comes from running the tests, not from a model's opinion.
- Track **tokens, cache hit rate and rounds** next to the pass rate. A change that passes the same tasks at 30% more cost is
  a regression.
- Turn every real miss into a case. The best guard rails come from failures you actually saw.
- An LLM judge needs calibrating against human labels before you trust it (the original says 20 in one place and 50 in
  another). Prefer execution checks wherever an outcome can be run.

---

## 9. What to skip

- Hand-written procedure graphs in JSON. Short markdown playbooks that attach by keyword do the same job and are easier to
  maintain. Move to a structured procedure only when a playbook keeps getting skipped.
- A fine-tuning dataset from corrections, until you have thousands of them.
- Per-project budget matrices. Rounds plus a hard stop adapt on their own.

---

## Where zerd stands

| Part | In zerd | Where |
|---|---|---|
| Project facts from the repo | yes: checks picked from manifests, `AGENTS.md`, folder rules, profiles | `agent/orient.rs`, `agent/instructions.rs`, `agent/profile.rs` |
| Live symbol graph, callers on signature change | yes | `util/symgraph.rs`, `tools/builtin/graph.rs` |
| Symbol reads, seen-line memory, archive and recall | yes | `tools/builtin/read_file.rs`, `util/archive.rs` |
| Round estimates, scoring, hard stop | yes | `agent/ledger.rs` |
| Cache-preserving routing and compaction | yes | `agent/routing_policy.rs`, `context/compaction.rs` |
| Guard rules, workspace confinement, sandbox | yes | `safety/` |
| Snapshot refs, worktrees for parallel workers | yes | `agent/verify.rs` (`backup`), `tools/builtin/parallel_tasks.rs` |
| Checks on changed files, pre-existing failure split, signature guard, citation check | yes | `agent/verify.rs`, `agent/grounding.rs` |
| Review pipeline with refuting verifier and repro | yes, on request or with `auto_review` | `agent/review_pipeline.rs` |
| **Review scaled to risk** | **added with this guide**: a high-risk path gets an independent review even with `auto_review` off (`review_risky`, on by default) | `agent/routing_policy.rs`, `agent/agent.rs` |
| Correction capture into `AGENTS.md`, verified-only edit history | yes | `agent/learn.rs` |
| Trace report | yes, `zerd insights` | `util/insights.rs` |
| Benchmarks | small: two rounds, kept outside the repo | README "The proof" |
