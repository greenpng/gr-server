#!/usr/bin/env bash
# hotcold_e2e.sh — 冷热数据交换端到端 (fulltest.yml hotcold 件)。
#
# 前置: boot_stack.sh --hotcold 已以短旋钮起栈
#   (GR_HOT_IDLE_MS=4s / GR_COLD_TTL_MS=61s / GR_COLD_PURGE_INTERVAL_MS=5s / PROMOTE 窗 1h)
#
# 分层语义 (gr-probe-store hot_cold.rs):
#   L1 热缓存 = 进程内 (idle 4s 后逐出); L2 = PG probe_batches (分析真值源);
#   L3 = PG probe_cold (ingest 即双写; TTL 后批量清除; VT 再上传促升 L3→L1)。
#
# 断言:
#   1. ingest 后 L2 probe_batches 与 L3 probe_cold 双写均在
#   2. 同 VT 再上传 (促升路径) → accepted + 新会话分析仍正确
#   3. TTL (61s) + 清除巡检 (5s) 后 → probe_cold 行清除 (≤120s 内)
#   4. L2 probe_batches 行仍在 (冷清除不动分析真值源) — L2/L3 生命周期独立
#   5. 全程 result 取回 ok, 服务 ERROR 行 == 0
# 用法: bash hotcold_e2e.sh [stack_env_path]
set -euo pipefail
STACK_ENV="${1:-/tmp/gr-e2e/stack.env}"
# shellcheck disable=SC1091
source "$STACK_ENV"

pass=0; fail=0
ok()  { echo "PASS  $*"; pass=$((pass+1)); }
bad() { echo "FAIL  $*"; fail=$((fail+1)); }

psql_biz() { # psql_biz <sql> — 业务库 (sessions/probe_batches/probe_cold)
  docker compose --project-name "$COMPOSE_PROJECT" --env-file "$E2E_DIR/compose.env" \
    -f "$COMPOSE_FILE" exec -T postgres psql -U greenpng -d greenpng -t -A -c "$1"
}

VT="hotcold_$(date +%s)"
SID=""
# 浏览器 UA: 本件测的是「正常访客」的冷热分层全链路。curl 默认 UA
# ("curl/…") 命中 robots 快道 (1.0.10+: 不落 L3/不挂臂, 早判即终判),
# 分层断言会全部失真 — 浏览器线路显式声明; 爬虫快道在 1b 节专项覆盖。
BROWSER_UA='user-agent: Mozilla/5.0 (X11; Linux x86_64) Chrome/120 Safari/537.36'
open_session() { # open_session <vt> [ua_header] → sid
  local vt="$1" ua="${2:-$BROWSER_UA}"
  curl -s -m 8 -X POST "$PROBE_BASE/v1/session/open" -H 'content-type: application/json' -H "$ua" \
    -d "{\"site_id\":\"e2e_hotcold\",\"visitor_terminal_id\":\"$vt\",\"meta\":{\"fe\":\"e2e\"}}" \
    | python3 -c '
import json,sys
d=json.load(sys.stdin)
def f(o):
    if isinstance(o,dict):
        if isinstance(o.get("session_id"),str) and o["session_id"]: return o["session_id"]
        for v in o.values():
            r=f(v)
            if r: return r
    return ""
print(f(d))'
}
ingest() { # ingest <sid> <bid> [ua_header]
  local ua="${3:-$BROWSER_UA}"
  curl -s -m 8 -X POST "$PROBE_BASE/v1/ingest" -H 'content-type: application/json' -H "$ua" \
    -d "{\"session_id\":\"$1\",\"batch_id\":\"$2\",\"source\":\"main\",\"payload\":{\"fields\":{\"os_family\":\"windows\",\"form_class\":\"desktop\",\"timezone\":\"Asia/Shanghai\",\"hardware_concurrency\":8}}}"
}
poll_result() { # poll_result <sid> → ok?
  local r="" i
  for i in $(seq 1 30); do
    r=$(curl -s -m 8 -H "X-Gr-Result-Token: $RESULT_TOKEN" "$PROBE_BASE/v1/session/$1/result?projection=public")
    echo "$r" | grep -qE '"pending"|no analysis' || break
    sleep 1
  done
  echo "$r" | grep -q '"ok" *: *true'
}

