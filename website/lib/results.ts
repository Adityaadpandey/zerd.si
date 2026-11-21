// The measured results, kept in one file so the landing page, the docs and the structured data
// quote the same numbers.
//
// Two rounds, both one run each, and neither is a ranking. Round 2 is the webhook reliability test
// (real Postgres, real Redis, a real killed worker). Round 1 is 11 mixed coding tasks. The numbers
// come from docs/benchmarks.mdx in the repo; they are transcribed here so the landing page can
// show them without pulling the whole MDX page into a homepage bundle.

// Round 2: the webhook reliability test. 15 scenarios, both agents, one run each.
export const WEBHOOK = {
  scenarios: "15 attack scenarios",
  vs: "Claude Code (Sonnet 5.5)",
  rows: [
    { label: "Scenarios with an automated test", zerd: "15 / 15", other: "14 / 15" },
    { label: "Database", zerd: "real PostgreSQL", other: "in-memory" },
    { label: "Queue", zerd: "real Redis", other: "throwing stub" },
    { label: "Worker crash", zerd: "SIGKILL", other: "lease expiry simulated" },
    { label: "Concurrency", zerd: "25 real races", other: "100 in memory" },
    { label: "Real bugs found", zerd: "12", other: "3" },
  ],
  // The line that keeps the table honest, in the table's own footer rather than a footnote.
  caveat: "One project, one run each. This measures how deeply each tool verified, not which model is better.",
} as const;

// Round 1: 11 mixed tasks. Both agents passed the same number; the cost differs.
export const TASKS = {
  label: "11 coding tasks, same hidden tests, one run each",
  rows: [
    { label: "Tasks passed", zerd: "10 / 11", other: "10 / 11" },
    { label: "Prompt tokens", zerd: "1.01M", other: "1.31M" },
    { label: "Input served from cache", zerd: "87%", other: "76%" },
  ],
  vs: "Claude Code (Sonnet 5.5)",
  caveat: "Different models and a small sample. A sanity check, not a leaderboard.",
} as const;
