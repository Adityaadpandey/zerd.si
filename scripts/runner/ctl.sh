#!/usr/bin/env bash
# tunyt-ctl: start, stop and restart the dev processes of ONE worktree.
#
#   tunyt-ctl status
#   tunyt-ctl restart worker      # worker has no hot reload, any change there needs this
#   tunyt-ctl restart api         # only when ts-node-dev crashed or got stuck
#   tunyt-ctl ensure              # restart whatever died (hot reload crashes now and then)
#   tunyt-ctl logs web            # last 80 lines
#
# Only the parts named in the worktree's .zerd-scope file are allowed to run.
# Example: scope "web" means api and worker are refused, so nothing runs that the task does not need.
#
# Postgres and Redis are NOT started here. They are shared and already up (docker compose).

set -u

WT="${ZERD_WT:-$(git rev-parse --show-toplevel 2>/dev/null || pwd)}"
RUN="$WT/.run"
API_PORT="${API_PORT:-7001}"
WEB_PORT="${WEB_PORT:-3000}"
mkdir -p "$RUN"

# .zerd-scope holds words like "server", "web" or "server web"; server means api + worker
scope_has() {
  local s; s="$(cat "$WT/.zerd-scope" 2>/dev/null || echo "server web")"
  case "$1" in
    api|worker) [[ " $s " == *" server "* ]] ;;
    web)        [[ " $s " == *" web "* ]] ;;
  esac
}

alive() { [[ -f "$RUN/$1.pid" ]] && kill -0 "$(cat "$RUN/$1.pid")" 2>/dev/null; }

start_one() {
  local c="$1"
  scope_has "$c" || { echo "$c is outside this task's scope ($(cat "$WT/.zerd-scope" 2>/dev/null)), not starting it"; return 1; }
  alive "$c" && { echo "$c already running"; return 0; }
  case "$c" in
    api)    cd "$WT/apps/server" && PORT="$API_PORT" setsid nohup pnpm dev                       >"$RUN/api.log" 2>&1 & ;;
    worker) cd "$WT/apps/server" && setsid nohup pnpm run:worker                                 >"$RUN/worker.log" 2>&1 & ;;
    web)    cd "$WT/apps/web"    && setsid nohup pnpm exec next dev -p "$WEB_PORT" -H 0.0.0.0    >"$RUN/web.log" 2>&1 & ;;
    *) echo "unknown part: $c (api, worker, web)"; return 1 ;;
  esac
  echo $! >"$RUN/$c.pid"
  echo "started $c"
}

# kill the whole process group, pnpm spawns children that a plain kill would leave behind
stop_one() {
  alive "$1" && kill -- "-$(cat "$RUN/$1.pid")" 2>/dev/null
  rm -f "$RUN/$1.pid"
  echo "stopped $1"
}

parts() { [[ "${1:-all}" == all ]] && echo api worker web || echo "$1"; }

cmd="${1:-status}"; target="${2:-all}"
case "$cmd" in
  start)   for c in $(parts "$target"); do scope_has "$c" && start_one "$c"; done ;;
  stop)    for c in $(parts "$target"); do stop_one "$c"; done ;;
  restart) for c in $(parts "$target"); do stop_one "$c"; scope_has "$c" && start_one "$c"; done ;;
  ensure)  for c in api worker web; do scope_has "$c" && ! alive "$c" && { echo "$c was down"; start_one "$c"; }; done ;;
  status)  for c in api worker web; do scope_has "$c" && { alive "$c" && echo "$c up" || echo "$c DOWN"; }; done ;;
  logs)    tail -n 80 "$RUN/${target/all/api}.log" ;;
  *) echo "usage: tunyt-ctl start|stop|restart|ensure|status|logs [api|worker|web|all]"; exit 2 ;;
esac
