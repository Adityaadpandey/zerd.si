// Hidden test for h1-checkout-integrity. Black-box: sets up its own data straight in Postgres, then drives the
// real HTTP API. Prints one line per requirement: PASS / FAIL / SKIP <id> <detail>. Exit 1 if anything FAILs.
// Run by test.sh from the worktree root, with the API (and two workers) already running. Needs DATABASE_URL,
// JWT_SECRET, RAZORPAY_WEBHOOK_SECRET; RAZORPAY_KEY_ID/SECRET for the parts that talk to Razorpay's test API.
const path = require("path");
const crypto = require("crypto");
const need = require("module").createRequire(path.join(process.cwd(), "apps/server/package.json"));
const { Client } = need("pg");
const jwt = need("jsonwebtoken");

const API = (process.env.API || "http://localhost:7001") + "/api/v1";
const E = process.env;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const rid = (p) => `${p}_${crypto.randomBytes(6).toString("hex")}`;
const results = [];
function report(id, ok, detail) {
  const word = ok === null ? "SKIP" : ok ? "PASS" : "FAIL";
  console.log(`${word} ${id} ${detail}`);
  results.push(ok);
}

// ---------------------------------------------------------------- database helpers (schema-agnostic inserts)
const db = new Client({ connectionString: (E.DATABASE_URL || "").split("?")[0] });
const colCache = {};
async function cols(table) {
  if (!colCache[table]) {
    const r = await db.query(
      `SELECT column_name, data_type, udt_name, is_nullable, column_default
         FROM information_schema.columns WHERE table_schema = 'public' AND table_name = $1`, [table]);
    if (!r.rows.length) throw new Error(`table ${table} not found`);
    colCache[table] = r.rows;
  }
  return colCache[table];
}
async function enumLabels(udt) {
  const r = await db.query(
    `SELECT e.enumlabel FROM pg_type t JOIN pg_enum e ON e.enumtypid = t.oid WHERE t.typname = $1 ORDER BY e.enumsortorder`, [udt]);
  return r.rows.map((x) => x.enumlabel);
}
async function pick(udt, prefs) {
  const labels = await enumLabels(udt);
  return prefs.find((p) => labels.includes(p)) || labels[0];
}
function filler(c) {
  const t = c.data_type;
  if (t === "ARRAY") return [];
  if (t === "jsonb" || t === "json") return "{}";
  if (t === "boolean") return false;
  if (/int|numeric|double|real/.test(t)) return 0;
  if (/timestamp|date/.test(t)) return new Date();
  if (t === "uuid") return crypto.randomUUID();
  return rid("bench"); // text: random, so unique columns never collide
}
async function insert(table, vals) {
  const cs = await cols(table);
  const row = { ...vals };
  for (const c of cs) {
    if (row[c.column_name] !== undefined) continue;
    if (c.is_nullable === "NO" && c.column_default === null) {
      row[c.column_name] = c.data_type === "USER-DEFINED" ? (await enumLabels(c.udt_name))[0] : filler(c);
    }
  }
  const names = Object.keys(row);
  const params = names.map((n, i) => {
    const c = cs.find((x) => x.column_name === n);
    if (!c) throw new Error(`${table}.${n} does not exist`);
    if (c.data_type === "USER-DEFINED") return `$${i + 1}::"${c.udt_name}"`;
    if (c.data_type === "jsonb" || c.data_type === "json") return `$${i + 1}::${c.data_type}`;
    return `$${i + 1}`;
  });
  const sql = `INSERT INTO "${table}" (${names.map((n) => `"${n}"`).join(",")}) VALUES (${params.join(",")}) RETURNING *`;
  const r = await db.query(sql, names.map((n) => row[n]));
  return r.rows[0];
}
const one = async (sql, args = []) => (await db.query(sql, args)).rows[0];
// rows in a table whose JSON mentions any of the given ids: a schema-agnostic way to count side effects
async function mentions(table, ids) {
  try {
    const r = await db.query(`SELECT count(*)::int AS n FROM "${table}" x WHERE row_to_json(x)::text LIKE ANY($1)`,
      [ids.filter(Boolean).map((i) => `%${i}%`)]);
    return r.rows[0].n;
  } catch {
    return null; // table does not exist in this database
  }
}

