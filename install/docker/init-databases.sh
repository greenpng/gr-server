#!/bin/sh
# Green V7 数据服务初始化: 创建 管理面/业务面 依赖的 3 个附加库
# (greenv6 由 compose 的 POSTGRES_DB 创建, 这里补齐 biz/admin/assoc)
# 8.0 起 (M4) 默认 gr_biz/gr_admin/gr_assoc; 设 GR_DB_LEGACY=1 保持 gv6_* 旧名。
set -e
if [ "${GR_DB_LEGACY:-0}" = "1" ]; then
  BIZ=gv6_biz; ADMIN=gv6_admin; ASSOC=gv6_assoc
else
  BIZ="${GR_DB_BIZ:-gr_biz}"; ADMIN="${GR_DB_ADMIN:-gr_admin}"; ASSOC="${GR_DB_ASSOC:-gr_assoc}"
fi
for db in "$BIZ" "$ADMIN" "$ASSOC"; do
  if ! psql -U gv6 -d postgres -tAc "SELECT 1 FROM pg_database WHERE datname='$db'" | grep -q 1; then
    psql -U gv6 -d postgres -c "CREATE DATABASE \"$db\";"
    echo "created database $db"
  else
    echo "database $db already exists"
  fi
done
