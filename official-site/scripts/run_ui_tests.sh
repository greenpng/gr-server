#!/usr/bin/env bash
# Start API + production web, run UI tests, then stop background jobs.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
API_URL="${GV6_OFFICIAL_PUBLIC_URL:-http://127.0.0.1:4101}"
WEB_URL="${GV6_MARKETING_URL:-http://127.0.0.1:3000}"
DB_URL="${GV6_DATABASE_URL:-postgresql://gv6:gv6@127.0.0.1:5432/gv6_official}"

cleanup() {
  [[ -n "${API_PID:-}" ]] && kill "$API_PID" 2>/dev/null || true
  [[ -n "${WEB_PID:-}" ]] && kill "$WEB_PID" 2>/dev/null || true
}
trap cleanup EXIT

wait_http() {
  local url=$1 max=${2:-60}
  for i in $(seq 1 "$max"); do
    if curl -sf "$url" >/dev/null 2>&1; then return 0; fi
    sleep 1
  done
  echo "timeout waiting for $url" >&2
  return 1
}

echo "== UI test stack =="

if ! curl -sf "${API_URL}/v1/health" >/dev/null 2>&1; then
  echo "Starting API on 4101..."
  (cd "$ROOT/official-site" && GV6_DATABASE_URL="$DB_URL" \
    GV6_OFFICIAL_WEB_ORIGIN="$WEB_URL" GV6_OFFICIAL_LAB_EMAIL=1 \
    node server/src/index.js) &
  API_PID=$!
  wait_http "${API_URL}/v1/health"
else
  echo "API already up"
fi

if ! curl -sf "$WEB_URL/" >/dev/null 2>&1; then
  echo "Building & starting official-web on 3000..."
  (cd "$ROOT/official-web" && npm run build && GV6_OFFICIAL_API_URL="$API_URL" npm run start) &
  WEB_PID=$!
  wait_http "$WEB_URL/"
else
  echo "Web already up"
fi

echo "== responsive (quick) =="
(cd "$ROOT/official-web" && UI_QUICK=1 GV6_OFFICIAL_API_URL="$API_URL" npm run test:responsive -- "$WEB_URL")

echo "== lighthouse (core pages) =="
(cd "$ROOT/official-web" && npm run test:lighthouse -- "$WEB_URL")

echo "UI_TESTS_PASS"
