#!/usr/bin/env bash
# GR naming migration tool — rewrite legacy GR_*/GR_* entries to GR_*.
#
# docs/guides/13-GR-MIGRATION.md §9 (M3 deliverable): for each target name, replace
# `GR_NAME=`/`GR_NAME=` lines with `GR_NAME=<value>`. When the value differs
# across the two legacy names, the GR value wins and the loser is kept as a
# comment. Backs up each file before writing and prints a diff summary.
#
# Usage:
#   scripts/gr_env_migrate.sh NAME [FILE_OR_DIR ...]
#   scripts/gr_env_migrate.sh --all [FILE_OR_DIR ...]   # every mapped name
#   --dry-run   print diff without writing
#
# Example:
#   scripts/gr_env_migrate.sh DATABASE_URL /opt/green-v7/.env /etc/systemd/system/
#   scripts/gr_env_migrate.sh --all --dry-run /opt/green-v7/.env
set -euo pipefail

# Builtin map: GR name → legacy name that is the semantic latest (charter §5).
# Legacy names not listed (e.g. GR-only side switches) still migrate: GR_<name>
# simply takes their value.
declare -A LEGACY=(
  [DATABASE_URL]=GR_DATABASE_URL
  [PRODUCT_VERSION]=GR_PRODUCT_VERSION
  [DEPLOY_ENV]=GR_DEPLOY_ENV
  [BUILD_ID]=GR_BUILD_ID
  [RELEASE_BASE]=GR_RELEASE_BASE
  [OTA_ROOT_PUBKEY_SHA256]=GR_OTA_ROOT_PUBKEY_SHA256
  [REQUIRE_MANIFEST_SIG]=GR_REQUIRE_MANIFEST_SIG
  [PUBKEY_PATH]=GR_PUBKEY_PATH
  [MODULE_VERSION]=GR_MODULE_VERSION
  [MODEL_KEY_DEFAULT_CLASS]=GR_MODEL_KEY_DEFAULT_CLASS
  [REQUIRE_RESULT_TOKEN]=GR_REQUIRE_RESULT_TOKEN
  [REQUIRE_SEALED_INGEST]=GR_REQUIRE_SEALED_INGEST
  [ADMIN_USER]=GR_ADMIN_USER
  [ADMIN_PASSWORD]=GR_ADMIN_PASSWORD
  [CORS_ORIGINS]=GR_CORS_ORIGINS
  [RESULT_TOKEN]=GR_RESULT_TOKEN
  [SITE_RESULT_TOKENS]=GR_SITE_RESULT_TOKENS
  [CHALLENGE_SECRET]=GR_CHALLENGE_SECRET
  [ALLOW_LAB_CHALLENGE]=GR_ALLOW_LAB_CHALLENGE
  [REDIS_URL]=GR_REDIS_URL
  [WEBHOOK_OUTBOX_PATH]=GR_WEBHOOK_OUTBOX_PATH
  [SIDE_LAB]=GR_SIDE_LAB
  [NO_KEEPALIVE]=GR_NO_KEEPALIVE
  [H3_ALT_SVC]=GR_H3_ALT_SVC
)

DRY_RUN=0
MODE_ALL=0
declare -a TARGETS=()
declare -a NAMES=()

usage() {
  echo "USAGE: $0 NAME [FILE_OR_DIR ...] | $0 --all [FILE_OR_DIR ...] [--dry-run]" >&2
  exit 2
}

[[ $# -gt 0 ]] || usage
while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run) DRY_RUN=1; shift ;;
    --all) MODE_ALL=1; shift ;;
    --*) echo "unknown option: $1" >&2; usage ;;
    *)
      if [[ $MODE_ALL -eq 0 && ${#NAMES[@]} -eq 0 ]]; then
        NAMES+=("$1")
      else
        TARGETS+=("$1")
      fi
      shift
      ;;
  esac
done

if [[ $MODE_ALL -eq 1 ]]; then
  NAMES=("${!LEGACY[@]}")
fi
[[ ${#NAMES[@]} -gt 0 ]] || usage
[[ ${#TARGETS[@]} -gt 0 ]] || TARGETS=("$PWD")

# Expand dirs → .env / *.env / *.service files.
declare -a FILES=()
for t in "${TARGETS[@]}"; do
  if [[ -d "$t" ]]; then
    while IFS= read -r -d '' f; do FILES+=("$f"); done < <(
      find "$t" -type f \( -name '.env' -o -name '*.env' -o -name '*.service' \) -print0 | sort -z
    )
  else
    FILES+=("$t")
  fi
done

total=0
for f in "${FILES[@]}"; do
  [[ -f "$f" && -r "$f" ]] || continue
  changed=0
  tmp="$(mktemp)"
  trap 'rm -f "$tmp"' RETURN
  while IFS= read -r line || [[ -n "$line" ]]; do
    # Never touch comment lines (original GR_* inside comments stays documented).
    if [[ "$line" =~ ^[[:space:]]*# ]]; then
      printf '%s\n' "$line" >> "$tmp"
      continue
    fi
    matched=""
    for name in "${NAMES[@]}"; do
      legacy="${LEGACY[$name]:-}"
      # Match the semantic-latest legacy name, or the generic GR_/GR_ twin.
      for cand in "$legacy" "GR_${name}" "GR_${name}"; do
        [[ -n "$cand" ]] || continue
        if [[ "$line" == "$cand="* ]]; then
          matched=("$name" "$cand" "${line#*=}")
          break 2
        fi
      done
    done
    if [[ -n "$matched" ]]; then
      name="${matched[0]}"; value="${matched[2]}"
      inner="$tmp"
      if grep -q "^GR_${name}=" "$inner" || grep -q "# gr-migrate: GR_${name}=" "$inner"; then
        # GR line already present/claimed in this file: keep legacy as comment.
        printf '# gr-migrate: kept %s (alias; GR_%s present)\n' "$line" "$name" >> "$tmp"
      else
        printf 'GR_%s=%s\n' "$name" "$value" >> "$tmp"
      fi
      changed=1
      continue
    fi
    printf '%s\n' "$line" >> "$tmp"
  done < "$f"

  if [[ $changed -eq 1 ]]; then
    total=$((total+1))
    if [[ $DRY_RUN -eq 1 ]]; then
      echo "== $f (dry-run) =="
      diff -u "$f" "$tmp" | tail -n +3 || true
    else
      bak="$f.grbak.$(date -u +%Y%m%dT%H%M%SZ)"
      cp -a "$f" "$bak"
      cat "$tmp" > "$f"
      echo "== $f rewritten (backup: $bak) =="
      diff -u "$bak" "$f" | tail -n +3 || true
    fi
  fi
  rm -f "$tmp"
  trap - RETURN
done

echo "gr_env_migrate: name(s)=${#NAMES[@]} file(s)=${#FILES[@]} rewritten=$total"
if [[ $DRY_RUN -eq 1 && $total -gt 0 ]]; then
  echo "gr_env_migrate: dry-run only — rerun without --dry-run to write"
fi
