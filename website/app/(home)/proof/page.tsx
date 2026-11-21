import type { Metadata } from "next";
import Link from "next/link";
import { Score, Compare, Finds, type Area } from "./interactive";
import { Arrow } from "../icons";
import { NAME, SITE } from "@/lib/site";
import "../how-it-works/how.css";

const TITLE = "The proof: zerd vs Claude Code on a real payments service";
const DESC = "We gave zerd and Claude Code the same job and attacked what each built with the same 15 scenarios. zerd tested on a real database, queue and killed process, and found 12 real bugs.";

export const metadata: Metadata = {
  title: TITLE,
  description: DESC,
  alternates: { canonical: "/proof" },
  robots: { index: true, follow: true },
  openGraph: { type: "website", siteName: NAME, locale: "en_US", url: `${SITE}/proof`, title: TITLE, description: DESC },
  twitter: { card: "summary_large_image", title: TITLE, description: DESC },
};

const SCENARIOS = [
  "100 identical webhooks at once, and two events for one payment",
  "A worker killed mid-job, and a crash between commit and acknowledge",
  "Redis down, Postgres down, and retries running out",
  "Bad signatures, oversized bodies, and one customer calling another’s admin routes",
  "Events arriving out of order",
];

const AREAS: Area[] = [
  { name: "Database", z: "A real PostgreSQL database (a schema-only copy, in its own isolated database).", c: "An in-memory fake repository.", why: "A fake can’t show SQL mistakes, locking problems or half-finished transactions." },
  { name: "Queue", z: "A real Redis client, with outages made by pointing it at dead ports.", c: "Made the enqueue call throw an error.", why: "A fake that throws can’t show a call that hangs forever, which is how the outbox stopped draining." },
  { name: "Worker crash", z: "A real child process, killed with SIGKILL.", c: "Simulated by letting a lease expire.", why: "A real kill shows what is left half-done when a process dies between two steps." },
  { name: "HTTP", z: "The real Fastify app, called through its HTTP layer.", c: "The service code, without the web layer.", why: "Going through the real HTTP layer covers more than the service code alone." },
  { name: "Concurrency", z: "25 repeated races on one payment.", c: "100 calls at once in memory, with Promise.all.", why: "Repeating real races finds timing bugs that a single run can miss." },
  { name: "Coverage", z: "All 15 scenarios have an automated test.", c: "14 of 15 do. One was left as a known failure.", why: "Every scenario that has a test can fail loudly the next time someone changes the code." },
];

const FOUND = [
  { bug: "Nothing was ever saved", fix: "Unquoted camelCase names in the SQL meant no webhook ever reached the database." },
  { bug: "A captured payment ended up marked FAILED", fix: "Two requests overwrote each other (a lost update). Fixed with row locking and a transaction." },
  { bug: "Retries ran about 5.5 hours late", fix: "A timezone bug on a machine set to IST. Fixed by using UTC." },
  { bug: "The outbox stopped sending", fix: "With Redis down, one call hung forever. Fixed with a time limit." },
  { bug: "An event got stuck after a database outage", fix: "The recovery sweep ignored events in the RECEIVED state." },
  { bug: "Two workers queued the same event", fix: "Fixed by claiming each event inside a transaction." },
  { bug: "A crash marked an event as ignored", fix: "It happened after the side effect had already run." },
  { bug: "The queue never accepted a single job", fix: "BullMQ rejected job ids that contain a colon, so the original enqueue path never added a job." },
];

export default function Proof() {
  return (
    <main className="hiw">
      <header className="hiw-head wrap">
        <h1>The proof</h1>
        <p className="hiw-lead">We gave zerd and Claude Code the same job, then attacked what each one built.</p>
        <p className="hiw-sub">zerd tested on a real database, a real queue and a really killed process. That is how it found bugs that fake versions can’t show.</p>
        <Score />
      </header>

      <section className="wrap blk" aria-labelledby="test">
        <h2 id="test" className="sec-title">The test</h2>
        <p className="blk-lead">Both tools got the same task on separate branches: build a payment webhook service. It checks signatures, never processes the same event twice, keeps an outbox, runs queue workers, retries failures, parks dead jobs and lets an admin replay events. Then both attacked it with the same 15 scenarios, including:</p>
        <ul className="chips">{SCENARIOS.map((s) => <li key={s}>{s}</li>)}</ul>
      </section>

      <section className="wrap blk" aria-labelledby="side">
        <h2 id="side" className="sec-title">How each one tested it</h2>
        <p className="blk-lead">Each area is something both tools tested, in different ways. Pick one.</p>
        <Compare areas={AREAS} />
      </section>

      <section className="wrap blk" aria-labelledby="found">
        <h2 id="found" className="sec-title">What zerd found</h2>
        <p className="blk-lead">Running against the real thing turned up bugs that in-memory tests can’t show. Eight of the twelve. Open one.</p>
        <Finds items={FOUND} />
      </section>

      <section className="wrap blk" aria-labelledby="claude">
        <h2 id="claude" className="sec-title">What Claude Code found</h2>
        <p className="blk-lead">Claude Code found a crash in the failure path caused by an <code>undefined</code> value, and admin retries that corrupted the attempt history. It also found one bug zerd missed: a refund being applied before its payment when events arrive out of order. It said plainly that it had not touched a real Postgres, Redis, HTTP server or process kill.</p>
      </section>

      <section className="wrap blk" aria-labelledby="why">
        <h2 id="why" className="sec-title">Why this matters</h2>
        <p className="blk-lead">Real webhooks break when two workers do the same job, when a database commits but the queue call fails, when a process dies between two steps, when a retry is scheduled wrong, or when a payment event arrives out of order. Code can read correctly and pass in-memory tests and still get all of those wrong. Running against the real database, the real queue and a really killed process is how you find out which.</p>
        <p className="honest"><b>Read this as depth, not a ranking.</b> It was one project and one run each. In a separate round of 11 mixed tasks, both tools passed the same number. <Link href="/docs/benchmarks">Full details in the docs.</Link></p>
      </section>

      <section className="hiw-end wrap">
        <h2>Try it on your own code.</h2>
        <p className="hiw-links">
          <Link href="/docs/quickstart">Get started <Arrow /></Link>
          <Link href="/how-it-works">How it works</Link>
        </p>
      </section>
    </main>
  );
}
