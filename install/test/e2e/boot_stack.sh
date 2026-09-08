#!/usr/bin/env bash
# boot_stack.sh — 公开仓 runner 自足的单节点栈引导 (fulltest.yml 行为面三件共用)。
#
# 用法:
#   bash install/test/e2e/boot_stack.sh [--dir <e2e_dir>] [--hotcold] [--stop]
#     --dir     工作目录 (默认 /tmp/gr-e2e)；stack.env / service.log / data 都在里头
#     --hotcold 以短冷热旋钮引导: HOT_IDLE 4s / COLD_TTL 61s / PURGE 每 5s / PROMOTE 窗 1h
#     --stop    停服务 + docker compose down (幂等)
# 产出: $E2E_DIR/stack.env — 可 source，含:
#     CONSOLE ADMIN_USER ADMIN_PASS (来自 data/admin_bootstrap_once.txt)
#     ADMIN_BASE PROBE_BASE GW_BASE SERVICE_PID SERVICE_LOG
#     GR_ADMIN_DATABASE_URL DSN_PG (psql 直连串) E2E_DIR
# 可覆写 env: GR_E2E_PG_PORT(15432) GR_E2E_REDIS_PORT(16379) GR_E2E_ADMIN_PORT(28680)
#             GR_E2E_PROBE_PORT(28765) GR_E2E_GW_PORT(28766) GR_E2E_ANALYZE_WORKERS(2)
#             GR_E2E_COMPOSE_PROJECT(gr-e2e)
# 本地可跑: compose 端口默认避开常驻 lab (5432/6379 不占)；顺序跑多栈时先 --stop。
set -euo pipefail

E2E_DIR="${GR_E2E_DIR:-/tmp/gr-e2e}"
PG_PORT="${GR_E2E_PG_PORT:-15432}"
REDIS_PORT="${GR_E2E_REDIS_PORT:-16379}"
ADMIN_PORT="${GR_E2E_ADMIN_PORT:-28680}"
PROBE_PORT="${GR_E2E_PROBE_PORT:-28765}"
GW_PORT="${GR_E2E_GW_PORT:-28766}"
WORKERS="${GR_E2E_ANALYZE_WORKERS:-2}"
PROJECT="${GR_E2E_COMPOSE_PROJECT:-gr-e2e}"

MODE="boot"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --dir) E2E_DIR="${2:-}"; shift 2 ;;
    --hotcold) MODE="hotcold"; shift ;;
    --stop) MODE="stop"; shift ;;
    *) echo "boot_stack: unknown arg $1" >&2; exit 2 ;;
  esac
done

# 两树同构解析: workspace 根 = 向上第一个含 Cargo.toml + crates/ 的目录
# (开发仓 greenpng/ 与公开仓 checkout 皆满足)。资产路径双回退:
#   compose: <root>/install/docker (公开仓) | <root>/04-release-github-ci/install/docker (开发仓)
#   fe:      <root>/probe/fe       (公开仓) | <root>/02-probe-analysis/probe/fe       (开发仓)
#   spa:     <root>/panel/admin-spa(公开仓) | <root>/02-probe-analysis/panel/admin-spa(开发仓)
D="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT=""
for _p in "$D/.." "$D/../.." "$D/../../.." "$D/../../../.." "$D/../../../../.."; do
  if [[ -f "$_p/Cargo.toml" ]] && [[ -d "$_p/crates" || -d "$_p/02-probe-analysis/crates" ]]; then
    ROOT="$(cd "$_p" && pwd)"; break
  fi
done
[[ -n "$ROOT" ]] || { echo "boot_stack: workspace root not found from $D" >&2; exit 2; }
pick() { for p in "$@"; do [[ -e "$p" ]] && { printf '%s' "$p"; return 0; }; done; return 1; }
COMPOSE_FILE="$(pick "$ROOT/install/docker/docker-compose.yml" "$ROOT/04-release-github-ci/install/docker/docker-compose.yml")" \
  || { echo "boot_stack: docker-compose.yml not found under $ROOT" >&2; exit 2; }
FE_SRC="$(pick "$ROOT/probe/fe" "$ROOT/02-probe-analysis/probe/fe")" \
  || { echo "boot_stack: probe/fe not found under $ROOT" >&2; exit 2; }
SPA_DIR="$(pick "$ROOT/panel/admin-spa" "$ROOT/02-probe-analysis/panel/admin-spa")" \
  || { echo "boot_stack: panel/admin-spa not found under $ROOT" >&2; exit 2; }
BIN_DIR="${GR_E2E_BIN_DIR:-$ROOT/target/debug}"
SRV="$BIN_DIR/gr-service"
CLI="$BIN_DIR/gr-cli"
[[ -x "$SRV" ]] || { echo "boot_stack: $SRV missing (build first: cargo build -p gr-service -p gr-cli)" >&2; exit 2; }

compose() { docker compose --project-name "$PROJECT" --env-file "$E2E_DIR/compose.env" -f "$COMPOSE_FILE" "$@"; }

