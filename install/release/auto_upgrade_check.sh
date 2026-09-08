#!/usr/bin/env bash
# greenpng unattended cold-part auto-upgrade — gate runner (L3).
# iss/ota-unattended-auto-upgrade-design: the ONLY privileged step of the
# unattended path. Runs as root via greenpng-auto-upgrade.timer (or manually).
#
# Gate order (every skip is a clean exit — the timer just tries again):
#   1. flock            — no overlap with a previous run
#   2. hold latch       — panel `POST {console}/api/ota/auto-hold` or an
#                         automatic rollback latch; expires after 24h
#   3. desired          — PG admin_settings.cluster_ota_desired with
#                         auto_apply=true (panel checkbox)
#   4. monotonic        — target version strictly newer than $PREFIX/VERSION
#                         (never downgrade; same-version redeploy stays manual)
#   5. window           — desired.window "HH:MM-HH:MM" (local, cross-midnight
#                         ok, comma list ok, empty = unrestricted)
# Then runs update_runtime_from_github.sh VERSION=<target> — pinned root key →
# bundle sha → manifest root signature → per-asset sha, staged slot + backup,
# ownership correction back to the service user, restart with health gate and
# AUTOMATIC ROLLBACK. All of that logic lives in the updater; this wrapper
# never reimplements it.
#
# Result state → $PREFIX/data/auto_upgrade_state.json (service-user owned) →
# panel `GET {console}/api/ota/auto-state`.
#
# Env:
#   GR_PREFIX     default /opt/greenpng
#   GR_UPDATER    default $PREFIX/sbin/update_runtime_from_github.sh
#   GR_RAW_BASE   bootstrap source for the updater when missing (repo main)
#   GR_RELEASE_REPO  default greenpng/gr-server (updater default too)
#   GR_AUTO_LOCK  flock path override (sandbox tests)
# Deliberately NOT `set -e`: gate outcomes are controlled exits, and the
# updater's non-zero exit must be captured (→ rolled_back latch), not abort.
set -u -o pipefail

PREFIX="${GR_PREFIX:-/opt/greenpng}"
ENVF="$PREFIX/.env"
STATE="$PREFIX/data/auto_upgrade_state.json"
UPDATER="${GR_UPDATER:-$PREFIX/sbin/update_runtime_from_github.sh}"
RAW_BASE="${GR_RAW_BASE:-https://raw.githubusercontent.com/greenpng/gr-server/main}"
RELEASE_REPO="${GR_RELEASE_REPO:-greenpng/gr-server}"
LOCK="${GR_AUTO_LOCK:-/run/greenpng-auto-upgrade.lock}"
COOLDOWN_MS=$((24 * 3600 * 1000)) # hold / rolled-back auto-expire

now_ms() { date +%s%3N 2>/dev/null || echo $(( $(date +%s) * 1000 )); }

log() { printf '[auto-upgrade] %s\n' "$*" >&2; } # systemd journal captures stderr

# write_state <result> [error] [extra-json-assignments...]
write_state() {
  local result="$1" err="${2:-}"
  mkdir -p "$PREFIX/data"
  python3 - "$STATE" "$result" "$err" "$(now_ms)" <<'PY'
import json, sys
path, result, err, now = sys.argv[1:5]
now = int(now)
try:
    st = json.load(open(path))
    if not isinstance(st, dict):
        st = {}
except Exception:
    st = {}
st["last_run"] = now
st["last_result"] = result
if err:
    st["last_error"] = err[:2000]
else:
    st.pop("last_error", None)
if result == "rolled_back":
    st["hold"] = True          # latch: needs panel clear or 24h cooldown
elif result == "ok":
    st["hold"] = False
json.dump(st, open(path, "w"), indent=2, sort_keys=True)
PY
  # The panel (service user) reads this file; keep it service-owned.
  local svc_user svc_group
  svc_user="$(systemctl show greenpng -p User --value 2>/dev/null || true)"
  svc_group="$(systemctl show greenpng -p Group --value 2>/dev/null || true)"
  if [[ -n "$svc_user" && "$svc_user" != "root" && -f "$STATE" ]]; then
    [[ -z "$svc_group" || "$svc_group" == "root" ]] && svc_group="$svc_user"
    chown "$svc_user:$svc_group" "$STATE" 2>/dev/null || true
  fi
}

