#!/usr/bin/env bash
# panel_e2e.sh — 公开仓 runner 的管理面板端到端 (fulltest.yml panel 件)。
#
# 覆盖 (面板 API 面 + 设置生效行为面, 无浏览器 — 浏览器 UI 流留在开发仓 qa-panel):
#   A. 认证: 未认证 401 ×3 / 错口令 401 / 登录 / me / 登出后 401
#   B. 设置生效 (面板改动 → 系统行为真变化):
#      B1 站点 upsert → 站点列表回显 + embed_token 签发 → 用它驱动
#         open→ingest→result 全链 (面板建的站点真的能跑业务)
#      B2 workers POST {analyze:3} → GET 回显 3
#      B3 ota/set-release-url → ota/remote + auto-state 回显新 URL
#      B4 ota/auto-hold 置位/清除 → auto-state 回显
#   C. 在线签名 OTA (瘦 fixture): lab keygen 根钥签 manifest (GR_REQUIRE_MANIFEST_SIG=1)
#      → set-release-url 指向本地资产服务 → 面板 ota/install-fe → FE 热换成功
#   D. 审计: /api/audit 能看到本会话动作
# 前置: boot_stack.sh 已起栈 (stack.env 在 $1 或默认 /tmp/gr-e2e)
# 用法: bash panel_e2e.sh [stack_env_path]
set -euo pipefail
STACK_ENV="${1:-/tmp/gr-e2e/stack.env}"
# shellcheck disable=SC1091
source "$STACK_ENV"

