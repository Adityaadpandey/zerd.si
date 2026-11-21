import { RECEIPT, RECEIPT_OUTCOME, type ReceiptLine } from "@/lib/receipt";

// The receipt, drawn the way zerd draws it: a terminal panel with the run's steps and the outcome
// line at the bottom. This is the artifact the whole site argues for, so it is drawn as a thing
// you can read at a glance rather than described in a sentence.
//
// The state colour follows the one rule on the site: green means a check passed, red means it
// failed, amber means nobody could check it. Nothing else is coloured.
const MARK: Record<ReceiptLine["state"], string> = {
  ok: "✓",
  bad: "✗",
  warn: "!",
  note: "·",
};

export default function Receipt() {
  return (
    <div className="receipt" role="img" aria-label="A zerd run receipt: two checks passed on the changed files, a second model reviewed the real diff and one of its two findings was refuted by a verifier, every path was checked on disk, and two claims stayed unverified.">
      <div className="receipt-head">
        <span className="receipt-title">receipt</span>
        <span className="receipt-file">src/paging.py</span>
      </div>
      <ul className="receipt-lines">
        {RECEIPT.map((l) => (
          <li key={l.step} className={`receipt-line s-${l.state}`}>
            <span className="receipt-mark" aria-hidden="true">{MARK[l.state]}</span>
            <span className="receipt-step">{l.step}</span>
            <span className="receipt-detail">{l.detail}</span>
          </li>
        ))}
      </ul>
      <p className={`receipt-outcome s-${RECEIPT_OUTCOME.state}`}>
        <span className="receipt-mark" aria-hidden="true">{MARK[RECEIPT_OUTCOME.state]}</span>
        <span className="receipt-step">result</span>
        <span className="receipt-detail">{RECEIPT_OUTCOME.detail}</span>
      </p>
    </div>
  );
}
