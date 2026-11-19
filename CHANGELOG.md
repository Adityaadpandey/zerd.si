# Changelog

All notable changes to zerd. Newest first. Versions follow [semver](https://semver.org).

## Unreleased

- `/loop <task>` keeps working on a task without asking for approval and goes again with what is still failing, until the checks pass, six rounds pass or you press Ctrl+C.

## 0.1.4

- Drop a screenshot into the input and zerd attaches it. An `[image: path]` line in a message goes to the model as a picture. Resumed sessions show only what you typed, not the nudges zerd sent.
- The side panel holds the work and the conversation holds what zerd says. Text printed before the first frame wraps to the conversation pane.
- Edits get sanity checks: CSS gets a brace check (a stray `}`, an unclosed `{`, a declaration outside any rule), and an edit that loses a selector line or a wrapping tag, or doubles a comma at its edge, says so. An eslint run handed CSS or Markdown files says it did not check them.
- `read_file` is more forgiving: top-level data like `export const cases = [...]` is a symbol, `use-client.mdx` opens `use-client.md`, and a read over the output cap stops on a whole line.
- `parallel_tasks` takes a single task and tasks without ids. A failed call shows the reason instead of the model's whole argument JSON.
- Nudges fit the task: a question tucked into a work request rides along as a reminder, "just do it" is not pushed when you asked how, a content question does not trigger code review, and an edit resets the write-down checkpoint.

## 0.1.3

- zerd checks once a day, when it opens in a terminal, whether a newer release is out and asks `Update now? [Y/n]`. Yes installs it next to the current binary and restarts on it. `zerd update` does the same on demand, and `/update` installs it from inside a session for the next start. `ZERD_NO_UPDATE_CHECK=1` turns the launch check off.
- zerd knows its name and who built it (Aditya Dutt Pandey, https://lab.adpandey.dev), so "who are you?" gets a straight answer instead of the underlying model's.
- The side panel shows from the start, and the conversation no longer repeats what it lists: files read, files changed, commands run and sub-agents get no line of their own while the panel is on screen. Failures still print. `/verbose` brings the full output back.
- A change to auth, payments, secrets or migrations gets an independent review even with `auto_review` off (`review_risky`, on by default).
- The code graph finds far more real callers: calls inside Rust macros (`assert!`, `format!`, `vec!`) count, Python imports resolve from the project folder in a monorepo, and TypeScript follows workspace packages (`@acme/utils`) and nested `index.ts` re-exports. A cut-off callers list says how many it left out. `zerd graph --help` prints usage.
- Checks run automatically in the interactive prompt again. One still running after 30 seconds (`verify_wait_sec`) moves to the background and prints its result when it finishes; a failure reaches the model with your next message.
- When no check covers the changed files, zerd asks the model once to run something that proves the change.
- An `unverified` result now says why (no check for these files, checks still running, token budget, a check that was already failing, docs only). It used to print "No verification commands found" for all of them.
- A run that hits the token budget before changing anything is no longer filed as `unverified`: it ends `failed` for a change request and `done_no_changes` for a question. `zerd insights` shows the reason beside each outcome and counts zerd's own reviewer and sub-agent runs separately.
- A run with nobody to approve a step (a script, a parallel worker) skips that step and finishes the rest, then lists what was skipped. It used to stop at the first one. The report carries the list as `needs_approval`.
- Parallel workers are told where their own checkout is and get the request with its paths rewritten to it, so they stop being blocked reaching for the original repo.

## 0.1.2

- The project is now called **zerd** (it was melon). Config moves to `.zerd/` and `~/.zerd`, env vars to `ZERD_*`. Nothing is read from the old names, so move your files by hand.
- Install from `https://zerd.si/install.sh` or `https://zerd.si/install.ps1`.
- New site at [zerd.si](https://zerd.si): a landing page with a recorded zerd run, and these docs as searchable pages. The docs are built from the README, so the README stays the one place they are written.
- Community files: contributing guide, code of conduct, security policy, issue forms and a PR template. CI gets weekly dependency updates and a RustSec audit.

## 0.1.1

First tagged release of the Rust binary.
