// The product's payoff, in one place: what a run leaves behind.
//
// The rest of the site talks about verification. This file is the artifact itself — the receipt
// zerd prints when a run ends. It is the thing a visitor should picture before they read anything
// else, so it is written here and reused by the landing page and the docs index rather than being
// retyped in two places that would then drift.

// One line of the receipt: a label, what happened, and how it ended.
export type ReceiptLine = {
  step: string;
  detail: string;
  // "ok" passed, "bad" failed and was repaired, "warn" could not be verified, "note" is information.
  state: "ok" | "bad" | "warn" | "note";
};

// The receipt itself. It mirrors a real zerd run: the checks that ran on the changed files, the
// second model that read the diff, the verifier that tried to disprove each finding, and the
// claims that could not be checked.
export const RECEIPT: ReceiptLine[] = [
  { step: "checks", detail: "pytest tests/test_paging.py · ruff on 2 files", state: "ok" },
  { step: "repair", detail: "1 failure on the first run, fixed and rerun", state: "ok" },
  { step: "review", detail: "second model read the real diff, 2 findings", state: "note" },
  { step: "verify", detail: "1 finding refuted, 1 held and reproduced", state: "warn" },
  { step: "paths", detail: "every path:line in the answer checked on disk", state: "ok" },
  { step: "unverified", detail: "2 claims it could not check, listed as such", state: "warn" },
];

// The last line: what the run is allowed to call itself.
export const RECEIPT_OUTCOME = {
  state: "ok",
  detail: "1 file changed · 2 checks ok · /undo reverts it",
} as const;
