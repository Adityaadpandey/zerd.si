import type { Metadata } from "next";
import Link from "next/link";
import CopyCmd from "../copy-cmd";
import { Walkthrough, Tabs, Fork, Stages, Endings, Limits, type Shot, type Tab } from "./interactive";
import { Arrow } from "../icons";
import { NAME, SITE } from "@/lib/site";
import "./how.css";

const TITLE = "How zerd works: it tests and reviews every change the AI makes";
const DESC = "See a real zerd run step by step: it edits your code, runs your tests, has a second model review the diff, and looks up every file and line it mentions.";

export const metadata: Metadata = {
  title: TITLE,
  description: DESC,
  alternates: { canonical: "/how-it-works" },
  robots: { index: true, follow: true },
  openGraph: { type: "website", siteName: NAME, locale: "en_US", url: `${SITE}/how-it-works`, title: TITLE, description: DESC },
  twitter: { card: "summary_large_image", title: TITLE, description: DESC },
};

const INSTALL = "curl -fsSL https://www.zerd.si/install.sh | sh";

// Real screenshots from the recording on the home page (public/shots). The dots point at what to look at.
const SHOTS: Shot[] = [
  {
    title: "You ask in plain words", one: "Type what you want, like you’d tell a teammate.",
    more: "There is no special syntax. This request is: “send_with_retry never waits between tries after a 429. fix it so it backs off and respects retry_after”.",
    img: "/shots/step-1.webp", alt: "zerd’s terminal with the request typed in and the model shown at the top",
    pins: [{ x: 45, y: 31.5, label: "Your request, exactly as typed" }, { x: 37, y: 11.6, label: "The model and approval mode, shown at the top" }],
  },
  {
    title: "It reads only what it needs", one: "It searches your project and opens the files that matter.",
    more: "Here it found 5 matches in 2 files and read both: the code and its tests. Every file it reads is listed in the side panel.",
    img: "/shots/step-2.webp", alt: "zerd searching the project and reading two files",
    pins: [{ x: 34, y: 50, label: "Searched, listed and read 2 files" }, { x: 76, y: 7, label: "Side panel: every file it has read" }],
  },
  {
    title: "It edits the file", one: "Changes appear as a diff.",
    more: "Red lines were removed and green lines were added. The bug was time.sleep(0) after a rate-limit error, so it never waited. The side panel counts what was written.",
    img: "/shots/step-3.webp", alt: "zerd showing a diff with red removed lines and green added lines",
    pins: [{ x: 36, y: 57, label: "Red was removed, green was added" }, { x: 80, y: 38, label: "Written: 1 file, +4 lines, −2 lines" }],
  },
  {
    title: "It runs your tests", one: "Right after editing, it runs the project’s own tests.",
    more: "The command shows as it runs, and the side panel lists every command it has run, with a ✓ when it finishes.",
    img: "/shots/step-4.webp", alt: "zerd running a pytest command after the edit",
    pins: [{ x: 30, y: 77, label: "The test command it ran" }, { x: 80, y: 62, label: "Every command it ran, with a ✓" }],
  },
  {
    title: "It reports what happened", one: "It says what it fixed, what the tests showed, and what it hasn’t proven yet.",
    more: "The tests show “3 passed”. The first try used python, which isn’t on this machine, so zerd said so and used python3. The status still reads unverified: zerd’s own checks haven’t run yet, so it won’t claim more than it knows.",
    img: "/shots/step-5.webp", alt: "zerd’s summary with 3 tests passed and an unverified status line",
    pins: [{ x: 30, y: 77.5, label: "Tests: 3 passed, and a note that it used python3" }, { x: 18, y: 86, label: "Status: unverified. /verify runs the checks, /undo reverts" }],
  },
  {
    title: "You run /verify", one: "One command runs the checks on the files the last request changed.",
    more: "zerd picks the checks for the files it changed. Here that is pytest on tests/test_client.py, and a ✓ shows when it passes. If something had broken, it would send the model back to repair it.",
    img: "/shots/step-6.webp", alt: "zerd running /verify and a check passing",
    pins: [{ x: 8, y: 79.5, label: "You type /verify" }, { x: 34, y: 86, label: "The check runs and passes" }],
  },
];