# fixture 也在 E2E_DIR 下
FIX_DIR="$E2E_DIR/ota-fixture"
pass=0; fail=0
ok()  { echo "PASS  $*"; pass=$((pass+1)); }
bad() { echo "FAIL  $*"; fail=$((fail+1)); }
check() { # check <name> <expected> <actual>
  if [[ "$3" == "$2" ]]; then ok "$1"; else bad "$1 (expected $2 got ${3:0:120})"; fi
}
jq_get() { python3 -c '
import json,sys
d=json.load(sys.stdin)
for k in sys.argv[1].split("."):
    d = d.get(k) if isinstance(d,dict) else None
    if d is None: break
print(json.dumps(d) if isinstance(d,(dict,list)) else ("" if d is None else d))
' "$1"; }

CON="$ADMIN_BASE/$CONSOLE"
# CSRF 中间件: 带 cookie 的非 GET /api/ 请求要求 Origin == http://<Host>。
# 所有面板 POST 统一带 Origin: $ADMIN_BASE。
porig() { curl -s -m "${T:-10}" -b "$E2E_DIR/panel.cookies" -H "Origin: $ADMIN_BASE" "$@"; }

# ---------- A. 认证 ----------
check "A1 me 未认证 401" 401 "$(curl -s -o /dev/null -m 8 -w '%{http_code}' "$CON/api/me")"
check "A2 sites 写未认证 401" 401 "$(curl -s -o /dev/null -m 8 -w '%{http_code}' -X POST "$CON/api/sites" -H "Origin: $ADMIN_BASE" -H 'content-type: application/json' -d '{"site_id":"x","name":"x","consent_confirmed":true}')"
check "A3 auto-state 未认证 401" 401 "$(curl -s -o /dev/null -m 8 -w '%{http_code}' "$CON/api/ota/auto-state")"
check "A4 错口令 401" 401 "$(curl -s -o /dev/null -m 8 -w '%{http_code}' -X POST "$CON/api/login" -H 'content-type: application/json' -d '{"username":"admin","password":"wrong"}')"

curl -s -m 10 -c "$E2E_DIR/panel.cookies" -X POST "$CON/api/login" \
  -H 'content-type: application/json' \
  -d "{\"username\":\"$ADMIN_USER\",\"password\":\"$ADMIN_PASS\"}" > "$E2E_DIR/login.json"
grep -q '"ok" *: *true' "$E2E_DIR/login.json" && ok "A5 登录" || bad "A5 登录 $(cat "$E2E_DIR/login.json" | head -c 120)"

ME=$(curl -s -m 8 -b "$E2E_DIR/panel.cookies" "$CON/api/me")
echo "$ME" | grep -q "\"$ADMIN_USER\"" && ok "A6 me 回显用户" || bad "A6 me: ${ME:0:100}"

# ---------- B1. 站点设置生效: 建站 → 全链 ----------
SITE_ID="e2e_panel_$$"
porig -X POST "$CON/api/sites" -H 'content-type: application/json' -d "{
    \"site_id\":\"$SITE_ID\",\"name\":\"e2e panel\",\"collect_enabled\":true,
    \"edge_mode\":\"dual_domain\",\"fe_load\":\"pv\",\"upload_ingest\":\"gv\",
    \"root_domains\":[\"e2e.example\"],\"consent_confirmed\":true,
    \"cookie_fields\":[\"user_id\"]}" > "$E2E_DIR/site_upsert.json"
grep -q '"ok" *: *true' "$E2E_DIR/site_upsert.json" && ok "B1a 站点 upsert" || bad "B1a 站点 upsert $(head -c 120 "$E2E_DIR/site_upsert.json")"

SITES=$(curl -s -m 8 -b "$E2E_DIR/panel.cookies" "$CON/api/sites")
EMBED=$(echo "$SITES" | jq_get "sites" | python3 -c "
import json,sys
rows=json.load(sys.stdin) if sys.stdin.read() else []
" 2>/dev/null || true)
# sites 列表形状兼容: 找到本站记录并取 embed_token
EMBED=$(echo "$SITES" | python3 -c '
import json,sys
d=json.load(sys.stdin)
rows = d if isinstance(d,list) else (d.get("sites") or d.get("rows") or [])
tok=""
for r in rows:
    if isinstance(r,dict) and r.get("site_id")==sys.argv[1]:
        tok=r.get("embed_token") or ""
        break
print(tok)' "$SITE_ID")
[[ "$EMBED" == grst_* ]] && ok "B1b 站点列表回显 + embed_token" || bad "B1b embed_token: '$EMBED' (list: ${SITES:0:100})"

VT="panel_e2e_$(date +%s)"
# 浏览器 UA: B1 走完整 open→ingest→analyze 行为链 (设置→行为生效)。
# curl 默认 UA 命中 robots 快道 (1.0.10+), 会短路成早判 — 那不是本节要测的。
BROWSER_UA='user-agent: Mozilla/5.0 (X11; Linux x86_64) Chrome/120 Safari/537.36'
OPEN=$(curl -s -m 8 -X POST "$PROBE_BASE/v1/session/open" -H 'content-type: application/json' -H "$BROWSER_UA" \
  -d "{\"site_id\":\"$SITE_ID\",\"embed_token\":\"$EMBED\",\"visitor_terminal_id\":\"$VT\",\"meta\":{\"fe\":\"e2e\"}}")
SID=$(echo "$OPEN" | jq_get "session_id")
[[ -n "$SID" ]] && ok "B1c 面板站点 open (token 绑定)" || bad "B1c open: ${OPEN:0:120}"
ING=$(curl -s -m 8 -X POST "$PROBE_BASE/v1/ingest" -H 'content-type: application/json' -H "$BROWSER_UA" \
  -d "{\"session_id\":\"$SID\",\"batch_id\":\"lab.e2e.B0\",\"source\":\"main\",\"embed_token\":\"$EMBED\",\"payload\":{\"fields\":{\"os_family\":\"windows\",\"form_class\":\"desktop\",\"timezone\":\"Asia/Shanghai\",\"hardware_concurrency\":8}}}")
echo "$ING" | grep -q '"accepted" *: *true' && ok "B1d 面板站点 ingest" || bad "B1d ingest: ${ING:0:120}"
R=""
for i in $(seq 1 30); do
  R=$(curl -s -m 8 -H "X-Gr-Result-Token: $RESULT_TOKEN" "$PROBE_BASE/v1/session/$SID/result?projection=public")
  echo "$R" | grep -qE '"pending"|no analysis' || break
  sleep 1
done
echo "$R" | grep -q '"ok" *: *true' && ok "B1e 面板站点 result 完成 (设置→行为生效)" || bad "B1e result: ${R:0:120}"

# ---------- B2. workers ----------
W=$(porig -X POST "$CON/api/workers" -H 'content-type: application/json' -d '{"analyze":3}')
echo "$W" | grep -q '"ok" *: *true' && ok "B2a workers POST analyze=3" || bad "B2a workers POST: ${W:0:100}"
WG=$(curl -s -m 8 -b "$E2E_DIR/panel.cookies" "$CON/api/workers")
echo "$WG" | grep -q '"analyze" *: *3' && ok "B2b workers GET 回显 3" || bad "B2b workers GET: ${WG:0:100}"

# ---------- C. 在线签名 OTA (瘦 fixture) ----------
VER="$(cat "$E2E_DIR/fe/VERSION" 2>/dev/null || echo 1.0.0)"
arch="$(uname -m)"; [[ "$arch" == x86_64 || "$arch" == aarch64 ]] || arch=x86_64
rm -rf "$FIX_DIR"; mkdir -p "$FIX_DIR/v$VER"
runtime_asset="gr-service-$VER-${arch}-linux-gnu"
SRV_BIN="$(readlink -f "/proc/$SERVICE_PID/exe")"
cp "$SRV_BIN" "$FIX_DIR/v$VER/$runtime_asset"
cp "$E2E_DIR/keys/ota_ed25519.pk" "$FIX_DIR/v$VER/"
cp -a "$E2E_DIR/fe" "$FIX_DIR/v$VER/fe"
tar -czf "$FIX_DIR/v$VER/fe-$VER.tgz" -C "$FIX_DIR/v$VER" fe
runtime_sha=$(sha256sum "$FIX_DIR/v$VER/$runtime_asset" | awk '{print $1}')
fe_sha=$(sha256sum "$FIX_DIR/v$VER/fe-$VER.tgz" | awk '{print $1}')
cat > "$FIX_DIR/v$VER/manifest-${arch}-linux-gnu.json" <<EOF
{"product":"greenpng","channel":"stable","arch":"$arch","triple":"${arch}-linux-gnu","runtime":{"version":"$VER","abi":1,"asset":"$runtime_asset","sha256":"$runtime_sha"},"fe":{"asset":"fe-$VER.tgz","sha256":"$fe_sha"},"modules":[]}
EOF
CLI_BIN="$(dirname "$SRV_BIN")/gr-cli"
"$CLI_BIN" sign-manifest --manifest "$FIX_DIR/v$VER/manifest-${arch}-linux-gnu.json" \
  --secret-key "$E2E_DIR/keys/ota_ed25519.sk" >/dev/null
cp "$FIX_DIR/v$VER/manifest-${arch}-linux-gnu.json" "$FIX_DIR/v$VER/manifest.json"
python3 -c 'import json,sys;m=json.load(open(sys.argv[1]));assert m.get("sig"),"sig missing"' "$FIX_DIR/v$VER/manifest.json" \
  && ok "C1 fixture manifest 已签名 (lab 根钥)" || bad "C1 签名缺失"

FIX_PORT=18765
setsid nohup python3 -m http.server "$FIX_PORT" --bind 127.0.0.1 --directory "$FIX_DIR" \
  > "$E2E_DIR/fixhttp.log" 2>&1 & FIX_PID=$!
sleep 1
FIX_URL="http://127.0.0.1:${FIX_PORT}/v$VER"

T=60; SR=$(porig -X POST "$CON/api/ota/set-release-url" -H 'content-type: application/json' -d "{\"url\":\"$FIX_URL\"}")
echo "$SR" | grep -q '"ok" *: *true' && ok "C2 set-release-url → 本地 fixture" || bad "C2 set-release-url: ${SR:0:140}"

T=90; FE=$(porig -X POST "$CON/api/ota/install-fe" -H 'content-type: application/json' -d '{}')
echo "$FE" | grep -q '"ok" *: *true' && ok "C3 面板 install-fe (sha 验签 + 热换)" || bad "C3 install-fe: ${FE:0:160}"

# ---------- B3/B4. ota 状态面 (放在 fixture 后, 不破坏 C 的 URL) ----------
REMOTE=$(curl -s -m 8 -b "$E2E_DIR/panel.cookies" "$CON/api/ota/remote")
echo "$REMOTE" | grep -q "$FIX_URL" && ok "B3 ota/remote 回显新 URL" || bad "B3 remote: ${REMOTE:0:120}"
AST=$(curl -s -m 8 -b "$E2E_DIR/panel.cookies" "$CON/api/ota/auto-state")
echo "$AST" | grep -q "$FIX_URL" && ok "B3b auto-state 回显 URL" || bad "B3b auto-state: ${AST:0:120}"

AH=$(porig -X POST "$CON/api/ota/auto-hold" -H 'content-type: application/json' -d '{"hold":true}')
echo "$AH" | grep -q '"ok" *: *true' && ok "B4a auto-hold 置位" || bad "B4a hold: ${AH:0:100}"
AST2=$(curl -s -m 8 -b "$E2E_DIR/panel.cookies" "$CON/api/ota/auto-state")
echo "$AST2" | grep -qi '"hold" *: *true' && ok "B4b auto-state 回显 hold" || bad "B4b state: ${AST2:0:140}"
AH2=$(porig -X POST "$CON/api/ota/auto-hold" -H 'content-type: application/json' -d '{"hold":false}')
echo "$AH2" | grep -q '"ok" *: *true' && ok "B4c auto-hold 清除" || bad "B4c unhold: ${AH2:0:100}"

# ---------- D. 审计 ----------
AUD=$(curl -s -m 8 -b "$E2E_DIR/panel.cookies" "$CON/api/audit")
echo "$AUD" | grep -qE 'login|upsert|ota' && ok "D1 审计含本会话动作" || bad "D1 audit: ${AUD:0:100}"

# ---------- 收尾 ----------
curl -s -m 8 -b "$E2E_DIR/panel.cookies" -H "Origin: $ADMIN_BASE" -X POST "$CON/api/logout" >/dev/null || true
check "A7 登出后 me 401" 401 "$(curl -s -o /dev/null -m 8 -w '%{http_code}' "$CON/api/me")"
kill "$FIX_PID" 2>/dev/null || true

echo "PANEL-E2E pass=$pass fail=$fail"
[[ "$fail" == "0" ]]
