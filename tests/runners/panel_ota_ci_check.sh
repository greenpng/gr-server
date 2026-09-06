#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
BASE="${GR_ADMIN_BASE:-${GR_ADMIN_BASE:-http://127.0.0.1:29680}}"
SECRETS="${GR_ADMIN_SECRETS:-$ROOT/03-local-test-lab/tests/data/lab-local/admin/admin_bootstrap_once.txt}"
RELEASE_BASE="${GR_RELEASE_BASE:-${GR_RELEASE_BASE:?GR_RELEASE_BASE or GR_RELEASE_BASE is required}}"
INSTALL_ROOT="${GR_INSTALL_ROOT:-${GR_INSTALL_ROOT:?GR_INSTALL_ROOT or GR_INSTALL_ROOT is required}}"
OUT="${REPORT_PATH:-$ROOT/03-local-test-lab/tests/reports/panel_ota_ci.json}"
COOKIE="$(mktemp)"
trap 'rm -f "$COOKIE"' EXIT

read_secret() {
  local key="$1"
  sed -n "s/^${key}=//p" "$SECRETS" | head -1
}

[[ -r "$SECRETS" ]] || { echo "missing admin secrets: $SECRETS" >&2; exit 1; }
USER="$(read_secret username)"
PASS="$(read_secret password)"
CONSOLE="$(read_secret console_path)"
[[ -n "$CONSOLE" ]] || CONSOLE="$(read_secret console)"
CONSOLE="${CONSOLE#/}"
CONSOLE="${CONSOLE%/}"
[[ -n "$USER" && -n "$PASS" && -n "$CONSOLE" ]] || {
  echo "incomplete admin secrets: $SECRETS" >&2
  exit 1
}

API="${BASE%/}/${CONSOLE}/api"
report='[]'
record() {
  local name="$1" ok="$2" detail="${3:-}"
  report="$(python3 - "$report" "$name" "$ok" "$detail" <<'PY'
import json, sys
items = json.loads(sys.argv[1])
items.append({"step": sys.argv[2], "ok": sys.argv[3] == "true", "detail": sys.argv[4]})
print(json.dumps(items))
PY
)"
}
call() {
  local method="$1" path="$2" body="${3:-}"
  if [[ -n "$body" ]]; then
    curl -fsS -b "$COOKIE" -c "$COOKIE" -H 'content-type: application/json' \
      -X "$method" "$API/$path" -d "$body"
  else
    curl -fsS -b "$COOKIE" -c "$COOKIE" -X "$method" "$API/$path"
  fi
}
json_ok() {
  python3 -c 'import json,sys; d=json.load(sys.stdin); assert d.get("ok") is True, d; print(json.dumps(d))'
}

LOGIN="$(curl -fsS -b "$COOKIE" -c "$COOKIE" -H 'content-type: application/json' \
  -X POST "$API/login" \
  -d "$(python3 - "$USER" "$PASS" <<'PY'
import json, sys
print(json.dumps({"username": sys.argv[1], "password": sys.argv[2]}))
PY
)")"
echo "$LOGIN" | json_ok >/dev/null
record "panel_login" true "$CONSOLE"

SET="$(call POST ota/set-release-url "$(python3 - "$RELEASE_BASE" <<'PY'
import json, sys
print(json.dumps({"url": sys.argv[1]}))
PY
)")"
echo "$SET" | json_ok >/dev/null
record "panel_set_release_url" true "$RELEASE_BASE"

REMOTE="$(call GET ota/remote)"
echo "$REMOTE" | json_ok >/dev/null
record "panel_fetch_remote_manifest" true

FE="$(call POST ota/install-fe '{}')"
echo "$FE" | json_ok >/dev/null
record "panel_install_fe" true
test "$(cat "$INSTALL_ROOT/fe/VERSION")" = "${OTA_VERSION:?OTA_VERSION is required}"
record "fe_version_written" true "$OTA_VERSION"

RUNTIME="$(call POST ota/install-runtime '{"restart":false}')"
echo "$RUNTIME" | json_ok >/dev/null
python3 - "$RUNTIME" <<'PY'
import json, sys
d = json.loads(sys.argv[1])
assert d.get("sha_verified") is True, d
assert d.get("restart_requested") is False, d
PY
record "panel_install_runtime" true
test -x "$INSTALL_ROOT/bin/gr-service"
test "$(cat "$INSTALL_ROOT/VERSION")" = "$OTA_VERSION"
record "runtime_version_written" true "$OTA_VERSION"

HEALTH="$(curl -fsS "$BASE/v1/health")"
echo "$HEALTH" >/dev/null
record "health_after_panel_ota" true

mkdir -p "$(dirname "$OUT")"
python3 - "$OUT" "$report" <<'PY'
import json, sys
out, raw = sys.argv[1:]
items = json.loads(raw)
doc = {"ok": all(x["ok"] for x in items), "pass": sum(x["ok"] for x in items),
       "fail": sum(not x["ok"] for x in items), "results": items}
open(out, "w").write(json.dumps(doc, indent=2) + "\n")
print(json.dumps(doc, indent=2))
raise SystemExit(0 if doc["ok"] else 1)
PY