// Messages below are ones zerd really prints. They appear on bigger changes than the demo.
const MORE: Tab[] = [
  { name: "Pick a model", title: "It picks the right model for the job", text: "Simple rules choose a fast, default or strong model for your request. There is no extra model call for this, and zerd prints which one it chose and why, so you know what you are paying for.", lines: ["Routing to the strong model (read by the fast model): claude-opus-5-5"] },
  { name: "Guard your API", title: "It protects code that depends on yours", text: "Tests can’t see callers outside your repo, so zerd compares each changed Python and JS or TS file with HEAD. A dropped parameter, a new required one, reordered positional arguments, a removed *args or **kwargs, or an export that disappeared sends the model back once.", lines: ["The change breaks 1 public signature(s) (utils.py: unique_by() no longer accepts `seen`); asking to keep old calls working"] },
  { name: "Review the diff", title: "A second model reads the real diff", text: "It runs when you ask for a review, when auto_review is on, or when the change touches paths like src/auth/, db/migrations/ or billing/. Problems go back for one fix, then the review runs again.", lines: ["Independent review of the diff...", "Reviewer found issues; sending them back for a fix", "Independent review of the diff..."] },
  { name: "Check the claims", title: "It looks up every file and line it mentions", text: "If the answer names a file that doesn’t exist, or a line past the end of one, the model gets one chance to correct it. Anything left becomes a warning. This is plain code with no model call, so it can’t be argued with.", lines: ["status: verified   checks: 1 passed   review: clean   citations: 4 checked, 0 unverified"] },
];

const REVIEW: [string, string][] = [
  ["Find the risks", "A cheap first pass picks what could realistically break."],
  ["Review from several angles", "Separate reviewers look at behavior, approach, failure cases, your project’s rules, git history and security."],
  ["Try to disprove each finding", "A verifier whose default is “this is wrong” has to name the exact line it read before a finding survives."],
  ["Try to reproduce it", "For serious findings, an agent tries to trigger the bug in a sandboxed copy. If it can’t, the finding drops to medium and shows what was tried."],
  ["Score and filter", "Low-confidence findings are dropped. Findings that were disproved are remembered per repo for two weeks."],
];

const ENDS = [
  { name: "verified", tone: "ok", what: "Files changed and the checks passed.", when: "You ask for a fix and the tests for the changed files pass." },
  { name: "unverified", tone: "warn", what: "There was nothing to check, or a check was already failing before the run.", when: "A project with no tests yet, or a test that was already red before zerd touched anything." },
  { name: "verification_failed", tone: "bad", what: "Checks still fail after the repair attempts.", when: "zerd tried two repairs, then a stronger model, and the tests still fail." },
  { name: "failed", tone: "bad", what: "An API error, a loop, or the time limit.", when: "Your API key ran out of quota, or the model got stuck repeating itself." },
  { name: "done_no_changes", tone: "dim", what: "No files changed.", when: "You asked “what does retry_after do?” and zerd answered without editing anything." },
];

const LIMITS = [
  { name: "Files", text: "zerd works out the real location of every path first. Anything that leads outside your workspace is refused, including symlinks.", ok: ["src/app.py", "tests/test_app.py"], no: ["../../etc/passwd", "a symlink that points to ~/.ssh"] },
  { name: "Commands", text: "Shell commands are scanned before they run. Anything that touches your home folder, /etc or other users’ folders is refused, and git commands that destroy history are blocked.", ok: ["pytest -q", "git diff"], no: ["cat ~/.ssh/id_rsa", "ls /etc"] },
  { name: "Sandbox (macOS)", text: "On macOS a command also runs inside the operating system’s sandbox. Linux and Windows get the file and command checks above, not this layer.", ok: ["writing inside the workspace", "reading a package cache"], no: ["reading ~/.aws/credentials", "reading the workspace’s .env", "writing to /usr/local/bin"] },
];

