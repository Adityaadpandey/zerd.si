#!/bin/sh
# Start the tunyt API from the current worktree, wait until it answers, run "$@", stop it.
# Same command the runner uses (scripts/runner/ctl.sh): cd apps/server && PORT=$API_PORT pnpm dev
# Usage inside a hidden test:  sh "$LIB/with_server.sh" sh -c 'curl ...'
PORT="${API_PORT:-7001}"
LOG="${TMPDIR:-/tmp}/bench-api-$$.log"
lsof -ti tcp:"$PORT" | xargs kill 2>/dev/null
( cd apps/server && PORT="$PORT" exec pnpm dev ) >"$LOG" 2>&1 &
pid=$!
i=0
until curl -s -o /dev/null "http://localhost:$PORT/"; do
  i=$((i + 1))
  if [ "$i" -gt 150 ]; then echo "API did not start in 150s"; tail -40 "$LOG"; kill "$pid" 2>/dev/null; exit 2; fi
  sleep 1
done
export API="http://localhost:$PORT"
"$@"
rc=$?
lsof -ti tcp:"$PORT" | xargs kill 2>/dev/null
kill "$pid" 2>/dev/null
exit $rc
