#!/usr/bin/env bash
set -euo pipefail

# 仓库根解析: 从脚本位置向上找 Cargo.toml (编号布局 / 扁平发行布局两用)
ROOT="$(cd "$(dirname "$0")" && pwd)"
while [[ "$ROOT" != "/" && ! -f "$ROOT/Cargo.toml" ]]; do ROOT="$(dirname "$ROOT")"; done
[[ -f "$ROOT/Cargo.toml" ]] || { echo "[FATAL] cannot locate workspace root (Cargo.toml) from $0" >&2; exit 1; }
SUITE="${1:-all}"
cd "$ROOT"

# 布局可移植: greenpng 工作区(01-official-site / 02-probe-analysis / 03-local-test-lab)
# 或扁平发行仓 gr-server(official-site / panel / tests 在根)。
if [[ -d "$ROOT/02-probe-analysis" ]]; then
  SITE_DIR="$ROOT/01-official-site"
  PANEL_UI_DIR="$ROOT/02-probe-analysis/panel/admin-ui"
  LOCAL_TESTS_DIR="$ROOT/03-local-test-lab/tests/local"
  RUNNERS_DIR="$ROOT/03-local-test-lab/tests/runners"
else
  SITE_DIR="$ROOT/official-site"
  PANEL_UI_DIR="$ROOT/panel/admin-ui"
  LOCAL_TESTS_DIR="$ROOT/tests/local"
  RUNNERS_DIR="$ROOT/tests/runners"
fi

case "$SUITE" in
  contract)
    cargo test -p gr-probe-store --lib
    cargo test -p gr-probe-plane --lib
    cargo check -p gr-service -p gr-admin
    npm ci --prefix "$SITE_DIR"
    npm run test:billing --prefix "$SITE_DIR"
    npm run test:stripe --prefix "$SITE_DIR"
    npm run test:integrations --prefix "$SITE_DIR"
    npm ci --prefix "$PANEL_UI_DIR"
    npm run build --prefix "$PANEL_UI_DIR"
    ;;
  frontend)
    node "$LOCAL_TESTS_DIR/audit_fix_fe_check.js"
    node "$LOCAL_TESTS_DIR/engine_unknown_compat_check.js"
    node "$LOCAL_TESTS_DIR/session_scheduler_check.js"
    node "$LOCAL_TESTS_DIR/dag_v2_fe_check.js"
    node "$LOCAL_TESTS_DIR/fe_standard_b_hashed_only_smoke.js"
    node "$LOCAL_TESTS_DIR/fe_standard_c_no_version_path_smoke.js"
    ;;
  business)
    if [[ ! -d "$ROOT/03-local-test-lab" ]]; then
      echo "[acceptance] business suite requires the dev-repo lab tree (03-local-test-lab); skipped on flat release repo" >&2
      exit 0
    fi
    SKIP_LB=1 MODE="${MODE:-host}" bash "$ROOT/03-local-test-lab/tests/local/run_local_full.sh"
    ;;
  multi-node)
    if [[ ! -d "$ROOT/03-local-test-lab" ]]; then
      echo "[acceptance] multi-node suite requires the dev-repo lab tree (03-local-test-lab); skipped on flat release repo" >&2
      exit 0
    fi
    bash "$ROOT/03-local-test-lab/tests/local/prodsim_full_suite.sh"
    ;;
  upgrade)
    bash "$RUNNERS_DIR/upgrade_rollback_check.sh"
    ;;
  ota-negative)
    bash "$RUNNERS_DIR/ota_negative_check.sh"
    ;;
  all)
    "$0" contract
    "$0" frontend
    "$0" business
    "$0" multi-node
    "$0" upgrade
    "$0" ota-negative
    ;;
  *)
    echo "usage: $0 {contract|frontend|business|multi-node|upgrade|ota-negative|all}" >&2
    exit 2
    ;;
esac
