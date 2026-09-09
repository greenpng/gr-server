#!/usr/bin/env bash
# ota_stability.sh — 公开仓 runner 的多轮面板 OTA 稳定性件 (fulltest.yml ota 件)。
#
# 背景 (178 v1.0.8 生产升级实测的四个坑, 本件全部回归化):
#   1. 并发解包竞争: auto-apply 热循环与面板手动调用同进程并发 fetch 同一
#      bundle, pid-only 暂存目录互踩 → tar exit 2 (gr-ota 已修, 此处回归)。
#   2. 换二进制暂存竞争: 并发 install-runtime 共享 .gr-service.new.<pid>
#      → 分级拷贝互踩 (gr-runtime 已修, 此处回归)。
#   3. 沙箱/属主链: 产品化安装器已带 ReadWritePaths+=bin/VERSION + polkit;
#      systemd 面归 install-smoke/gate-b, 此处覆盖行为面 (install_root 钉
#      GR_INSTALL_ROOT, 断言换文件/版本槽/.bak/VERSION/data overlay 全落地)。
#   4. 重启健康门 + boot data 自举: 旧二进制升级场景由新二进制首次 boot
#      从 ota_staging 自举 data (r100 模板 + geoip) — 断言双通道 (install 时
#      overlay 到 install_root/data; boot 时自举到 data_dir)。
#
# 前置: boot_stack.sh 已起栈 (stack.env 在 $1 或默认 /tmp/gr-e2e);
#       cargo build -p gr-service -p gr-cli (debug) 已完成。
# 轮次:
#   C1 install-runtime A (restart=false) — 单发落地断言
#   C2 full-upgrade B (restart_runtime=false) — 模块+FE+runtime 一体 + data
#   C3 并发风暴 C — 3×install-runtime + 2×full-upgrade 同时打 (坑 1/2 回归)
#   C4 幂等重装 C — 同版重装不坏树、不重复暂存
#   C5 进程重启 → boot data 自举 (旧二进制升级场景) + 新二进制在跑
#   C6 降级回 A — 回滚路径 data 随版本回退
set -euo pipefail
STACK_ENV="${1:-/tmp/gr-e2e/stack.env}"
# shellcheck disable=SC1091
source "$STACK_ENV"

FIX_ROOT="$E2E_DIR/ota-stability"
FIX_PORT="${GR_E2E_OTA_FIX_PORT:-18771}"
pass=0; fail=0
ok()  { echo "PASS  $*"; pass=$((pass+1)); }
bad() { echo "FAIL  $*"; fail=$((fail+1)); }
check() { # check <name> <expected> <actual>
  if [[ "$3" == "$2" ]]; then ok "$1"; else bad "$1 (expected $2 got ${3:0:140})"; fi
}