kill_service_only() {
  # 连续 boot 之间的清场: 只杀服务 + 等端口释放, 不动 compose (抹卷会触发
  # PG entrypoint 重启窗, 服务首连即拒)。--stop 才 compose down -v。
  if [[ -f "$E2E_DIR/stack.env" ]]; then
    # shellcheck disable=SC1091
    set +e; source "$E2E_DIR/stack.env" 2>/dev/null; set -e
    [[ -n "${SERVICE_PID:-}" ]] && kill "$SERVICE_PID" 2>/dev/null || true
  fi
  pkill -f "gr-service.*--data-dir $E2E_DIR" 2>/dev/null || true
  sleep 2
  pkill -9 -f "gr-service.*--data-dir $E2E_DIR" 2>/dev/null || true
  local _p
  for _p in "$ADMIN_PORT" "$PROBE_PORT" "$GW_PORT"; do
    for _ in $(seq 1 15); do
      ss -ltn 2>/dev/null | grep -q ":${_p} " || break
      pkill -9 -f "gr-service.*--data-dir $E2E_DIR" 2>/dev/null || true
      sleep 1
    done
  done
}

stop_stack() {
  kill_service_only
  if [[ -f "$E2E_DIR/compose.env" ]]; then
    compose down -v >/dev/null 2>&1 || true
  fi
  echo "[boot_stack] stopped (project=$PROJECT)"
}

if [[ "$MODE" == "stop" ]]; then stop_stack; exit 0; fi

mkdir -p "$E2E_DIR"

# --- 1. compose PG + Redis (项目隔离; 密码本地随机, 不进 git) ---
if [[ ! -f "$E2E_DIR/compose.env" ]]; then
  {
    echo "POSTGRES_PASSWORD=$(openssl rand -hex 16)"
    echo "REDIS_PASSWORD=$(openssl rand -hex 16)"
    echo "POSTGRES_PORT=$PG_PORT"
    echo "REDIS_PORT=$REDIS_PORT"
  } > "$E2E_DIR/compose.env"
fi
# shellcheck disable=SC1091
set -a; source "$E2E_DIR/compose.env"; set +a

compose up -d --wait
for i in $(seq 1 60); do
  compose exec -T postgres psql -U greenpng -d postgres -tAc "SELECT 1" >/dev/null 2>&1 && break
  sleep 1
done
compose exec -T postgres psql -U greenpng -d postgres -tAc "SELECT 1" >/dev/null 2>&1 \
  || { echo "boot_stack: PG never became ready" >&2; exit 1; }
# 全新卷的 entrypoint 在 init 脚本后会重启一次 PG — 逐库带重试地建, 覆盖重启窗
# (与 release.yml gate-b 同款模式)。
for db in greenpng gr_biz gr_admin gr_assoc; do
  ok=0
  for i in $(seq 1 30); do
    compose exec -T postgres psql -U greenpng -d postgres -c "CREATE DATABASE $db" >/dev/null 2>&1 || true
    compose exec -T postgres psql -U greenpng -d postgres -tAc "SELECT 1 FROM pg_database WHERE datname='$db'" 2>/dev/null | grep -q 1 && { ok=1; break; }
    sleep 1
  done
  [[ "$ok" == "1" ]] || { echo "boot_stack: db $db never appeared" >&2; exit 1; }
done
DSN_PG="postgres://greenpng:${POSTGRES_PASSWORD}@127.0.0.1:${PG_PORT}/greenpng"

# --- 2. 实验室 OTA 根钥 (runner 无私钥; keygen 一次性, 用于签名 fixture) ---
if [[ ! -f "$E2E_DIR/keys/ota_ed25519.sk" ]]; then
  "$CLI" keygen --out-dir "$E2E_DIR/keys" >/dev/null
fi

# --- 3. 引导环境 ---
# 连续 boot 清场: 只杀服务 (compose 保持, 避免抹卷重建的 entrypoint 重启窗)
kill_service_only
rm -rf "$E2E_DIR/data" "$E2E_DIR/fe" "$E2E_DIR/modules"
mkdir -p "$E2E_DIR/data" "$E2E_DIR/modules"
cp -a "$FE_SRC" "$E2E_DIR/fe"

ADMIN_PASS_E2E="e2e-$(openssl rand -hex 12)"
cat > "$E2E_DIR/gr.env" <<EOF
GR_DATABASE_URL=$DSN_PG
GR_BIZ_DATABASE_URL=postgres://greenpng:${POSTGRES_PASSWORD}@127.0.0.1:${PG_PORT}/gr_biz
GR_ASSOCIATION_DATABASE_URL=postgres://greenpng:${POSTGRES_PASSWORD}@127.0.0.1:${PG_PORT}/gr_assoc
GR_ADMIN_DATABASE_URL=postgres://greenpng:${POSTGRES_PASSWORD}@127.0.0.1:${PG_PORT}/gr_admin
GR_REDIS_URL=redis://:${REDIS_PASSWORD}@127.0.0.1:${REDIS_PORT}/0
GR_OFFICIAL_URL=https://example.com
GR_OAUTH_ADMIN_EMAILS=admin@example.com
GR_OAUTH_LOCAL_USER=admin
GR_OAUTH_REDIRECT_URI=http://127.0.0.1:${ADMIN_PORT}/oauth/callback
GR_ADMIN_USER=admin
GR_ADMIN_PASSWORD=${ADMIN_PASS_E2E}
GR_DEPLOY_ENV=lab
GR_CLUSTER_KEY=e2e-cluster-key-0123456789
GR_PUBKEY_PATH=$E2E_DIR/keys/ota_ed25519.pk
GR_REQUIRE_MANIFEST_SIG=1
GR_RESULT_TOKEN=e2e-result-token
GR_REQUIRE_RESULT_TOKEN=1
EOF

