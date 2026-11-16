// What zerd may send us, and nothing else. A record that carries an unknown field, a long string or a nested
// object is cut down to this shape before it is stored, so a modified client cannot park free text in our database.
//
// The sending side is src/util/telemetry.rs in the zerd repo; the two lists have to stay in step.

const TEXT = ["install", "zerd", "os", "arch", "model", "intent", "status", "stop"] as const;
const NUMBERS = ["v", "rounds", "estimate", "sec", "calls", "prompt_tokens", "cached_tokens", "completion_tokens", "changed", "repairs", "checks", "checks_failed", "compactions", "prunes"] as const;

export type TelemetryRecord = Record<string, string | number | Record<string, number | number[]>>;

const count = (x: unknown) => (typeof x === "number" && Number.isFinite(x) && x >= 0 ? Math.round(x) : null);
const name = (k: string) => /^[a-z][a-z0-9_]{0,39}$/.test(k);

// Returns the cleaned record, or null when the body is not a zerd record at all.
// Example: { v: 1, install: "3f0c…", status: "verified", tools: { edit: [30, 2] }, note: "hi" } comes back without `note`.
export function clean(body: unknown): TelemetryRecord | null {
  if (!body || typeof body !== "object" || Array.isArray(body)) return null;
  const b = body as Record<string, unknown>;
  if (b.v !== 1 || typeof b.install !== "string" || !/^[0-9a-f-]{36}$/.test(b.install)) return null;
  const out: TelemetryRecord = {};
  for (const k of TEXT) if (typeof b[k] === "string") out[k] = (b[k] as string).slice(0, 80);
  for (const k of NUMBERS) {
    const n = count(b[k]);
    if (n !== null) out[k] = n;
  }
  // tools: { name: [calls, errors] }, nudges: { kind: times }
  for (const [field, width] of [["tools", 2], ["nudges", 1]] as const) {
    const src = b[field];
    if (!src || typeof src !== "object" || Array.isArray(src)) continue;
    const kept: Record<string, number | number[]> = {};
    for (const [k, v] of Object.entries(src).slice(0, 40)) {
      if (!name(k)) continue;
      const nums = (Array.isArray(v) ? v : [v]).slice(0, width).map(count);
      if (nums.length === width && nums.every((n) => n !== null)) kept[k] = width === 1 ? (nums[0] as number) : (nums as number[]);
    }
    out[field] = kept;
  }
  return out;
}