// ---------------------------------------------------------------- fixtures
async function user() {
  const userId = crypto.randomUUID();
  await insert("User", {
    userId, email: `${rid("buyer")}@bench.test`, phone: `+9199${Math.floor(1e7 + Math.random() * 9e7)}`,
    name: "Bench Buyer", isActive: true, phoneVerified: true, emailVerified: true, profileComplete: true,
    role: await pick("Role", ["USER"]), updatedAt: new Date(),
  });
  const token = jwt.sign({ userId, role: "USER" }, E.JWT_SECRET, { algorithm: "HS256", expiresIn: 3600 });
  return { userId, token };
}
async function fixture({ price = 100, quantity = 1 } = {}) {
  const host = await user();
  const listerId = crypto.randomUUID();
  await insert("Lister", { listerId, userId: host.userId, status: await pick("ListerStatus", ["APPROVED", "ACTIVE", "VERIFIED"]), updatedAt: new Date() });
  const eventId = crypto.randomUUID();
  const when = new Date(Date.now() + 30 * 86400e3);
  await insert("Event", {
    eventId, listerId, title: "Bench checkout event", description: "bench", banner_horizontal: "x", banner_vertical: "x",
    banner_square: "x", date: when, time: when, location: "Bench Hall", tags: [], chips: [],
    status: await pick("Status", ["APPROVED", "LIVE", "PUBLISHED", "ACTIVE"]), updatedAt: new Date(),
  });
  const ticketTypeId = crypto.randomUUID();
  await insert("TicketType", { ticketTypeId, eventId, name: "GA", price, quantity, soldCount: 0, updatedAt: new Date() });
  return { eventId, ticketTypeId, quantity, price };
}
async function api(method, url, token, body, extraHeaders = {}) {
  try {
    const res = await fetch(API + url, {
      method, signal: AbortSignal.timeout(30000),
      headers: { "content-type": "application/json", ...(token ? { authorization: `Bearer ${token}` } : {}), ...extraHeaders },
      body: body === undefined ? undefined : typeof body === "string" ? body : JSON.stringify(body),
    });
    let json = null;
    try { json = await res.json(); } catch {}
    return { status: res.status, json };
  } catch (e) {
    return { status: 0, json: { error: String(e) } };
  }
}
function find(obj, key) {
  if (!obj || typeof obj !== "object") return undefined;
  if (key in obj) return obj[key];
  for (const v of Object.values(obj)) { const f = find(v, key); if (f !== undefined) return f; }
}
async function book(fx, buyer, qty = 1, extra = {}) {
  const r = await api("POST", "/bookings", buyer.token, {
    eventId: fx.eventId, commonFieldResponses: [], ...extra,
    items: [{ ticketTypeId: fx.ticketTypeId, quantity: qty, ticketFieldResponses: Array.from({ length: qty }, () => []), ...(extra.item || {}) }],
  });
  return { ...r, bookingId: find(r.json, "bookingId") };
}
const sold = async (fx) => (await one(`SELECT "soldCount" FROM "TicketType" WHERE "ticketTypeId" = $1`, [fx.ticketTypeId])).soldCount;
const bookingRow = (id) => one(`SELECT * FROM "Booking" WHERE "bookingId" = $1`, [id]);
async function payFor(bookingId, b) {
  const orderId = rid("order_bench");
  const p = await insert("Payment", {
    paymentId: crypto.randomUUID(), userId: b.userId, eventId: b.eventId, bookingId, razorpayOrderId: orderId,
    amount: b.total, platformFee: 0, listerRevenue: b.total, receiptId: rid("rcpt"),
    status: await pick("PaymentStatus", ["PENDING"]), notes: JSON.stringify({ bookingId }), updatedAt: new Date(),
  });
  return { orderId, paymentRowId: p.paymentId, razorpayPaymentId: rid("pay_bench") };
}
function webhookBody(b, pay, eventName = "payment.captured") {
  const now = Math.floor(Date.now() / 1000);
  return JSON.stringify({
    entity: "event", account_id: "acc_bench", event: eventName, contains: ["payment"], created_at: now,
    payload: { payment: { entity: {
      id: pay.razorpayPaymentId, entity: "payment", amount: Math.round(b.total * 100), currency: "INR", status: "captured",
      order_id: pay.orderId, method: "upi", captured: true, email: "buyer@bench.test", contact: "+919999999999",
      notes: { bookingId: b.bookingId }, created_at: now } } },
  });
}
async function sendWebhook(body, eventId) {
  const sig = crypto.createHmac("sha256", E.RAZORPAY_WEBHOOK_SECRET).update(body).digest("hex");
  return api("POST", "/payments/webhook", null, body, { "x-razorpay-signature": sig, "x-razorpay-event-id": eventId });
}
async function waitFor(fn, ms) {
  const end = Date.now() + ms;
  while (Date.now() < end) { const v = await fn(); if (v) return v; await sleep(1000); }
  return await fn();
}
// expire a hold the way production would: move its deadline into the past and give the worker a chance; if no
// worker picks it up in 90 s, do what the expiry would do (status + inventory) so A3 still tests late payment
async function expire(fx, bookingId, qty = 1) {
  await db.query(`UPDATE "Booking" SET "holdExpiresAt" = now() - interval '1 minute' WHERE "bookingId" = $1`, [bookingId]);
  const done = await waitFor(async () => (await bookingRow(bookingId)).status !== "HELD", 90000);
  if (done) return "worker";
  await db.query(`UPDATE "Booking" SET status = 'EXPIRED' WHERE "bookingId" = $1`, [bookingId]);
  await db.query(`UPDATE "TicketType" SET "soldCount" = "soldCount" - $2 WHERE "ticketTypeId" = $1`, [fx.ticketTypeId, qty]);
  return "simulated";
}

