import postgres from "postgres";
import { clean } from "@/lib/telemetry";

// Where the opt-in usage records from `zerd telemetry on` land.
//   POST /api/telemetry   a batch of records (one per finished request) from the zerd binary
//   GET  /api/telemetry   totals for the last 30 days, for us (needs the admin key)
//
// Needs DATABASE_URL (any Postgres) and, for the GET, TELEMETRY_ADMIN_KEY. Without a database the POST answers 503
// and zerd simply drops the record. The caller's IP address is never read or stored.

const sql = process.env.DATABASE_URL ? postgres(process.env.DATABASE_URL, { max: 1, idle_timeout: 20 }) : null;

// The table is made on the first request after a cold start, so there is no migration step to forget.
let ready: Promise<unknown> | null = null;
function table() {
  ready ??= sql!`
    create table if not exists telemetry_events (
      rid uuid primary key,
      at timestamptz not null default now(),
      install text not null,
      zerd text,
      model text,
      status text,
      data jsonb not null
    )`;
  return ready;
}

export async function POST(request: Request) {
  if (!sql) return Response.json({ error: "telemetry storage is not set up" }, { status: 503 });
  // ponytail: no rate limit, only a size cap and a strict shape. Add one per install id if someone floods it.
  const raw = await request.text();
  if (raw.length > 300_000) return Response.json({ error: "too large" }, { status: 413 });
  let body: unknown;
  try {
    body = JSON.parse(raw);
  } catch {
    return Response.json({ error: "not json" }, { status: 400 });
  }
  // zerd sends an array: everything that was waiting on that machine, usually one record, more after time offline
  const recs = (Array.isArray(body) ? body : [body]).slice(0, 200).map(clean).filter((r) => r !== null);
  if (recs.length === 0) return Response.json({ error: "no zerd telemetry record in the body" }, { status: 400 });
  await table();
  const rows = recs.map((r) => ({ rid: r.rid as string, install: r.install as string, zerd: (r.zerd as string) ?? null, model: (r.model as string) ?? null, status: (r.status as string) ?? null, data: sql.json(r) }));
  // one statement for the whole batch; a record we already have (a retried send) is skipped
  await sql`insert into telemetry_events ${sql(rows)} on conflict (rid) do nothing`;
  return new Response(null, { status: 204 });
}

export async function GET(request: Request) {
  const key = process.env.TELEMETRY_ADMIN_KEY;
  if (!sql || !key || request.headers.get("authorization") !== `Bearer ${key}`) return Response.json({ error: "not found" }, { status: 404 });
  await table();
  // one row per model and outcome: how often it happens, how many rounds and tokens it takes, how often it gets cut off
  const rows = await sql`
    select model, status, data->>'stop' as stop, count(*)::int as requests, count(distinct install)::int as installs,
           round(avg((data->>'rounds')::numeric), 1) as avg_rounds,
           round(avg((data->>'prompt_tokens')::numeric)) as avg_prompt_tokens,
           round(avg((data->>'peak_prompt')::numeric)) as avg_peak_prompt,
           round(avg((data->>'sec')::numeric)) as avg_sec
    from telemetry_events
    where at > now() - interval '30 days'
    group by 1, 2, 3
    order by requests desc
    limit 200`;
  return Response.json({ days: 30, rows });
}
