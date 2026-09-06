#!/bin/bash
# Green V7 dev compose: create the 3 derived DBs (main DB comes from POSTGRES_DB).
# 8.0 (M4): default gr_biz / gr_admin / gr_assoc; GR_DB_LEGACY=1 keeps gr_* old
# names for existing deployments. See docs/13.
set -e
create_db() {
  local db="$1"
  if ! psql -v ON_ERROR_STOP=1 --username "$POSTGRES_USER" --dbname postgres -tAc \
      "SELECT 1 FROM pg_database WHERE datname = '$db'" | grep -q 1; then
    createdb --username "$POSTGRES_USER" "$db"
    echo "[init-databases] created $db"
  else
    echo "[init-databases] $db already exists"
  fi
}
if [[ "${GR_DB_LEGACY:-0}" == "1" ]]; then
  create_db gr_biz
  create_db gr_admin
  create_db gr_assoc
else
  create_db "${GR_DB_BIZ:-gr_biz}"
  create_db "${GR_DB_ADMIN:-gr_admin}"
  create_db "${GR_DB_ASSOC:-gr_assoc}"
fi