// ---------------------------------------------------------------- A1: no oversell under 25 concurrent checkouts
async function a1() {
  const rounds = [];
  for (let i = 0; i < 3; i++) {
    const fx = await fixture({ quantity: 1 });
    const buyers = await Promise.all(Array.from({ length: 25 }, user));
    const rs = await Promise.all(buyers.map((b) => book(fx, b)));
    const ok = rs.filter((r) => r.status >= 200 && r.status < 300).length;
    const err5 = rs.filter((r) => r.status >= 500 || r.status === 0).length;
    const active = (await one(`SELECT count(*)::int AS n FROM "Booking" WHERE "eventId" = $1 AND status IN ('HELD','PAID')`, [fx.eventId])).n;
    const s = await sold(fx);
    rounds.push({ ok, err5, active, s, codes: [...new Set(rs.map((r) => r.status))].join("/") });
  }
  const pass = rounds.every((r) => r.ok === 1 && r.active === 1 && r.s === 1 && r.err5 === 0);
  report("A1", pass, rounds.map((r, i) => `round${i + 1}: ${r.ok} ok, ${r.active} active, soldCount ${r.s}, 5xx ${r.err5}, codes ${r.codes}`).join("; "));
}

// ---------------------------------------------------------------- A2: HOLD_TTL_SECONDS, expiry by the worker, idempotent
async function a2() {
  const fx = await fixture({ quantity: 1 });
  const b = await book(fx, await user());
  if (!b.bookingId) return report("A2", false, `could not create a booking: ${b.status} ${JSON.stringify(b.json).slice(0, 200)}`);
  const row = await bookingRow(b.bookingId);
  const ttl = (new Date(row.holdExpiresAt) - new Date(row.createdAt)) / 1000;
  const ttlOk = Math.abs(ttl - Number(E.HOLD_TTL_SECONDS || 600)) <= 3;
  let minSold = Infinity;
  const released = await waitFor(async () => {
    const s = await sold(fx); minSold = Math.min(minSold, s);
    const st = (await bookingRow(b.bookingId)).status;
    return st !== "HELD" && s === 0 ? st : null;
  }, 75000);
  const finalSold = await sold(fx);
  report("A2", ttlOk && !!released && finalSold === 0 && minSold >= 0,
    `hold lasted ${ttl.toFixed(1)}s (want ${E.HOLD_TTL_SECONDS}), status after 75s: ${(await bookingRow(b.bookingId)).status}, soldCount ${finalSold} (min seen ${minSold}; two workers were running)`);
}