CON="$ADMIN_BASE/$CONSOLE"
porig() { curl -s -m "${T:-120}" -b "$E2E_DIR/panel.cookies" -H "Origin: $ADMIN_BASE" "$@"; }
jq_get() { python3 -c '
import json,sys
d=json.load(sys.stdin)
for k in sys.argv[1].split("."):
    d = d.get(k) if isinstance(d,dict) else None
    if d is None: break
print(json.dumps(d) if isinstance(d,(dict,list)) else ("" if d is None else d))
' "$1"; }

# ---------- 根定位 (fe/spec/admin/r100 源, 双布局: 公开仓 / 开发仓) ----------
# COMPOSE_FILE: 公开仓 <root>/install/docker/… (root = 上三级);
#               开发仓 <root>/04-release-github-ci/install/docker/… (root = 上四级)。
R3="$(dirname "$(dirname "$(dirname "$COMPOSE_FILE")")")"
R4="$(dirname "$R3")"
pick() { for p in "$@"; do [[ -e "$p" ]] && { printf '%s' "$p"; return 0; }; done; return 1; }
R100_SRC="$(pick "$R3/data/r100_templates.json" "$R4/data/r100_templates.json")" \
  || { echo "ota_stability: r100_templates.json not found" >&2; exit 2; }
GEO_SRC="$(dirname "$R100_SRC")/geo"

arch="$(uname -m)"; [[ "$arch" == x86_64 || "$arch" == aarch64 ]] || arch=x86_64
TRIPLE="${arch}-linux-gnu"
SRV_BIN="$(readlink -f "/proc/$SERVICE_PID/exe")"
CLI_BIN="$(dirname "$SRV_BIN")/gr-cli"
[[ -x "$CLI_BIN" ]] || { echo "ota_stability: gr-cli missing at $CLI_BIN" >&2; exit 2; }

# ---------- 登录 ----------
curl -s -m 10 -c "$E2E_DIR/panel.cookies" -X POST "$CON/api/login" \
  -H 'content-type: application/json' \
  -d "{\"username\":\"$ADMIN_USER\",\"password\":\"$ADMIN_PASS\"}" > "$E2E_DIR/ota-login.json"
grep -q '"ok" *: *true' "$E2E_DIR/ota-login.json" && ok "登录" || { bad "登录 $(head -c 120 "$E2E_DIR/ota-login.json")"; echo "OTA-STABILITY pass=$pass fail=$fail"; exit 1; }

# ---------- 健康三面 ----------
health3() {
  local c p g
  c=$(curl -s -o /dev/null -m 3 -w '%{http_code}' "$ADMIN_BASE/v1/health" || true)
  p=$(curl -s -o /dev/null -m 3 -w '%{http_code}' "$PROBE_BASE/v1/health" || true)
  g=$(curl -s -o /dev/null -m 3 -w '%{http_code}' "$GW_BASE/healthz" || true)
  [[ "$c" == "200" && "$p" == "200" && "$g" == "200" ]]
}

# ---------- fixture: 自足签名的整包 release (与 build_multiarch 同构) ----------
# 尺寸约束 (runner 时限): runtime 副本 strip (debug 未 strip ~300MB → ~40MB);
# admin 收敛为单文件 (面板 OTA 流不消费 admin_tree, 交给 install.sh/gate-b);
# spec 省略 (签名体条件字段, 消费方是 updater 而非面板流)。
build_release() {
  # build_release <ver> <marker>
  local ver="$1" marker="$2" top="greenpng-$1-$arch" rd="$FIX_ROOT/v$1"
  rm -rf "$rd"; mkdir -p "$rd/$top/bin" "$rd/$top/admin"
  cp -f "$SRV_BIN" "$rd/$top/bin/gr-service"
  strip "$rd/$top/bin/gr-service" 2>/dev/null || true
  cp -a "$E2E_DIR/fe" "$rd/$top/fe"
  echo "rel-$marker" > "$rd/$top/fe/e2e-marker.txt"
  echo "e2e admin stub" > "$rd/$top/admin/index.html"
  mkdir -p "$rd/$top/data/geo"
  python3 - "$R100_SRC" "$rd/$top/data/r100_templates.json" "$marker" <<'PY'
import json, sys
src, dst, marker = sys.argv[1:]
d = json.load(open(src))
d["e2e_marker"] = marker
json.dump(d, open(dst, "w"), sort_keys=True)
PY
  cp -f "$GEO_SRC/dbip-asn-lite.mmdb" "$GEO_SRC/dbip-country-lite.mmdb" "$rd/$top/data/geo/"
  # 顺序铁律: fe tgz → manifest(fe sha) → 签名 → 断言 → bundle tar(含 manifest)
  # → index(bundle sha)。tar 必须在 manifest 写入之后, 否则包内无 manifest。
  tar -czf "$rd/fe-$ver.tgz" -C "$rd/$top" fe
  python3 - "$rd" "$top" "$ver" "$TRIPLE" "$arch" <<'PY'
import hashlib, json, pathlib, sys
rd, top, ver, triple, arch = sys.argv[1:6]
rd, bd = pathlib.Path(rd), pathlib.Path(rd) / top
def sha(p): return hashlib.sha256(p.read_bytes()).hexdigest()
def tree_files(root):
    return {str(p.relative_to(root)): sha(p)
            for p in sorted(root.rglob("*")) if p.is_file() and not p.is_symlink()}
man = {
    "product": "greenpng", "channel": "stable", "arch": arch, "triple": triple,
    "runtime": {"version": ver, "abi": 1, "asset": "bin/gr-service", "sha256": sha(bd / "bin/gr-service")},
    "fe": {"asset": f"fe-{ver}.tgz", "sha256": sha(rd / f"fe-{ver}.tgz")},
    "fe_tree": {"epoch": ver, "files": tree_files(bd / "fe")},
    "admin_tree": {"files": tree_files(bd / "admin")},
    "data_tree": {"files": tree_files(bd / "data")},
    "modules": [],
}
json.dump(man, open(bd / "manifest.json", "w"), indent=2)
PY
  "$CLI_BIN" sign-manifest --manifest "$rd/$top/manifest.json" \
    --secret-key "$E2E_DIR/keys/ota_ed25519.sk" >/dev/null
  python3 - "$rd/$top/manifest.json" <<'PY'
import json, sys
m = json.load(open(sys.argv[1]))
assert m.get("sig"), "sig missing"
assert m.get("sig_data"), "sig_data missing (data_tree present → dual sign)"
PY
  tar -C "$rd" -czf "$rd/$top.tar.gz" "$top"
  python3 - "$rd" "$top" "$arch" "$TRIPLE" <<'PY'
import hashlib, json, pathlib, sys
rd, top, arch, triple = sys.argv[1:5]
rd = pathlib.Path(rd)
bundle = f"{top}.tar.gz"
json.dump({"architectures": {arch: {
    "triple": triple, "bundle": bundle,
    "bundle_sha256": hashlib.sha256((rd / bundle).read_bytes()).hexdigest()}}},
    open(rd / "manifest-index.json", "w"), indent=2)
PY
  echo "built rel-$ver marker=$marker"
}

echo "== fixture builds =="
rm -rf "$FIX_ROOT" "$E2E_DIR/data/ota_staging/bundle"
mkdir -p "$FIX_ROOT"
# 可重跑清场: 上一轮 full-upgrade 落的 FE marker 会误伤 C1m 的「install-runtime
# 不动 FE」断言 (runner 每次全新栈, 本地连跑同一栈时需要)。
rm -f "$E2E_DIR/fe/e2e-marker.txt"
build_release 1.0.81 A
build_release 1.0.82 B
build_release 1.0.83 C
ok "三套整包 fixture (bundle+index+双签 manifest)"

setsid nohup python3 -m http.server "$FIX_PORT" --bind 127.0.0.1 --directory "$FIX_ROOT" \
  > "$E2E_DIR/ota-fixhttp.log" 2>&1 & FIX_PID=$!
trap 'kill "$FIX_PID" 2>/dev/null || true' EXIT
sleep 1

# 每套 release 的期望 sha (断言用)
sha_of() { sha256sum "$1" | awk '{print $1}'; }
RT_A=$(sha_of "$FIX_ROOT/v1.0.81/greenpng-1.0.81-$arch.tar.gz")
RT_B=$(sha_of "$FIX_ROOT/v1.0.82/greenpng-1.0.82-$arch.tar.gz")
RT_C=$(sha_of "$FIX_ROOT/v1.0.83/greenpng-1.0.83-$arch.tar.gz")
BIN_A=$(sha_of "$FIX_ROOT/v1.0.81/greenpng-1.0.81-$arch/bin/gr-service")
BIN_B=$(sha_of "$FIX_ROOT/v1.0.82/greenpng-1.0.82-$arch/bin/gr-service")
BIN_C=$(sha_of "$FIX_ROOT/v1.0.83/greenpng-1.0.83-$arch/bin/gr-service")
D_A=$(sha_of "$FIX_ROOT/v1.0.81/greenpng-1.0.81-$arch/data/r100_templates.json")
D_B=$(sha_of "$FIX_ROOT/v1.0.82/greenpng-1.0.82-$arch/data/r100_templates.json")
D_C=$(sha_of "$FIX_ROOT/v1.0.83/greenpng-1.0.83-$arch/data/r100_templates.json")
GEO_A=$(sha_of "$FIX_ROOT/v1.0.81/greenpng-1.0.81-$arch/data/geo/dbip-country-lite.mmdb")

# ---------- C1. install-runtime A (单发, restart=false) ----------
echo "== C1 install-runtime rel-1.0.81 =="
T=180; SR=$(porig -X POST "$CON/api/ota/set-release-url" -H 'content-type: application/json' \
  -d '{"url":"'"http://127.0.0.1:$FIX_PORT/v1.0.81"'"}')
echo "$SR" | grep -q '"ok" *: *true' && ok "C1a set-release-url → rel-A" || bad "C1a set-release-url: ${SR:0:120}"
T=240; R1=$(porig -X POST "$CON/api/ota/install-runtime" -H 'content-type: application/json' -d '{"restart":false}')
check "C1b install-runtime ok" "True" "$(echo "$R1" | jq_get ok)"
check "C1c version" "1.0.81" "$(echo "$R1" | jq_get version)"
check "C1d sha_verified" "True" "$(echo "$R1" | jq_get sha_verified)"
check "C1e restart_requested" "False" "$(echo "$R1" | jq_get restart_requested)"
[[ "$(echo "$R1" | jq_get data_installed)" != "[]" && -n "$(echo "$R1" | jq_get data_installed)" ]] \
  && ok "C1f data_installed 非空 (r100+geo overlay)" || bad "C1f data_installed: $(echo "$R1" | jq_get data_installed)"
check "C1g 版本槽 sha" "$BIN_A" "$(sha_of "$E2E_DIR/install/bin/releases/1.0.81/gr-service")"
check "C1h 主二进制 sha" "$BIN_A" "$(sha_of "$E2E_DIR/install/bin/gr-service")"
ls "$E2E_DIR/install/bin"/.gr-service.new.* >/dev/null 2>&1 \
  && bad "C1i 残留换二进制暂存" || ok "C1i 无残留暂存 (首装无 .bak 属正常)"
check "C1j VERSION" "1.0.81" "$(cat "$E2E_DIR/install/VERSION")"
check "C1k install data overlay (r100)" "$D_A" "$(sha_of "$E2E_DIR/install/data/r100_templates.json")"
check "C1l install data overlay (geo)" "$GEO_A" "$(sha_of "$E2E_DIR/install/data/geo/dbip-country-lite.mmdb")"
[[ ! -f "$E2E_DIR/fe/e2e-marker.txt" ]] && ok "C1m install-runtime 不动 FE" || bad "C1m FE 被误动"
health3 && ok "C1n 服务健康 (三面 200)" || bad "C1n 服务不健康"

# ---------- C2. full-upgrade B (模块+FE+runtime 一体) ----------
echo "== C2 full-upgrade rel-1.0.82 =="
T=300; R2=$(porig -X POST "$CON/api/ota/full-upgrade" -H 'content-type: application/json' \
  -d '{"release_url":"'"http://127.0.0.1:$FIX_PORT/v1.0.82"'","restart_runtime":false}')
check "C2a full-upgrade ok" "True" "$(echo "$R2" | jq_get ok)"
ls "$E2E_DIR/install/bin"/gr-service.bak.* >/dev/null 2>&1 && ok "C2a2 .bak 回滚副本 (换装自 A)" || bad "C2a2 无 .bak"
check "C2b VERSION" "1.0.82" "$(cat "$E2E_DIR/install/VERSION")"
check "C2c 主二进制 sha" "$BIN_B" "$(sha_of "$E2E_DIR/install/bin/gr-service")"
check "C2d fe/VERSION" "1.0.82" "$(cat "$E2E_DIR/fe/VERSION")"
check "C2e fe marker 落地" "rel-B" "$(cat "$E2E_DIR/fe/e2e-marker.txt" 2>/dev/null || echo missing)"
check "C2f install data overlay (r100)" "$D_B" "$(sha_of "$E2E_DIR/install/data/r100_templates.json")"
health3 && ok "C2g 服务健康" || bad "C2g 服务不健康"

# ---------- C3. 并发风暴 C (坑 1/2 的回归: 同进程并发 fetch+swap) ----------
echo "== C3 并发风暴 rel-1.0.83: 3×install-runtime + 2×full-upgrade =="
T=180; porig -X POST "$CON/api/ota/set-release-url" -H 'content-type: application/json' \
  -d '{"url":"'"http://127.0.0.1:$FIX_PORT/v1.0.83"'"}' >/dev/null
# 只 wait 本轮的 5 个调用: 裸 `wait` 会连 fixture http.server (常驻后台任务)
# 一起等 → 永久挂起 (本地实测 24min+ 零 CPU)。
storm_pids=""
for i in 1 2 3; do
  ( T=300 porig -X POST "$CON/api/ota/install-runtime" -H 'content-type: application/json' \
      -d '{"restart":false}' > "$E2E_DIR/ota-storm-rt$i.json" 2>&1 ) &
  storm_pids="$storm_pids $!"
done
for i in 1 2; do
  ( T=300 porig -X POST "$CON/api/ota/full-upgrade" -H 'content-type: application/json' \
      -d '{"restart_runtime":false}' > "$E2E_DIR/ota-storm-fu$i.json" 2>&1 ) &
  storm_pids="$storm_pids $!"
done
# shellcheck disable=SC2086
wait $storm_pids || true
storm_ok=0; storm_fail=""
for f in "$E2E_DIR"/ota-storm-*.json; do
  if grep -q '"ok" *: *true' "$f"; then storm_ok=$((storm_ok+1)); else storm_fail="$storm_fail $f:$(head -c 80 "$f")"; fi
done
check "C3a 风暴 5/5 全 ok" "5" "$storm_ok"
[[ -z "$storm_fail" ]] || echo "     storm failures:$storm_fail" >&2
check "C3b 主二进制 sha (终态=C)" "$BIN_C" "$(sha_of "$E2E_DIR/install/bin/gr-service")"
check "C3c VERSION" "1.0.83" "$(cat "$E2E_DIR/install/VERSION")"
check "C3d install data overlay (r100)" "$D_C" "$(sha_of "$E2E_DIR/install/data/r100_templates.json")"
check "C3e fe marker (终态=C)" "rel-C" "$(cat "$E2E_DIR/fe/e2e-marker.txt" 2>/dev/null || echo missing)"
leftover_extract=$(find "$E2E_DIR/data/ota_staging/bundle" -maxdepth 1 -name '*.extract-*' 2>/dev/null | wc -l)
check "C3f 无残留解包 stage" "0" "$leftover_extract"
leftover_swap=$(find "$E2E_DIR/install/bin" -maxdepth 1 -name '.gr-service.new.*' 2>/dev/null | wc -l)
check "C3g 无残留换二进制暂存" "0" "$leftover_swap"
health3 && ok "C3h 风暴后服务健康" || bad "C3h 风暴后服务不健康"

# ---------- C4. 幂等重装 C ----------
echo "== C4 幂等重装 rel-1.0.83 =="
T=240; R4=$(porig -X POST "$CON/api/ota/install-runtime" -H 'content-type: application/json' -d '{"restart":false}')
check "C4a 重装 ok" "True" "$(echo "$R4" | jq_get ok)"
check "C4b data 未变 (幂等)" "$D_C" "$(sha_of "$E2E_DIR/install/data/r100_templates.json")"
check "C4c 二进制未损" "$BIN_C" "$(sha_of "$E2E_DIR/install/bin/gr-service")"
leftover_extract=$(find "$E2E_DIR/data/ota_staging/bundle" -maxdepth 1 -name '*.extract-*' 2>/dev/null | wc -l)
check "C4d 无残留解包 stage" "0" "$leftover_extract"
health3 && ok "C4e 服务健康" || bad "C4e 服务不健康"

# ---------- C5. 进程重启 → boot data 自举 (旧二进制升级场景收口) ----------
echo "== C5 重启新二进制 → boot data 自举 =="
mapfile -t ORIG_ARGS < <(tr '\0' '\n' < "/proc/$SERVICE_PID/cmdline" | tail -n +2)
ADMIN_PORT_NUM="${ADMIN_BASE##*:}"
kill "$SERVICE_PID" 2>/dev/null || true
for _ in $(seq 1 15); do ss -ltn | grep -q ":$ADMIN_PORT_NUM " || break; sleep 1; done
( set -a; source "$E2E_DIR/gr.env"; set +a
  nohup "$E2E_DIR/install/bin/gr-service" "${ORIG_ARGS[@]}" \
    > "$E2E_DIR/service2.log" 2>&1 & echo $! > "$E2E_DIR/service.pid" )
booted=""
for i in $(seq 1 60); do
  if health3; then booted=1; break; fi
  sleep 1
done
[[ -n "$booted" ]] && ok "C5a 新二进制重启健康" || { bad "C5a 重启不健康"; tail -20 "$E2E_DIR/service2.log" >&2; }
NEW_PID="$(cat "$E2E_DIR/service.pid")"
check "C5b 运行进程 = OTA 装入的二进制" "$E2E_DIR/install/bin/gr-service" "$(readlink -f "/proc/$NEW_PID/exe")"
grep -q "boot data bootstrap" "$E2E_DIR/service2.log" \
  && ok "C5c boot data 自举日志" || bad "C5c 无 boot data bootstrap 日志 (staging 自举缺失?)"
check "C5d data_dir 自举 (r100, boot 通道)" "$D_C" "$(sha_of "$E2E_DIR/data/r100_templates.json")"
check "C5e data_dir 自举 (geo)" "$GEO_A" "$(sha_of "$E2E_DIR/data/geo/dbip-country-lite.mmdb")"
curl -s -o /dev/null -m 5 -w '%{http_code}' "$PROBE_BASE/v1/r100/pack/R00_spotcheck.js" | grep -q 200 \
  && ok "C5f r100 端点活" || bad "C5f r100 端点非 200"
# 重启后重登 (会话可能随进程换代)
curl -s -m 10 -c "$E2E_DIR/panel.cookies" -X POST "$CON/api/login" \
  -H 'content-type: application/json' \
  -d "{\"username\":\"$ADMIN_USER\",\"password\":\"$ADMIN_PASS\"}" >/dev/null

# ---------- C6. 降级回 A (回滚路径) ----------
echo "== C6 降级 rel-1.0.81 =="
T=180; SR=$(porig -X POST "$CON/api/ota/set-release-url" -H 'content-type: application/json' \
  -d '{"url":"'"http://127.0.0.1:$FIX_PORT/v1.0.81"'"}')
echo "$SR" | grep -q '"ok" *: *true' && ok "C6a set-release-url → rel-A" || bad "C6a: ${SR:0:120}"
T=240; R6=$(porig -X POST "$CON/api/ota/install-runtime" -H 'content-type: application/json' -d '{"restart":false}')
check "C6b 降级 ok" "True" "$(echo "$R6" | jq_get ok)"
check "C6c VERSION 回退" "1.0.81" "$(cat "$E2E_DIR/install/VERSION")"
check "C6d 二进制回退" "$BIN_A" "$(sha_of "$E2E_DIR/install/bin/gr-service")"
check "C6e data 随版本回退" "$D_A" "$(sha_of "$E2E_DIR/install/data/r100_templates.json")"
health3 && ok "C6f 服务健康" || bad "C6f 服务不健康"

kill "$FIX_PID" 2>/dev/null || true
echo "OTA-STABILITY pass=$pass fail=$fail"
[[ "$fail" == "0" ]]