# --- 1. 双写 ---
SID="$(open_session "$VT")"
[[ -n "$SID" ]] && ok "open (vt=$VT sid=${SID:0:16}…)" || { bad "open 失败"; echo "HOTCOLD fail=$fail"; exit 1; }
ingest "$SID" lab.hotcold.B0 | grep -q '"accepted" *: *true' && ok "ingest B0" || bad "ingest B0"
ingest "$SID" lab.hotcold.B3 >/dev/null

poll_result "$SID" && ok "分析完成 (L2 真值源工作)" || bad "分析未完成"

N_L2=$(psql_biz "select count(*) from probe_batches where session_id='$SID'")
N_L3=$(psql_biz "select count(*) from probe_cold where session_id='$SID'")
[[ "$N_L2" -ge 2 ]] && ok "L2 probe_batches 双行在 ($N_L2)" || bad "L2 行数 $N_L2"
[[ "$N_L3" -ge 2 ]] && ok "L3 probe_cold 双写 ($N_L3, ingest 即落冷)" || bad "L3 行数 $N_L3"

# --- 1b. 爬虫快道 (curl 默认 UA = robots facet) ---
# 1.0.10+: UA 自明爬虫不落 L3/不挂分析臂 — 早判结果即终判, L2 证据照留。
RSID="$(open_session "${VT}_robot" 'user-agent: curl/8.5.0')"
[[ -n "$RSID" ]] && ok "爬虫 UA open" || bad "爬虫 UA open 失败"
ingest "$RSID" lab.hotcold.B0 'user-agent: curl/8.5.0' | grep -q '"accepted" *: *true' \
  && ok "爬虫 UA ingest accepted (L2 照留)" || bad "爬虫 UA ingest 失败"
RL2=$(psql_biz "select count(*) from probe_batches where session_id='$RSID'")
RL3=$(psql_biz "select count(*) from probe_cold where session_id='$RSID'")
[[ "$RL2" -ge 1 ]] && ok "爬虫快道 L2 证据在 ($RL2)" || bad "爬虫快道 L2 行数 $RL2"
[[ "$RL3" == "0" ]] && ok "爬虫快道 L3 零落 (不冷存等补充探测)" || bad "爬虫快道 L3 应为 0, 实为 $RL3"
poll_result "$RSID" && ok "爬虫快道早判结果可取" || bad "爬虫快道结果缺失"

# --- 2. idle 逐出 + 同 VT 再上传 (促升 L3→L1 路径) ---
echo "[hotcold] sleep 7s (L1 idle 4s 逐出)…"
sleep 7
ING2=$(ingest "$SID" lab.hotcold.B0r)
echo "$ING2" | grep -q '"accepted" *: *true' && ok "同 VT 再上传 accepted (促升路径)" || bad "再上传: ${ING2:0:100}"

# 再上传后新会话依旧正确 (L1 促升不影响分析)
SID2="$(open_session "${VT}_b")"
ingest "$SID2" lab.hotcold.B0 >/dev/null
poll_result "$SID2" && ok "促升后新会话分析正常" || bad "促升后新会话分析失败"

# --- 3. TTL 清除 (61s + 巡检 5s; 轮询 ≤120s) ---
echo "[hotcold] 等 L3 TTL 清除 (≤120s 轮询)…"
purged=0
for i in $(seq 1 60); do
  N=$(psql_biz "select count(*) from probe_cold where session_id='$SID'")
  [[ "$N" == "0" ]] && { purged=1; break; }
  sleep 2
done
[[ "$purged" == "1" ]] && ok "L3 probe_cold TTL 清除" || bad "L3 未清除 (剩 $N)"

# --- 4. L2 独立 (冷清除不动分析真值源) ---
N_L2b=$(psql_biz "select count(*) from probe_batches where session_id='$SID'")
[[ "$N_L2b" -ge 2 ]] && ok "L2 probe_batches 仍在 ($N_L2b) — 与 L3 生命周期独立" || bad "L2 被误清 ($N_L2b)"

# --- 5. 服务面 ---
curl -s -o /dev/null -m 5 -w '' "http://127.0.0.1:$(echo "$PROBE_BASE" | grep -oE '[0-9]+$')/v1/health"
H=$(curl -s -o /dev/null -m 5 -w '%{http_code}' "$PROBE_BASE/v1/health")
[[ "$H" == "200" ]] && ok "健康 200" || bad "健康 $H"
E=$(grep -acE ' ERROR |panic' "$SERVICE_LOG" || true)
[[ "$E" == "0" ]] && ok "服务零 ERROR" || bad "ERROR 行 $E"

echo "HOTCOLD-E2E pass=$pass fail=$fail"
[[ "$fail" == "0" ]]
