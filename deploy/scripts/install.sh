#!/usr/bin/env bash
# Pure-script install for green-v6 (user-managed PostgreSQL).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
PREFIX="${GR_PREFIX:-${GR_PREFIX:-/opt/green-v6}}"
DATA="${GR_DATA_DIR:-${GR_DATA_DIR:-$PREFIX/data}}"

echo "[gr] install prefix=$PREFIX data=$DATA"
mkdir -p "$PREFIX/bin" "$PREFIX/modules" "$DATA" "$PREFIX/admin-spa"

if [[ -x "$ROOT/target/release/gr-service" ]]; then
  cp -f "$ROOT/target/release/gr-service" "$PREFIX/bin/"
  cp -f "$ROOT/target/release/gr-cli" "$PREFIX/bin/" 2>/dev/null || true
  cp -f "$ROOT/target/release/gr-harden" "$PREFIX/bin/" 2>/dev/null || true
else
  echo "[gr] building release..."
  (cd "$ROOT" && cargo build --release -p gr-service -p gr-cli -p gr-harden)
  cp -f "$ROOT/target/release/gr-service" "$PREFIX/bin/"
  cp -f "$ROOT/target/release/gr-cli" "$PREFIX/bin/"
  cp -f "$ROOT/target/release/gr-harden" "$PREFIX/bin/"
fi

if [[ -d "$ROOT/admin-spa" ]]; then
  mkdir -p "$PREFIX/admin-spa/dist"
  cp -f "$ROOT/admin-spa/"* "$PREFIX/admin-spa/dist/" 2>/dev/null || true
fi

cat > "$PREFIX/.env.example" <<EOF
GR_BIND=127.0.0.1:28680
GR_BIND=127.0.0.1:28680
GR_PROBE_BIND=127.0.0.1:28765
GR_PROBE_BIND=127.0.0.1:28765
GR_GATEWAY_BIND=127.0.0.1:28766
GR_GATEWAY_BIND=127.0.0.1:28766
GR_DATA_DIR=/opt/green-v6/data
GR_DATA_DIR=/opt/green-v6/data
GR_MODULES_DIR=/opt/green-v6/modules
GR_MODULES_DIR=/opt/green-v6/modules
GR_ADMIN_SPA=/opt/green-v6/admin-spa/dist
GR_ADMIN_SPA=/opt/green-v6/admin-spa/dist
GR_DATABASE_URL=postgres://gr:CHANGE_ME@127.0.0.1:5432/greenv6
GR_DATABASE_URL=postgres://gr:CHANGE_ME@127.0.0.1:5432/greenv6
GR_BIZ_DATABASE_URL=postgres://gr:CHANGE_ME@127.0.0.1:5432/gr_biz
GR_BIZ_DATABASE_URL=postgres://gr:CHANGE_ME@127.0.0.1:5432/gr_biz
GR_ADMIN_DATABASE_URL=postgres://gr:CHANGE_ME@127.0.0.1:5432/gr_admin
GR_ADMIN_DATABASE_URL=postgres://gr:CHANGE_ME@127.0.0.1:5432/gr_admin
GR_ASSOCIATION_DATABASE_URL=postgres://gr:CHANGE_ME@127.0.0.1:5432/gr_assoc
GR_ASSOCIATION_DATABASE_URL=postgres://gr:CHANGE_ME@127.0.0.1:5432/gr_assoc
GR_CLUSTER_KEY=$(openssl rand -hex 32)
GR_CLUSTER_KEY=$(openssl rand -hex 32)
GR_DEPLOY_ENV=prod
GR_DEPLOY_ENV=prod
EOF

if [[ -f "$PREFIX/.env" ]]; then
  set -a
  # shellcheck disable=SC1091
  source "$PREFIX/.env"
  set +a
fi
if [[ -z "${GR_ADMIN_DATABASE_URL:-${GR_ADMIN_DATABASE_URL:-}}" || -z "${GR_DATABASE_URL:-${GR_DATABASE_URL:-}}" ]]; then
  echo "[gr] PostgreSQL DSNs required before install. Copy $PREFIX/.env.example → $PREFIX/.env and set:"
  echo "  GR_DATABASE_URL (旧 GR_DATABASE_URL)  GR_ADMIN_DATABASE_URL  GR_BIZ_DATABASE_URL  GR_ASSOCIATION_DATABASE_URL"
  exit 1
fi

"$PREFIX/bin/gr-cli" install --data-dir "$DATA"

echo "[gr] done. Secrets:"
cat "$DATA/admin/admin_bootstrap_once.txt" || true
echo "[gr] configure DB DSNs in $PREFIX/.env then start:"
echo "  set -a; source $PREFIX/.env; set +a"
echo "  $PREFIX/bin/gr-service --data-dir $DATA --modules-dir $PREFIX/modules --admin-spa $PREFIX/admin-spa/dist"
