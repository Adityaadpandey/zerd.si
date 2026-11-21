Checkout integrity: no overselling, holds that expire, and payments that can't double-book

Make every requirement below true in the current codebase. Some may already hold: verify each one first, and
change only what doesn't. This touches the booking repository/service, the worker, the Razorpay webhook,
shared types and the web payment step.

Server
A1. Capacity: confirmed bookings plus active holds never exceed a ticket tier's capacity, even with 25 checkout
    attempts for the last seat arriving at the same moment. Enforce it inside the database transaction (row lock
    or conditional update), not with a read-then-write check in application code.
A2. Hold expiry: a checkout hold expires after HOLD_TTL_SECONDS (default 600, read from config). Expired holds give
    their capacity back within 60 seconds, using the existing worker, no new service. Expiry must be idempotent and
    safe when two workers run it at the same time.
A3. Late payment: when Razorpay's payment.captured arrives for a hold that already expired, confirm the booking if
    capacity is still free; otherwise never confirm it, and either refund through the existing Razorpay client or,
    if there is no refund path, record the payment as needing a refund. Never go over capacity.
A4. Exactly once: duplicate webhook deliveries (same event id) and the client's payment-verify call racing the
    webhook must produce exactly one confirmed booking, one payment record, one settlement/transfer and one
    confirmation message.
A5. Money: amounts are integer paise end to end. The Razorpay order amount equals the server-computed total, and
    the server never trusts a price sent by the client.

Web
W1. The payment step shows the time left on the hold from the server's expiry timestamp (not a timer started on
    page load). When the hold expires it says so and offers to start checkout again, and it never opens Razorpay
    for an expired hold.

Constraints
- No schema changes and no new dependencies. If you conclude a schema change is unavoidable, put the exact SQL in
  your final report, do not run it, and implement everything that does not depend on it.
- Leave unrelated files alone (for example the existing merge conflict in GlassModal.tsx). Follow AGENTS.md.
- You may add tests in the repo's existing test setup to prove A1-A4.

Final report: for each of A1-A5 and W1 say "already true", "changed" or "not done", with file:line evidence and
the exact command or test that proves it. Do not claim anything you did not prove.
