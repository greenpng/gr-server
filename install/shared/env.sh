#!/usr/bin/env bash
# GR naming migration shell helpers (docs/guides/13-GR-MIGRATION.md §4).
#
# Read rule: GR_* first, fall back to GV6_*, then GV5_*. New scripts and
# new env-file/unit content write GR_* only; legacy names keep working for
# billing-cycle-old nodes.
#
# Source from other scripts:
#   . "$(dirname "$0")/shared/env.sh"   # (install/ prefix layout)
#   gr_env NAME [default]

# gr_env NAME [default] — first set of GR_/GV6_/GV5_ wins; unset → default/empty.
gr_env() {
  local name="$1" default="${2:-}" p k v
  for p in GR_ GV6_ GV5_; do
    k="${p}${name}"
    v="${!k:-}"
    if [[ -n "$v" ]]; then
      printf '%s' "$v"
      return 0
    fi
  done
  printf '%s' "$default"
}

# gr_env_flag NAME [default] — "1"/"true"/"yes" → 1, else 0 (or default).
gr_env_flag() {
  local name="$1" default="${2:-0}" v
  v="$(gr_env "$name")"
  case "${v,,}" in
    1|true|yes|on) printf '1' ;;
    ''|0|false|no|off) printf '%s' "$default" ;;
    *) printf '%s' "$default" ;;
  esac
}
