#!/usr/bin/env bash
# greenpng results SDK (shell) — backend only.
#
# Thin, read-only client: gr_get_result / gr_wait_for_result / gr_query.
# Does NOT collect or relay browser probes.
#
# Auth: X-Gr-Sdk-Key with the site-scoped backend key.
# Projection: "public" | "sdk" | "diagnostic" (diagnostic needs an ops key).
#
# Requires: bash + curl. Emits raw result JSON on stdout (or error JSON),
# exits 1 on transport/HTTP errors, 2 on wait timeout.

set -u

: "${GR_CURL:=curl}"
: "${GR_WAIT_INTERVAL_MS:=250}"

# gr_get_result <session_id> [projection] [strategy_id] [response_profile] [lang] [profile_cap] — one snapshot.
gr_get_result() {
  local session_id="$1"
  local projection="${2:-sdk}"
  local strategy_id="${3:-}"
  local response_profile="${4:-}"
  local lang="${5:-}"
  local profile_cap="${6:-}"
  local url="${GR_BASE_URL%/}/v1/session/${session_id}/result"
  local -a query=(--data-urlencode "projection=${projection}")
  [ -n "$strategy_id" ] && query+=(--data-urlencode "strategy_id=${strategy_id}")
  [ -n "$response_profile" ] && query+=(--data-urlencode "response_profile=${response_profile}")
  [ -n "$lang" ] && query+=(--data-urlencode "lang=${lang}")
  [ -n "$profile_cap" ] && query+=(--data-urlencode "profile_cap=${profile_cap}")
  "$GR_CURL" -fsS \
    -H "Accept: application/json" \
    -H "X-Gr-Sdk-Key: ${GR_API_KEY:?set GR_API_KEY (or GR_SITE_RESULT_KEY)}" \
    -H "X-Request-Id: req_$(date +%s%3N)" \
    --max-time "${GR_TIMEOUT_MS:-15000}" \
    -G "${query[@]}" \
    "$url"
}

# gr_wait_for_result <session_id> [projection] [timeout_ms] [strategy_id] [response_profile] [lang] [profile_cap]
# Polls until product_public/sdk_projection present and analysis_pending
# is not true. Exit 2 on timeout.
gr_wait_for_result() {
  local session_id="$1"
  local projection="${2:-sdk}"
  local timeout_ms="${3:-8000}"
  local strategy_id="${4:-}"
  local response_profile="${5:-}"
  local lang="${6:-}"
  local profile_cap="${7:-}"
  local deadline_ms start now rc body last
  start=$(date +%s%3N)
  deadline_ms=$((start + timeout_ms))
  while : ; do
    now=$(date +%s%3N)
    if [ "$now" -ge "$deadline_ms" ]; then
      printf '{"error":"analysis_pending"}\n' >&2
      exit 2
    fi
    rc=0
    body=$(gr_get_result "$session_id" "$projection" "$strategy_id" "$response_profile" "$lang" "$profile_cap") || rc=$?
    if [ "${rc:-0}" -eq 0 ] && [ -n "$body" ]; then
      case "$body" in
        *'"analysis_pending"'*true*|*'"ok":false'*)
          # still pending — keep polling
          ;;
        *)
          printf '%s\n' "$body"
          return 0
          ;;
      esac
    fi
    last="$body"
    sleep "${GR_WAIT_INTERVAL_MS}e-3" 2>/dev/null || sleep 0.25
  done
}

# gr_query <session_id> [projection] [wait=0|1] [timeout_ms] [strategy_id] [response_profile] [lang] [profile_cap]
# wait=0 → single fetch (same as gr_get_result).
gr_query() {
  local session_id="$1"
  local projection="${2:-sdk}"
  local wait="${3:-0}"
  local timeout_ms="${4:-8000}"
  local strategy_id="${5:-}"
  local response_profile="${6:-}"
  local lang="${7:-}"
  local profile_cap="${8:-}"
  if [ "$wait" = "1" ]; then
    gr_wait_for_result "$session_id" "$projection" "$timeout_ms" "$strategy_id" "$response_profile" "$lang" "$profile_cap"
  else
    gr_get_result "$session_id" "$projection" "$strategy_id" "$response_profile" "$lang" "$profile_cap"
  fi
}

# gr_cookie_fields <result_json> — prints the cookie_fields object (or
# "null") from sdk_projection/product_public. Requires a JSON tool; falls
# back to grep on cookie_fields when tr/jq are missing.
gr_cookie_fields() {
  local result="$1"
  local got
  if command -v jq >/dev/null 2>&1; then
    got=$(printf '%s' "$result" | \
      jq -r '.sdk_projection.cookie_fields // .product_public.cookie_fields // .cookie_fields // "null"' 2>/dev/null)
    printf '%s\n' "${got:-null}"
    return 0
  fi
  got=$(printf '%s' "$result" | grep -o '"cookie_fields":{[^}]*}' | head -n1)
  if [ -n "$got" ]; then
    printf '%s\n' "${got#*:}"
  else
    printf 'null\n'
  fi
}

# Optional env alias: GR_SITE_RESULT_KEY.
if [ -z "${GR_API_KEY:-}" ] && [ -n "${GR_SITE_RESULT_KEY:-}" ]; then
  GR_API_KEY="$GR_SITE_RESULT_KEY"
fi

if [ "${BASH_SOURCE[0]}" = "$0" ] && [ "$#" -gt 0 ]; then
  cmd="$1"; shift
  case "$cmd" in
    get) gr_get_result "$@" ;;
    wait) gr_wait_for_result "$@" ;;
    query) gr_query "$@" ;;
    cookie-fields) gr_cookie_fields "${1:-$(cat)}" ;;
    *) printf 'usage: gr_sdk.sh <get|wait|query> [args...]\n' >&2; exit 64 ;;
  esac
fi
