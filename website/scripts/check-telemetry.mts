// Run with: node --experimental-strip-types scripts/check-telemetry.mts
// Fails if the telemetry endpoint would store anything beyond the agreed counts and names.
import { clean } from "../lib/telemetry.ts";
import assert from "node:assert";
const id = "3f0c2b1a-1111-4222-8333-444455556666";
const r = clean({ v: 1, install: id, status: "verified", model: "x".repeat(500), rounds: 4.4, note: "free text", tools: { edit: [30, 2], "Bad Name": [1, 1], shell: ["a", 1] }, nudges: { edit_churn: 2 }, nested: { a: 1 } })!;
assert.deepStrictEqual(r, { install: id, status: "verified", model: "x".repeat(80), v: 1, rounds: 4, tools: { edit: [30, 2] }, nudges: { edit_churn: 2 } });
assert.equal(clean({ v: 2, install: id }), null);
assert.equal(clean({ v: 1, install: "me@example.com" }), null);
assert.equal(clean("hi"), null);
console.log("clean() ok");