# 冷热短旋钮 (--hotcold): 默认 L1 30m / L3 7d / 窗 24h → 分钟级可观测
if [[ "$MODE" == "hotcold" ]]; then
  cat >> "$E2E_DIR/gr.env" <<EOF
GR_HOT_IDLE_MS=4000
GR_COLD_TTL_MS=61000
GR_COLD_PROMOTE_WINDOW_MS=3600000
GR_COLD_PURGE_INTERVAL_MS=5000
EOF
fi

boot_once() {
  # shellcheck disable=SC1091
  ( set -a; source "$E2E_DIR/gr.env"; set +a
    setsid nohup "$SRV" \
      --bind "127.0.0.1:${ADMIN_PORT}" --probe-bind "127.0.0.1:${PROBE_PORT}" \
      --gateway-bind "127.0.0.1:${GW_PORT}" \
      --data-dir "$E2E_DIR/data" --static-dir "$E2E_DIR/fe" \
      --admin-spa "$SPA_DIR" --modules-dir "$E2E_DIR/modules" \
      --role all --analyze-workers "$WORKERS" \
      --cluster-key e2e-cluster-key-0123456789 \
      --pubkey-path "$E2E_DIR/keys/ota_ed25519.pk" \
      > "$E2E_DIR/service.log" 2>&1 & echo $! > "$E2E_DIR/service.pid" )
}

wait_healthy() {
  local i c p g
  for i in $(seq 1 60); do
    kill -0 "$(cat "$E2E_DIR/service.pid" 2>/dev/null)" 2>/dev/null || return 1
    c=$(curl -s -o /dev/null -m 3 -w '%{http_code}' "http://127.0.0.1:${ADMIN_PORT}/v1/health" || true)
    p=$(curl -s -o /dev/null -m 3 -w '%{http_code}' "http://127.0.0.1:${PROBE_PORT}/v1/health" || true)
    g=$(curl -s -o /dev/null -m 3 -w '%{http_code}' "http://127.0.0.1:${GW_PORT}/healthz" || true)
    # 三面全绿才健康 — 网关线程 AddrInUse panic 时主进程仍答 admin/probe
    [[ "$c" == "200" && "$p" == "200" && "$g" == "200" ]] && return 0
    sleep 1
  done
  return 1
}

boot_once
if ! wait_healthy; then
  # 首轮偶发撞 PG 就绪缝 → 等稳后原数据目录重试一次 (admin store 幂等)
  echo "[boot_stack] first boot not healthy — retry once" >&2
  kill_service_only
  sleep 5
  boot_once
fi
SERVICE_PID="$(cat "$E2E_DIR/service.pid")"
if ! wait_healthy; then
  echo "boot_stack: service did not become healthy" >&2
  tail -20 "$E2E_DIR/service.log" >&2 || true
  stop_stack
  exit 1
fi

# --- 5. bootstrap 凭据 (console 随机路径; AdminHub 落 data/admin/ 子目录) ---
BOOT=""
for _b in "$E2E_DIR/data/admin/admin_bootstrap_once.txt" "$E2E_DIR/data/admin_bootstrap_once.txt"; do
  [[ -f "$_b" ]] && BOOT="$_b" && break
done
[[ -n "$BOOT" ]] || { echo "boot_stack: bootstrap secrets file missing" >&2; stop_stack; exit 1; }
CONSOLE="$(sed -n 's/^console_path=//p' "$BOOT" | head -1 | tr -d '/')"

cat > "$E2E_DIR/stack.env" <<EOF
E2E_DIR=$E2E_DIR
CONSOLE=$CONSOLE
ADMIN_USER=admin
ADMIN_PASS=${ADMIN_PASS_E2E}
ADMIN_BASE=http://127.0.0.1:${ADMIN_PORT}
PROBE_BASE=http://127.0.0.1:${PROBE_PORT}
GW_BASE=http://127.0.0.1:${GW_PORT}
SERVICE_PID=$SERVICE_PID
SERVICE_LOG=$E2E_DIR/service.log
GR_ADMIN_DATABASE_URL=postgres://greenpng:${POSTGRES_PASSWORD}@127.0.0.1:${PG_PORT}/gr_admin
DSN_PG=$DSN_PG
RESULT_TOKEN=e2e-result-token
COMPOSE_FILE=$COMPOSE_FILE
COMPOSE_PROJECT=$PROJECT
EOF

echo "[boot_stack] up: console=/$CONSOLE admin=:$ADMIN_PORT probe=:$PROBE_PORT gw=:$GW_PORT pid=$SERVICE_PID mode=$MODE"
