#!/bin/sh
# greenpng 数据服务初始化: 创建 管理面/业务面 依赖的 3 个附加库
# (greenpng 主库由 compose 的 POSTGRES_DB 创建, 这里补齐 gr_biz/gr_admin/gr_assoc)。
set -e
BIZ="${GR_DB_BIZ:-gr_biz}"; ADMIN="${GR_DB_ADMIN:-gr_admin}"; ASSOC="${GR_DB_ASSOC:-gr_assoc}"
for db in "$BIZ" "$ADMIN" "$ASSOC"; do
  if ! psql -U greenpng -d postgres -tAc "SELECT 1 FROM pg_database WHERE datname='$db'" | grep -q 1; then
    psql -U greenpng -d postgres -c "CREATE DATABASE \"$db\";"
    echo "created database $db"
  else
    echo "database $db already exists"
  fi
done
