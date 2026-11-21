#!/bin/sh
# Hidden test for h1-checkout-integrity: the API plus TWO workers (A2 must be safe with two), then h1.cjs.
HERE="$(cd "$(dirname "$0")" && pwd)"
LIB="$HERE/../../_lib"
export HOLD_TTL_SECONDS=5
export H1_JS="$HERE/h1.cjs"
sh "$LIB/with_server.sh" sh -c '
  W="${TMPDIR:-/tmp}/h1-worker-$API_PORT"
  ( cd apps/server && exec node -r ts-node/register/transpile-only src/workers/index.worker.ts ) >"$W-1.log" 2>&1 & W1=$!
  ( cd apps/server && exec node -r ts-node/register/transpile-only src/workers/index.worker.ts ) >"$W-2.log" 2>&1 & W2=$!
  sleep 15
  kill -0 $W1 2>/dev/null || { echo "worker 1 did not stay up:"; tail -20 "$W-1.log"; }
  node "$H1_JS"; rc=$?
  kill $W1 $W2 2>/dev/null
  exit $rc
'