# ---- 1. flock: never two runs at once (updater restarts the service) ----
exec 9>"$LOCK"
if ! flock -n 9; then
  log "another auto-upgrade run holds the lock — exiting"
  exit 0
fi

[[ -f "$ENVF" ]] || { log "missing $ENVF"; exit 0; }

# ---- 2. hold latch (panel hold, or automatic rollback) with 24h cooldown ----
read_state_field() { # <key>
  python3 -c 'import json,sys; st=json.load(open(sys.argv[1])); v=st.get(sys.argv[2]); print(v if v is not None else "")' "$STATE" "$1" 2>/dev/null || true
}
STATE_EXISTS=0; [[ -f "$STATE" ]] && STATE_EXISTS=1
if [[ "$STATE_EXISTS" == "1" ]]; then
  LAST_RUN="$(read_state_field last_run)"
  HOLD="$(read_state_field hold)"
  LAST_RESULT="$(read_state_field last_result)"
  AGE_MS=$(( $(now_ms) - ${LAST_RUN:-0} ))
  if [[ "$HOLD" == "True" || "$LAST_RESULT" == "rolled_back" ]]; then
    if [[ "$AGE_MS" -lt "$COOLDOWN_MS" ]]; then
      log "hold latch active (hold=$HOLD last_result=$LAST_RESULT, ${AGE_MS}ms ago < ${COOLDOWN_MS}ms) — skip"
      exit 0
    fi
    log "hold latch expired after 24h — proceeding with a fresh attempt"
  fi
fi

# ---- 3. desired state from shared admin PG (same source the hot thread reads) ----
if ! command -v psql >/dev/null 2>&1; then
  log "psql not installed — cannot read desired state (skip)"
  exit 0
fi
DSN="$(grep -E '^GR_ADMIN_DATABASE_URL=' "$ENVF" | head -1 | cut -d= -f2- || true)"
if [[ -z "$DSN" ]]; then
  DSN="$(grep -E '^GR_DATABASE_URL=' "$ENVF" | head -1 | cut -d= -f2- || true)"
fi
[[ -n "$DSN" ]] || { log "no admin DSN in $ENVF (skip)"; exit 0; }
DESIRED_JSON="$(psql "$DSN" -t -A -c "select value from admin_settings where key='cluster_ota_desired'" 2>/dev/null || true)"
[[ -n "${DESIRED_JSON// /}" ]] || { log "no cluster_ota_desired row (skip)"; exit 0; }

# Parse + window check in one pass (python3 is a hard dep of the updater too).
# NOTE: the desired JSON goes through a temp file — a pipe would lose the race
# with the `python3 -` heredoc that supplies the program itself.
DESIRED_TMP="$(mktemp)"
printf '%s' "$DESIRED_JSON" >"$DESIRED_TMP"
PARSED="$(python3 - "$DESIRED_TMP" "$(date +%H:%M)" <<'PY'
import json, sys
def hhmm(s):
    try:
        h, m = s.strip().split(":")
        h, m = int(h), int(m)
        assert 0 <= h <= 23 and 0 <= m <= 59
        return h * 60 + m
    except Exception:
        return None
def contains(window, now):
    w = window.strip()
    if not w:
        return True
    n = hhmm(now)
    if n is None:
        return False
    for part in w.split(","):
        part = part.strip()
        if not part or "-" not in part:
            continue
        a, b = part.split("-", 1)
        s, e = hhmm(a), hhmm(b)
        if s is None or e is None:
            continue
        if s <= e:
            if s <= n < e:
                return True
        elif n >= s or n < e:  # cross-midnight
            return True
    return False
