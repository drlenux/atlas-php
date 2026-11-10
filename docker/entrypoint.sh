#!/usr/bin/env bash
set -Eeuo pipefail

postgres_pid=""
app_pid=""
cleanup() {
  trap - TERM INT EXIT
  if [[ -n "$app_pid" ]]; then kill -TERM "$app_pid" 2>/dev/null || true; fi
  if [[ -n "$postgres_pid" ]]; then kill -INT "$postgres_pid" 2>/dev/null || true; fi
  if [[ -n "$app_pid" ]]; then wait "$app_pid" 2>/dev/null || true; fi
  if [[ -n "$postgres_pid" ]]; then wait "$postgres_pid" 2>/dev/null || true; fi
}
trap cleanup EXIT
trap 'exit 143' TERM
trap 'exit 130' INT

/usr/local/bin/docker-entrypoint.sh postgres -c listen_addresses=127.0.0.1 &
postgres_pid=$!
for attempt in $(seq 1 120); do
  if ! kill -0 "$postgres_pid" 2>/dev/null; then
    echo "PostgreSQL exited during startup" >&2
    exit 1
  fi
  # Initialization's temporary server has no TCP listener, so this waits for the final server.
  if pg_isready -h 127.0.0.1 -U "$POSTGRES_USER" -d "$POSTGRES_DB" >/dev/null 2>&1; then break; fi
  sleep 1
done
pg_isready -h 127.0.0.1 -U "$POSTGRES_USER" -d "$POSTGRES_DB" >/dev/null
psql -h 127.0.0.1 -U "$POSTGRES_USER" -d "$POSTGRES_DB" -v ON_ERROR_STOP=1 -c 'CREATE EXTENSION IF NOT EXISTS vector;' >/dev/null

if [[ $# -eq 0 ]]; then set -- serve; fi
gosu postgres /usr/local/bin/code-atlas "$@" &
app_pid=$!
set +e
wait -n "$postgres_pid" "$app_pid"
result=$?
set -e
exit "$result"