// ---------------------------------------------------------------- A3: payment.captured after the hold expired
async function a3() {
  if (!E.RAZORPAY_WEBHOOK_SECRET) return report("A3", null, "RAZORPAY_WEBHOOK_SECRET not set");
  // a) seat still free: confirm
  const fa = await fixture({ quantity: 1 });
  const ua = await user();
  const ba = await book(fa, ua);
  if (!ba.bookingId) return report("A3", false, `could not create a booking: ${ba.status}`);
  const rowA = await bookingRow(ba.bookingId);
  const payA = await payFor(ba.bookingId, rowA);
  const howA = await expire(fa, ba.bookingId);
  const wa = await sendWebhook(webhookBody(rowA, payA), rid("evt"));
  const paidA = await waitFor(async () => (await bookingRow(ba.bookingId)).status === "PAID", 20000);
  const soldA = await sold(fa);
  const okA = !!paidA && soldA === 1;
  // b) seat taken by someone else meanwhile: never confirm, mark for refund
  const fb = await fixture({ quantity: 1 });
  const ub = await user();
  const bb = await book(fb, ub);
  const rowB = await bookingRow(bb.bookingId);
  const payB = await payFor(bb.bookingId, rowB);
  const howB = await expire(fb, bb.bookingId);
  const other = await book(fb, await user());
  const wb = await sendWebhook(webhookBody(rowB, payB), rid("evt"));
  await sleep(15000);
  const stB = (await bookingRow(bb.bookingId)).status;
  const soldB = await sold(fb);
  const refundRows = await mentions("Refund", [payB.paymentRowId, payB.razorpayPaymentId, bb.bookingId]);
  const payRow = await one(`SELECT row_to_json(p)::text AS j, status FROM "Payment" p WHERE "paymentId" = $1`, [payB.paymentRowId]);
  const refundMarked = (refundRows || 0) > 0 || /refund/i.test(payRow?.j || "") || payRow?.status === "REFUNDED";
  const okB = stB !== "PAID" && soldB <= 1 && !!other.bookingId && refundMarked;
  report("A3", okA && okB,
    `a) free seat: webhook ${wa.status}, booking ${(await bookingRow(ba.bookingId)).status}, soldCount ${soldA} (expiry ${howA}) | ` +
    `b) seat taken: webhook ${wb.status}, late booking ${stB}, other buyer ${other.bookingId ? "holds the seat" : "could not book: " + other.status}, soldCount ${soldB}, refund marked ${refundMarked} (expiry ${howB})`);
}

// ---------------------------------------------------------------- A4: duplicates and a racing verify give exactly one of everything
const SIDE_TABLES = ["Payment", "Ticket", "LedgerEntry", "Payout", "VenuePayout", "Notification", "Refund", "ProcessedWebhookEvent"];
async function sideEffects(b, pay) {
  const out = {};
  for (const t of SIDE_TABLES) out[t] = await mentions(t, [b.bookingId, pay.paymentRowId, pay.razorpayPaymentId, pay.orderId]);
  return out;
}
async function a4() {
  if (!E.RAZORPAY_WEBHOOK_SECRET) return report("A4", null, "RAZORPAY_WEBHOOK_SECRET not set");
  async function prepared() {
    const fx = await fixture({ quantity: 2 });
    const u = await user();
    const b = await book(fx, u);
    await db.query(`UPDATE "Booking" SET "holdExpiresAt" = now() + interval '1 hour' WHERE "bookingId" = $1`, [b.bookingId]);
    const row = await bookingRow(b.bookingId);
    return { fx, u, row, pay: await payFor(b.bookingId, row) };
  }
  // control: one clean delivery, to learn what "exactly once" looks like in this codebase
  const c = await prepared();
  await sendWebhook(webhookBody(c.row, c.pay), rid("evt"));
  await waitFor(async () => (await bookingRow(c.row.bookingId)).status === "PAID", 20000);
  await sleep(3000);
  const ctrl = await sideEffects(c.row, c.pay);
  // race: the same event twice, a second event for the same payment, and the client's verify call, all at once
  const r = await prepared();
  const body = webhookBody(r.row, r.pay);
  const evt = rid("evt");
  const calls = [sendWebhook(body, evt), sendWebhook(body, evt), sendWebhook(webhookBody(r.row, r.pay, "order.paid"), rid("evt"))];
  if (E.RAZORPAY_KEY_SECRET) {
    const sig = crypto.createHmac("sha256", E.RAZORPAY_KEY_SECRET).update(`${r.pay.orderId}|${r.pay.razorpayPaymentId}`).digest("hex");
    calls.push(api("POST", `/bookings/${r.row.bookingId}/verify-payment`, r.u.token,
      { razorpay_order_id: r.pay.orderId, razorpay_payment_id: r.pay.razorpayPaymentId, razorpay_signature: sig }));
  }
  const codes = (await Promise.all(calls)).map((x) => x.status).join("/");
  await waitFor(async () => (await bookingRow(r.row.bookingId)).status === "PAID", 20000);
  await sleep(5000);
  const race = await sideEffects(r.row, r.pay);
  const st = (await bookingRow(r.row.bookingId)).status;
  const ctrlSt = (await bookingRow(c.row.bookingId)).status;
  const diffs = SIDE_TABLES.filter((t) => race[t] !== null && ctrl[t] !== null && race[t] > ctrl[t]);
  report("A4", st === "PAID" && ctrlSt === "PAID" && diffs.length === 0 && (await sold(r.fx)) === 1,
    `control booking ${ctrlSt}, raced booking ${st}, calls ${codes}, soldCount ${await sold(r.fx)}; ` +
    (diffs.length ? `MORE rows than one clean delivery in: ${diffs.map((t) => `${t} ${race[t]} vs ${ctrl[t]}`).join(", ")}` : "same row counts as one clean delivery"));
}