try:
    with open(sys.argv[1]) as f:
        d = json.load(f)
except Exception:
    print("\t".join(["parse_error"]))
    sys.exit(0)
print("\t".join([
    "1" if d.get("auto_apply") else "0",
    str(d.get("version") or ""),
    str(d.get("release_url") or ""),
    "1" if contains(str(d.get("window") or ""), sys.argv[2]) else "0",
]))
PY
)"
rm -f "$DESIRED_TMP"
IFS=$'\t' read -r AUTO_APPLY TARGET RELEASE_URL WINDOW_OK <<<"${PARSED:-}"
if [[ "$AUTO_APPLY" != "1" ]]; then
  log "desired.auto_apply is not true (manual semantics — skip)"
  exit 0
fi
[[ -n "$TARGET" ]] || { log "desired.version empty (skip)"; exit 0; }
if [[ "$WINDOW_OK" != "1" ]]; then
  log "outside desired maintenance window (skip)"
  exit 0
fi

# ---- 4. monotonic version gate: only strictly newer ----
CUR="$(tr -d '[:space:]' < "$PREFIX/VERSION" 2>/dev/null || true)"
[[ -n "$CUR" ]] || CUR="0.0.0"
NEWER="$(python3 - "$TARGET" "$CUR" <<'PY'
import sys
def v(s):
    try:
        return tuple(int(x) for x in s.strip().split("."))
    except Exception:
        return None
a, b = v(sys.argv[1]), v(sys.argv[2])
print("1" if a and b and a > b else "0")
PY
)" || NEWER="0"
if [[ "$NEWER" != "1" ]]; then
  log "target $TARGET is not newer than current $CUR (no downgrade / no same-version churn — skip)"
  exit 0
fi

# ---- updater present? bootstrap once from the pinned repo when missing ----
if [[ ! -f "$UPDATER" ]]; then
  log "updater missing at $UPDATER — bootstrapping from $RAW_BASE"
  mkdir -p "$(dirname "$UPDATER")"
  if curl -fsSL --max-time 30 -o "$UPDATER" "$RAW_BASE/release/update_runtime_from_github.sh" 2>/dev/null; then
    chmod 0755 "$UPDATER"
  else
    rm -f "$UPDATER"
    log "cannot bootstrap updater (network) — retry next tick"
    exit 0
  fi
fi

# ---- run the root updater (verify chain + staged install + health gate + rollback) ----
UPDATER_ENV=(VERSION="$TARGET" INSTALL_ROOT="$PREFIX" RELEASE_REPO="$RELEASE_REPO")
if [[ "$RELEASE_URL" == https://* ]]; then
  UPDATER_ENV+=("GR_RELEASE_BASE=$RELEASE_URL")  # honor a custom https release base
fi
log "applying $TARGET (current $CUR) via $UPDATER"
UPGRADE_LOG="$PREFIX/log/auto-upgrade-$(date -u +%Y%m%dT%H%M%SZ).log"
mkdir -p "$PREFIX/log"
TMPLOG="$(mktemp)"
if env "${UPDATER_ENV[@]}" bash "$UPDATER" >"$TMPLOG" 2>&1; then
  cat "$TMPLOG" | tee -a "$UPGRADE_LOG" >&2
  rm -f "$TMPLOG"
  chown --reference="$PREFIX/log" "$UPGRADE_LOG" 2>/dev/null || true
  log "updater OK — $CUR → $TARGET"
  write_state "ok" ""
  exit 0
else
  RC=$?
  tail -c 2000 "$TMPLOG" | tee -a "$UPGRADE_LOG" >&2 || true
  rm -f "$TMPLOG"
  log "updater FAILED (exit $RC) — it has rolled back automatically; latching hold (panel auto-hold clears it, or 24h cooldown)"
  write_state "rolled_back" "updater exit $RC for $TARGET (see $UPGRADE_LOG)"
  exit "$RC"
fi