const JUMPS: [string, string][] = [["Watch a real run", "run"], ["More checks", "more"], ["When a test fails", "fail"], ["Reviews", "review"], ["How runs end", "ends"], ["Safety", "safety"]];

export default function HowItWorks() {
  return (
    <main className="hiw">
      <header className="hiw-head wrap">
        <h1>How zerd works</h1>
        <p className="hiw-lead">You ask for a change. zerd edits your code, then proves it works before it says it’s done.</p>
        <p className="hiw-sub">Most AI coding tools stop when the model says “done.” zerd doesn’t take its word for it. It runs your checks, has a second model review the diff, and confirms every file it mentions is real.</p>
        <nav className="jumps" aria-label="On this page">{JUMPS.map(([t, id]) => (<a key={id} href={`#${id}`}>{t}</a>))}</nav>
      </header>

      <section className="wrap steps-sec" id="run" aria-labelledby="run-h">
        <h2 id="run-h" className="sec-title">Watch a real run</h2>
        <p className="blk-lead">These are screenshots from a real recording: zerd fixing a bug in a small Python project. Scroll down, use the arrows, or click a step number. Hover the dots on the screenshot to see what each part is.</p>
        <Walkthrough shots={SHOTS} />
      </section>

      <section className="wrap blk" id="more" aria-labelledby="more-h">
        <h2 id="more-h" className="sec-title">What else it checks</h2>
        <p className="blk-lead">The demo is a small change. On bigger ones zerd also runs these. Pick one.</p>
        <Tabs tabs={MORE} label="Other checks" />
      </section>

      <section className="wrap blk" id="fail" aria-labelledby="fail-h">
        <h2 id="fail-h" className="sec-title">When a test fails, whose fault is it?</h2>
        <p className="blk-lead">A failing test doesn’t always mean the model broke something. It may have been failing before the run. zerd finds out by experiment, so it never sends the model to fix what it didn’t break.</p>
        <Fork />
      </section>

      <section className="wrap blk" id="review" aria-labelledby="review-h">
        <h2 id="review-h" className="sec-title">How reviews work</h2>
        <p className="blk-lead">Ask zerd to “review my changes”, “review my last 3 commits” or “review PR #42”. It reviews in stages instead of with one prompt, and you only see the findings that survived. Step through them.</p>
        <Stages stages={REVIEW} />
        <p className="blk-note">The write-up also lists what was <em>not</em> checked, so you know where to look yourself.</p>
      </section>

      <section className="wrap blk" id="ends" aria-labelledby="ends-h">
        <h2 id="ends-h" className="sec-title">Five ways a run can end</h2>
        <p className="blk-lead">Every run ends in exactly one of these. zerd never claims more than it checked, and everything it does is written to a trace file you can read later.</p>
        <Endings ends={ENDS} />
      </section>

      <section className="wrap blk" id="safety" aria-labelledby="safety-h">
        <h2 id="safety-h" className="sec-title">What the model can’t do</h2>
        <p className="blk-lead">These limits are enforced in code, not requested in a prompt. The model can’t talk its way past them. A cloned repo also can’t switch on hooks, MCP servers or verify commands until you trust that exact file.</p>
        <Limits limits={LIMITS} />
      </section>

      <section className="wrap blk" id="undo" aria-labelledby="undo-h">
        <h2 id="undo-h" className="sec-title">Don’t like the result? Undo it.</h2>
        <p className="blk-lead">Type <code>/undo</code> and every file goes back to how it was before the run. zerd also learns from corrections: tell it “not like that” or “always use pnpm” and it saves one short rule in your own <code>AGENTS.md</code>, where you can read, edit and commit it.</p>
      </section>

      <section className="hiw-end wrap">
        <h2>Try it on one small change.</h2>
        <CopyCmd cmd={INSTALL} winCmd="irm https://www.zerd.si/install.ps1 | iex" />
        <p className="hiw-links">
          <Link href="/docs/quickstart">Get started <Arrow /></Link>
          <Link href="/proof">See the proof</Link>
          <Link href="/docs/how-a-run-works">Full run guide</Link>
        </p>
      </section>
    </main>
  );
}
