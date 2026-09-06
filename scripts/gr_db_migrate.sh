#!/usr/bin/env bash
# gr_db_migrate.sh — 8.0 GR 命名迁移：gr_* DB → gr_* DB（仅新部署/自愿迁移）
# 用法:  GR_ADMIN_DB_URL=postgres://user:pass@host:5432/postgres bash gr_db_migrate.sh [--dry-run]
# 动作:  对 gr_biz/gr_admin/gr_assoc: 不存在 gr_* 时 CREATE DATABASE gr_* (TEMPLATE gr_*),
#        存在时跳过(不覆盖)。
set -euo pipefail
ADMIN="${GR_ADMIN_DB_URL:-${GR_ADMIN_DB_URL:-postgres://gr:grdev@127.0.0.1:5432/postgres}}"
DRY="${1:-}"
MAPPING=(gr_biz:gr_biz gr_admin:gr_admin gr_assoc:gr_assoc)
for m in "${MAPPING[@]}"; do
  old="${m%%:*}"; new="${m##*:}"
  if psql "$ADMIN" -tAc "SELECT 1 FROM pg_database WHERE datname='$new'" | grep -q 1; then
    echo "[gr-db] $new already exists (skip)"
    continue
  fi
  if psql "$ADMIN" -tAc "SELECT 1 FROM pg_database WHERE datname='$old'" | grep -q 1; then
    if [[ "$DRY" == "--dry-run" ]]; then
      echo "[gr-db][dry] would CREATE DATABASE $new TEMPLATE $old"
    else
      psql "$ADMIN" -v ON_ERROR_STOP=1 -c "CREATE DATABASE $new TEMPLATE $old"
      echo "[gr-db] created $new from $old"
    fi
  else
    echo "[gr-db] $old not present (skip)"
  fi
done
