import postgres from "postgres";
import { clean } from "@/lib/telemetry";

// Where the opt-in usage records from `zerd telemetry on` land.
//   POST /api/telemetry   one record per finished request, from the zerd binary
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
      id bigserial primary key,
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
  if (raw.length > 8000) return Response.json({ error: "too large" }, { status: 413 });
  let body: unknown;
  try {
    body = JSON.parse(raw);
  } catch {
    return Response.json({ error: "not json" }, { status: 400 });
  }
  const rec = clean(body);
  if (!rec) return Response.json({ error: "not a zerd telemetry record" }, { status: 400 });
  await table();
  await sql`insert into telemetry_events (install, zerd, model, status, data)
            values (${rec.install as string}, ${(rec.zerd as string) ?? null}, ${(rec.model as string) ?? null}, ${(rec.status as string) ?? null}, ${sql.json(rec)})`;
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
           round(avg((data->>'sec')::numeric)) as avg_sec
    from telemetry_events
    where at > now() - interval '30 days'
    group by 1, 2, 3
    order by requests desc
    limit 200`;
  return Response.json({ days: 30, rows });
}