// ---------------------------------------------------------------- A5: server-side price, integer paise at Razorpay
async function a5() {
  const fx = await fixture({ price: 499.99, quantity: 5 });
  const u = await user();
  const b = await book(fx, u, 1, { total: 1, subtotal: 1, amount: 1, item: { unitPrice: 1, price: 1 } });
  if (!b.bookingId) return report("A5", false, `could not create a booking: ${b.status} ${JSON.stringify(b.json).slice(0, 200)}`);
  const row = await bookingRow(b.bookingId);
  const priceOk = Math.abs(Number(row.total) - 499.99) < 0.005;
  const o = await api("POST", `/bookings/${b.bookingId}/payment-order`, u.token, {});
  const orderId = find(o.json, "orderId") || find(o.json, "razorpayOrderId") || find(o.json, "id");
  let orderAmount = null, how = "";
  if (o.status >= 200 && o.status < 300 && orderId && E.RAZORPAY_KEY_ID && E.RAZORPAY_KEY_SECRET) {
    try {
      const res = await fetch(`https://api.razorpay.com/v1/orders/${orderId}`, {
        signal: AbortSignal.timeout(20000),
        headers: { authorization: "Basic " + Buffer.from(`${E.RAZORPAY_KEY_ID}:${E.RAZORPAY_KEY_SECRET}`).toString("base64") },
      });
      if (res.ok) { orderAmount = (await res.json()).amount; how = "Razorpay's own record of the order"; }
    } catch {}
  }
  if (orderAmount === null && o.status >= 200 && o.status < 300) { orderAmount = find(o.json, "amount"); how = "the payment-order response"; }
  if (o.status < 200 || o.status >= 300)
    return report("A5", priceOk ? null : false, `server price ${priceOk ? "ok" : "WRONG"} (booking total ${row.total}); payment-order failed: ${o.status} ${JSON.stringify(o.json).slice(0, 160)} (Razorpay test keys missing or offline?)`);
  report("A5", priceOk && orderAmount === 49999,
    `booking total ${row.total} for a ₹499.99 ticket with a tampered client price; order amount ${orderAmount} paise from ${how} (want 49999)`);
}

(async () => {
  await db.connect();
  for (const [id, fn] of [["A1", a1], ["A2", a2], ["A3", a3], ["A4", a4], ["A5", a5]]) {
    try { await fn(); } catch (e) { report(id, false, `test crashed: ${String(e).slice(0, 300)}`); }
  }
  await db.end();
  console.log(`\n${results.filter((r) => r === true).length} pass, ${results.filter((r) => r === false).length} fail, ${results.filter((r) => r === null).length} skip`);
  process.exit(results.includes(false) ? 1 : 0);
})();
