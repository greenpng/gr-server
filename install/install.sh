#!/usr/bin/env bash
# Green V7 一键安装脚本 (sh)
#
# 从公开发行仓库 greenpng/gr-server 下载签名发布资产 (x86_64 / aarch64), 校验 sha256 +
# ed25519 模块签名, 落地 /opt/green-v7, 生成 systemd 服务。
# 历史版本 6.x/7.x/8.0.x 曾发布于 greenpng/install, 可用 GR_RELEASE_REPO 覆盖回去。
#
# 数据库/缓存连接信息按"管理面 / 业务面 / 缓存"分组填写 (8.0 起 GR_* 为主名, 旧名在括号):
#   - 业务面(探测/分析): GR_DATABASE_URL  GR_BIZ_DATABASE_URL  GR_ASSOCIATION_DATABASE_URL
#                      (旧 GV6_DATABASE_URL / GV6_BIZ_DATABASE_URL / GV6_ASSOCIATION_DATABASE_URL)
#   - 管理面(面板/审计):  GR_ADMIN_DATABASE_URL (旧 GV6_ADMIN_DATABASE_URL)
#   - 缓存(可选):         GR_REDIS_URL (旧 GV5_REDIS_URL; 留空 = 本地文件 soft store)
#   - 管理面板登录:        本地管理员账号（GR_ADMIN_USER，无需官网）
#
# 用法:
#   bash install.sh [--version 8.0.0] [--arch auto|x86_64|aarch64]
#                   [--prefix /opt/green-v7] [--env-file path]
#                   [--with-docker] [--no-systemd] [--yes] [--dry-run]
#
# 环境变量: GR_RELEASE_REPO (旧 GV6_RELEASE_REPO; 默认 greenpng/gr-server), GR_RAW_BASE
#           (旧 GV6_RAW_BASE; 默认 raw.githubusercontent.com/greenpng/gr-server/main),
#           GR_OTA_ROOT_PUBKEY_SHA256 (信任根公钥指纹, 8.0.3 起切换到 greenpng 新根),
#           SUDO_PASS (免交互 sudo). 旧名 ENV_FILE 输入仍兼容 (自动同步)。
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RELEASE_REPO="${GR_RELEASE_REPO:-${GV6_RELEASE_REPO:-greenpng/gr-server}}"
RAW_BASE="${GR_RAW_BASE:-${GV6_RAW_BASE:-https://raw.githubusercontent.com/greenpng/gr-server/main}}"
OTA_ROOT_PUBKEY_SHA256="${GR_OTA_ROOT_PUBKEY_SHA256:-${GV6_OTA_ROOT_PUBKEY_SHA256:-4a2296d33e66838d8a8cbd697a686bfb79c93a3d7da8a8f4cd60e949b297ea0d}}"
PREFIX="${GR_PREFIX:-${GV6_PREFIX:-/opt/green-v7}}"
VERSION=""
ARCH="auto"
ENV_FILE=""
WITH_DOCKER=0
NO_SYSTEMD=0
YES=0
DRY_RUN=0

# ---------- 参数 ----------
while [[ $# -gt 0 ]]; do
  case "$1" in
    --version) VERSION="${2:-}"; shift 2 ;;
    --arch) ARCH="${2:-auto}"; shift 2 ;;
    --prefix) PREFIX="${2:-}"; shift 2 ;;
    --env-file) ENV_FILE="${2:-}"; shift 2 ;;
    --with-docker) WITH_DOCKER=1; shift ;;
    --no-systemd) NO_SYSTEMD=1; shift ;;
    --yes) YES=1; shift ;;
    --dry-run) DRY_RUN=1; shift ;;
    -h|--help) sed -n '1,20p' "$0"; exit 0 ;;
    *) echo "[install] unknown arg: $1" >&2; exit 2 ;;
  esac
done

# ---------- 环境探测 ----------
host_arch="$(uname -m)"
case "$host_arch" in
  x86_64|amd64) host_arch=x86_64 ;;
  aarch64|arm64) host_arch=aarch64 ;;
  *) echo "[install] ERROR: unsupported host architecture: $(uname -m); use x86_64 or aarch64" >&2; exit 1 ;;
esac
[[ "$ARCH" == "auto" ]] && ARCH="$host_arch"
case "$ARCH" in
  x86_64|aarch64) ;;
  *) echo "[install] ERROR: unsupported --arch '$ARCH'; expected auto, x86_64, or aarch64" >&2; exit 2 ;;
esac
TRIPLE="${ARCH}-linux-gnu"

for c in curl openssl tar sha256sum file install; do
  command -v "$c" >/dev/null 2>&1 || { echo "[install] missing required: $c" >&2; exit 1; }
done
PY3=0; command -v python3 >/dev/null 2>&1 && PY3=1

say() { printf '[install] %s\n' "$*"; }
die() { printf '[install] ERROR: %s\n' "$*" >&2; exit 1; }
have_sudo() { id -u 2>/dev/null | grep -q '^0$' || sudo -n true 2>/dev/null; }

ask() { # ask <var> <prompt> <default> <secret=0|1>
  local var="$1" prompt="$2" dflt="${3:-}" secret="${4:-0}" ans
  if [[ "$YES" == "1" || -n "$ENV_FILE" ]]; then
    printf -v "$var" '%s' "${!var:-$dflt}"
    return 0
  fi
  while :; do
    if [[ "$secret" == "1" ]]; then
      read -r -p "? $prompt ${dflt:+[默认保密]} : " -s ans; echo
    else
      read -r -p "? $prompt ${dflt:+[$dflt]} : " ans
    fi
    ans="${ans:-$dflt}"
    [[ -n "$ans" ]] && break
  done
  printf -v "$var" '%s' "$ans"
}

# ---------- 版本解析 ----------
if [[ -z "$VERSION" ]]; then
  # 未认证 api.github.com 在共享/CI IP 上偶发限流: 重试数轮。
  # 仍解析不到则直接报错并要求显式 --version 8.x.y（或兼容 7.x.y）— 刻意不回退到
  # /releases/latest 的 302 跳转目标, 避免把 v6 旧版当 V7/GR 下载
  # (v6/v7 资产命名、manifest 结构与签名要求不同, 混用会被下方 sha/ELF/签名校验拒绝)。
  if [[ "$PY3" == "1" ]]; then
    for _attempt in 1 2 3; do
      VERSION="$(curl -fsSL --max-time 15 "https://api.github.com/repos/${RELEASE_REPO}/releases?per_page=100" 2>/dev/null \
        | python3 -c 'import sys,json,re; vs=[]; rs=json.load(sys.stdin); [vs.append((tuple(map(int,m.groups())),m.group(0))) for r in rs if r.get("tag_name") and (m:=re.fullmatch(r"v?([78])\.(\d+)\.(\d+)",r["tag_name"]))]; print(max(vs)[1].lstrip("v") if vs else "")' 2>/dev/null || true)"
      [[ -n "$VERSION" ]] && break
      sleep 2
    done
  fi
  if [[ -z "$VERSION" ]]; then
    die "cannot resolve V7/GR stable release; pass --version 8.x.y (7.x.y still accepted for rollback)"
  fi
fi
[[ "$VERSION" =~ ^(7|8)\.[0-9]+\.[0-9]+$ ]] || die "Green V7/GR installer requires a 7.x.y or 8.x.y version (got $VERSION)"
say "version=$VERSION arch=$ARCH triple=$TRIPLE prefix=$PREFIX"
BASE="${GR_RELEASE_BASE:-${GV6_RELEASE_BASE:-https://github.com/${RELEASE_REPO}/releases/download/v${VERSION}}}"
if [[ "$DRY_RUN" == "1" ]]; then
  say "dry-run: architecture and release selection validated; no files or services changed"
  exit 0
fi

# ---------- (可选) Docker 数据服务: PG(4 库) + Redis ----------
POSTGRES_PASSWORD=""; REDIS_PASSWORD=""
# 8.0 GR 命名迁移 (M4, docs/13): 新部署数据库名 gr_biz/gr_admin/gr_assoc
# (+ greenv7 主库); 存量 gv6_* 名保留, 迁移用 scripts/gr_db_migrate.sh。
GR_DB_NAMES="${GR_DB_NAMES:-gr}"   # gr=新命名; 旧=保持 gv6_biz/gv6_admin/gv6_assoc
DBZ_BIZ="${DBZ_BIZ:-$( [[ "$GR_DB_NAMES" == "gr" ]] && echo gr_biz || echo gv6_biz )}"
DBZ_ADMIN="${DBZ_ADMIN:-$( [[ "$GR_DB_NAMES" == "gr" ]] && echo gr_admin || echo gv6_admin )}"
DBZ_ASSOC="${DBZ_ASSOC:-$( [[ "$GR_DB_NAMES" == "gr" ]] && echo gr_assoc || echo gv6_assoc )}"
# 端口跟随 docker/.env 里的 POSTGRES_PORT/REDIS_PORT 覆盖 (默认 5432/6379),
# 避免服务器已有 5432/6379 占用时生成的 DSN 与容器映射不一致。
pg_dsn() { echo "postgres://gv6:${POSTGRES_PASSWORD}@127.0.0.1:${POSTGRES_PORT:-5432}/$1"; }
if [[ "$WITH_DOCKER" == "1" ]]; then
  command -v docker >/dev/null 2>&1 || die "--with-docker requires docker"
  DDIR="$SCRIPT_DIR/docker"
  D_ENV="$DDIR/.env"
  if [[ -f "$D_ENV" ]]; then
    set -a; # shellcheck disable=SC1091
    source "$D_ENV"; set +a
  else
    POSTGRES_PASSWORD="$(openssl rand -hex 16)"
    REDIS_PASSWORD="$(openssl rand -hex 16)"
    cat >"$D_ENV" <<EOF
POSTGRES_PASSWORD=${POSTGRES_PASSWORD}
REDIS_PASSWORD=${REDIS_PASSWORD}
EOF
    chmod 600 "$D_ENV"
  fi
  say "docker: starting postgres(4 db: greenv7 + $DBZ_BIZ/$DBZ_ADMIN/$DBZ_ASSOC)+redis on 127.0.0.1"
  (cd "$DDIR" && docker compose up -d --wait 2>/dev/null || docker compose up -d)
  for i in $(seq 1 30); do
    if docker compose -f "$DDIR/docker-compose.yml" exec -T postgres pg_isready -U gv6 >/dev/null 2>&1; then break; fi
    sleep 1
  done
  docker compose -f "$DDIR/docker-compose.yml" exec -T postgres pg_isready -U gv6 >/dev/null 2>&1 \
    || die "postgres not ready"
  say "docker: data services ready (dsn prefix postgres://gv6:****@127.0.0.1:5432)"
fi

# ---------- 连接信息(分组向导 / ENV_FILE) ----------
if [[ -n "$ENV_FILE" ]]; then
  [[ -f "$ENV_FILE" ]] || die "--env-file not found: $ENV_FILE"
  set -a; # shellcheck disable=SC1091
  source "$ENV_FILE"; set +a
fi

# ---------- GR_* ↔ 旧名双向同步 (8.0 GR 命名迁移, docs/13 §3) ----------
# 老 ENV_FILE 只写 GV5_/GV6_ 名也能用; 新文件写 GR_* 则旧名自动补出 (7.x 二进制兼容)。
gr_sync() { # gr_sync <GR名> [<GV6名|空>] [<GV5名|空>]
  local gr="$1" a="${2:-}" b="${3:-}" v
  if [[ -z "${!gr:-}" ]]; then
    if [[ -n "${a:-}" && -n "${!a:-}" ]]; then v="${!a}"
    elif [[ -n "${b:-}" && -n "${!b:-}" ]]; then v="${!b}"
    fi
    [[ -n "${v:-}" ]] && printf -v "$gr" '%s' "$v"
  fi
  [[ -n "${!gr:-}" ]] || return 0
  [[ -z "${a:-}" || -n "${!a:-}" ]] || printf -v "$a" '%s' "${!gr}"
  [[ -z "${b:-}" || -n "${!b:-}" ]] || printf -v "$b" '%s' "${!gr}"
}
gr_sync GR_DATABASE_URL GV6_DATABASE_URL
gr_sync GR_BIZ_DATABASE_URL GV6_BIZ_DATABASE_URL
gr_sync GR_ASSOCIATION_DATABASE_URL GV6_ASSOCIATION_DATABASE_URL
gr_sync GR_ADMIN_DATABASE_URL GV6_ADMIN_DATABASE_URL
gr_sync GR_REDIS_URL "" GV5_REDIS_URL
gr_sync GR_ANALYZE_WORKERS GV6_ANALYZE_WORKERS
gr_sync GR_CLUSTER_KEY GV6_CLUSTER_KEY
gr_sync GR_RESULT_TOKEN GV6_RESULT_TOKEN
gr_sync GR_OPS_TOKEN GV6_OPS_TOKEN
gr_sync GR_ADMIN_USER "" GV5_ADMIN_USER
gr_sync GR_COOKIE_SECURE GV6_COOKIE_SECURE
gr_sync GR_DEV_INSECURE_COOKIE GV6_DEV_INSECURE_COOKIE

echo; echo "=============================================="
echo " Green V7 连接信息配置 (管理面 / 业务面 / 缓存)"
echo "=============================================="
[[ "$WITH_DOCKER" == "1" ]] || echo " (可用 --with-docker 自动在本机起 PG+Redis 数据服务)"
echo; echo "--- [A] 业务面 PostgreSQL (会话/探针冷存储/分析队列) ---"
ask GR_DATABASE_URL "GR_DATABASE_URL (旧 GV6_DATABASE_URL)" "$(pg_dsn greenv6 2>/dev/null || echo "${PG_DSN_PREFIX:-postgres://127.0.0.1:5432}/greenv7")"
ask GR_BIZ_DATABASE_URL "GR_BIZ_DATABASE_URL (旧 GV6_BIZ_DATABASE_URL)" "$(pg_dsn "$DBZ_BIZ" 2>/dev/null || true)"
ask GR_ASSOCIATION_DATABASE_URL "GR_ASSOCIATION_DATABASE_URL (旧 GV6_ASSOCIATION_DATABASE_URL)" "$(pg_dsn "$DBZ_ASSOC" 2>/dev/null || true)"
echo; echo "--- [B] 管理面 PostgreSQL (面板/审计/集群 OTA) ---"
ask GR_ADMIN_DATABASE_URL "GR_ADMIN_DATABASE_URL (旧 GV6_ADMIN_DATABASE_URL)" "$(pg_dsn "$DBZ_ADMIN" 2>/dev/null || true)"
echo; echo "--- [C] 缓存 Redis (可选; 留空 = 本地文件) ---"
ask GR_REDIS_URL "GR_REDIS_URL (旧 GV5_REDIS_URL)" "${REDIS_PASSWORD:+redis://:${REDIS_PASSWORD}@127.0.0.1:${REDIS_PORT:-6379}/0}"
echo; echo "--- [D] 管理面板登录 ---"
echo "  本地管理员账号 = GR_ADMIN_USER（首次启动自动生成一次性密码，见 data/admin/admin_bootstrap_once.txt）"
echo; echo "--- [E] 运行参数 (回车默认) ---"
ask GR_ANALYZE_WORKERS "分析 worker 数 GR_ANALYZE_WORKERS / GV6_ANALYZE_WORKERS" "2"

GR_CLUSTER_KEY="${GR_CLUSTER_KEY:-${GV6_CLUSTER_KEY:-$(openssl rand -hex 32)}}"
GR_RESULT_TOKEN="${GR_RESULT_TOKEN:-${GV6_RESULT_TOKEN:-$(openssl rand -hex 16)}}"
GR_OPS_TOKEN="${GR_OPS_TOKEN:-${GV6_OPS_TOKEN:-$(openssl rand -hex 16)}}"
GR_ADMIN_USER="${GR_ADMIN_USER:-${GV5_ADMIN_USER:-gv7admin}}"

if [[ "$YES" != "1" ]]; then
  echo; echo "---- 配置汇总 ----"
  for v in GR_DATABASE_URL GR_BIZ_DATABASE_URL GR_ADMIN_DATABASE_URL GR_REDIS_URL GR_ADMIN_USER; do
    echo "  $v=${!v}"
  done
  printf '继续安装? [y/N] '; read -r ok; [[ "$ok" == "y" || "$ok" == "Y" ]] || exit 0
fi

# ---------- 下载 + 校验 (参考 update_runtime_from_github.sh) ----------
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$PREFIX/bin" "$PREFIX/modules" "$PREFIX/data" "$PREFIX/dist/release-$VERSION"

resolve_manifest() {
  local ok=0
  if curl -fsSL -o "$TMP/manifest-index.json" "$BASE/manifest-index.json" 2>/dev/null; then
    if [[ "$PY3" == "1" ]]; then
      man_name=$(python3 - <<PY
import json
idx=json.load(open("$TMP/manifest-index.json"))
e=(idx.get("architectures") or {}).get("$ARCH") or {}
print(e.get("manifest") or "")
PY
)
      if [[ -n "$man_name" && "$man_name" != *"/"* && "$man_name" != *".."* ]] \
        && curl -fsSL -o "$TMP/manifest.json" "$BASE/$man_name"; then
        say "manifest from index: $man_name"
        ok=1
      fi
    fi
  fi
  if [[ "$ok" != "1" ]]; then
    for m in "manifest-${TRIPLE}.json" "manifest.json"; do
      if curl -fsSL -o "$TMP/manifest.json" "$BASE/$m" 2>/dev/null; then
        say "manifest: $m"; ok=1; break
      fi
    done
  fi
  [[ "$ok" == "1" ]]
}
resolve_manifest || die "cannot resolve manifest for $VERSION/$TRIPLE"

manifest_val() { # manifest_val <json-path>
  [[ "$PY3" == "1" ]] && python3 -c "import json,sys; m=json.load(open(sys.argv[1])); print(m$1 or '')" "$TMP/manifest.json" 2>/dev/null || true
}

SVC_ASSET="$(manifest_val '["runtime"]["asset"]')"
[[ -n "$SVC_ASSET" ]] || die "manifest runtime.asset missing"
[[ "$SVC_ASSET" != */* && "$SVC_ASSET" != *..* && "$SVC_ASSET" == *"-${TRIPLE}" ]] || die "runtime asset invalid or wrong architecture: $SVC_ASSET"
say "fetching $SVC_ASSET"
curl -fsSL -o "$TMP/gr-service" "$BASE/$SVC_ASSET"
chmod +x "$TMP/gr-service"

# sha256 校验 (R-01)
EXPECT_SHA="$(manifest_val '["runtime"]["sha256"]')"
[[ -n "$EXPECT_SHA" ]] || die "runtime sha256 missing from manifest"
GOT_SHA="$(sha256sum "$TMP/gr-service" | awk '{print $1}')"
[[ "$GOT_SHA" == "$EXPECT_SHA" ]] || die "runtime sha256 mismatch: got $GOT_SHA expect $EXPECT_SHA"
say "runtime sha256 verified"

# ELF 检查
file "$TMP/gr-service" 2>/dev/null | grep -qi ELF || die "downloaded asset is not an ELF binary"
if [[ "$ARCH" == "x86_64" ]]; then
  file "$TMP/gr-service" | grep -q 'x86-64' || die "runtime ELF architecture mismatch"
else
  file "$TMP/gr-service" | grep -q 'ARM aarch64' || die "runtime ELF architecture mismatch"
fi

# FE / admin-spa / 公钥
( curl -fsSL -o "$TMP/fe.tgz" "$BASE/fe-${VERSION}.tgz" 2>/dev/null \
  && curl -fsSL -o "$TMP/admin-spa.tgz" "$BASE/admin-spa.tgz" 2>/dev/null ) || true
curl -fsSL -o "$TMP/ota_ed25519.pk" "$RAW_BASE/ota_ed25519.pk" 2>/dev/null \
  || curl -fsSL -o "$TMP/ota_ed25519.pk" "$BASE/ota_ed25519.pk" 2>/dev/null \
  || die "cannot fetch ota_ed25519.pk"
[[ "$(sha256sum "$TMP/ota_ed25519.pk" | awk '{print $1}')" == "$OTA_ROOT_PUBKEY_SHA256" ]] \
  || die "OTA root public key fingerprint mismatch"
# P1-4: the `gr-cli` helper performs module verification + staging, so its bytes
# must match the signed manifest before it is ever invoked. 8.x manifests use
# gr-cli-*; 7.x rollback manifests may still point at legacy gv6-* helper assets.
CLI_ASSET="$(manifest_val '["cli"]["asset"]')"
CLI_SHA="$(manifest_val '["cli"]["sha256"]')"
if [[ -n "$CLI_ASSET" || -n "$CLI_SHA" ]]; then
  [[ -n "$CLI_ASSET" ]] || die "manifest cli.asset missing (signed CLI coverage required)"
  [[ "$CLI_ASSET" != */* && "$CLI_ASSET" != *..* && "$CLI_ASSET" == *"-${TRIPLE}" ]] \
    || die "manifest cli.asset invalid or wrong architecture: $CLI_ASSET"
  curl -fsSL -o "$TMP/gr-cli" "$BASE/$CLI_ASSET"
  chmod +x "$TMP/gr-cli"
  [[ -n "$CLI_SHA" ]] || die "manifest cli.sha256 missing"
  GOT_CLI_SHA="$(sha256sum "$TMP/gr-cli" | awk '{print $1}')"
  [[ "$GOT_CLI_SHA" == "$CLI_SHA" ]] || die "cli sha256 mismatch: got $GOT_CLI_SHA expect $CLI_SHA"
  say "cli sha256 verified ($CLI_ASSET)"
else
  # Legacy 7.0.0/7.0.1 manifests predate `cli`; try the 8.x GR name first and
  # the old 7.x gv6 helper second, then install it locally as bin/gr-cli.
  for cand in "gr-cli-${VERSION}-${TRIPLE}" "gv6-${VERSION}-${TRIPLE}"; do
    if curl -fsSL -o "$TMP/gr-cli" "$BASE/$cand" 2>/dev/null; then
      chmod +x "$TMP/gr-cli"
      say "WARN: manifest has no cli entry; using unsigned legacy helper asset $cand"
      CLI_ASSET="$cand"
      break
    fi
  done
  [[ -x "$TMP/gr-cli" ]] || die "cannot fetch gr-cli helper asset for $VERSION/$TRIPLE"
fi
# Verify the manifest root signature before invoking any downloaded helper.
# The canonical body mirrors gv6-ota::manifest_sign_message.
python3 - "$TMP/manifest.json" "$TMP/manifest-body" "$TMP/manifest.sig" <<'PY'
import base64, json, sys
m = json.load(open(sys.argv[1]))
body = {
  "product": m.get("product", ""),
  "channel": m.get("channel", ""),
  "build_id": m.get("build_id") or "",
  "release_pubkey": m.get("release_pubkey") or "",
  "runtime": {k: m["runtime"].get(k) for k in ("version", "abi", "asset", "sha256")},
  "fe": ({"asset": m["fe"].get("asset"), "sha256": m["fe"].get("sha256")}
         if m.get("fe") is not None else None),
  "modules": [{k: a.get(k) for k in (
    "name", "version", "abi", "requires_major", "min_runtime_minor",
    "max_runtime_minor", "asset", "sha256", "domain")} for a in m.get("modules", [])],
}
# P1-4: mirror the Rust canonical body — `cli` is included ONLY when present.
if m.get("cli") is not None:
    body["cli"] = {k: m["cli"].get(k) for k in ("version", "abi", "asset", "sha256")}
if not m.get("sig"):
    raise SystemExit("manifest signature missing")
open(sys.argv[2], "wb").write(json.dumps(body, sort_keys=True, separators=(",", ":")).encode())
s = m["sig"].replace("-", "+").replace("_", "/")
open(sys.argv[3], "wb").write(base64.b64decode(s + "=" * (-len(s) % 4)))
PY
python3 - "$TMP/ota_ed25519.pk" "$TMP/ota-root.der" <<'PY'
import sys
k = open(sys.argv[1], "rb").read()
if len(k) != 32: raise SystemExit("OTA root key must be 32 bytes")
open(sys.argv[2], "wb").write(bytes.fromhex("302a300506032b6570032100") + k)
PY
openssl pkey -pubin -inform DER -in "$TMP/ota-root.der" -out "$TMP/ota-root.pem" >/dev/null 2>&1 \
  || die "invalid OTA root public key"
openssl pkeyutl -verify -pubin -inkey "$TMP/ota-root.pem" -rawin \
  -in "$TMP/manifest-body" -sigfile "$TMP/manifest.sig" >/dev/null 2>&1 \
  || die "manifest root signature verification failed"
"$TMP/gr-cli" verify-manifest --manifest "$TMP/manifest.json" --pubkey "$TMP/ota_ed25519.pk" \
  --modules-dir "$PREFIX/modules" >/dev/null

# ---------- 安装 ----------
install -m 0755 "$TMP/gr-service" "$PREFIX/bin/gr-service"
[[ -f "$TMP/gr-cli" ]] && install -m 0755 "$TMP/gr-cli" "$PREFIX/bin/gr-cli" 2>/dev/null || true
install -m 0644 "$TMP/ota_ed25519.pk" "$PREFIX/ota_ed25519.pk"
echo "$VERSION" > "$PREFIX/VERSION"
cp -f "$TMP/manifest.json" "$PREFIX/dist/release-$VERSION/manifest.json"

# FE + admin-spa 安全解包 (R-05: 拒路径逃逸/符号链接)
safe_extract_tar_gz() {
  local tgz="$1" dest="$2"
  local listing vlist
  listing="$(tar -tzf "$tgz")"
  while IFS= read -r path; do
    [[ -z "$path" ]] && continue
    # 仅拒绝真正的路径逃逸: 绝对路径 / ../ 目录段 (注意 fe 包含 [[...path]] 模板名, 不能误伤)
    if [[ "$path" == /* ]] || [[ "$path" == ../* ]] || [[ "$path" == *"/../"* ]] || [[ "$path" == *"/.." ]]; then
      die "tar entry rejected (path escape): $path"
    fi
  done <<<"$listing"
  if vlist="$(tar -tvzf "$tgz" 2>/dev/null)"; then
    while IFS= read -r line; do
      local t="${line#"${line%%[![:space:]]*}"}"
      if [[ "$t" == l* ]] || [[ "$line" == *" -> "* ]]; then
        die "tar entry rejected (symlink): $line"
      fi
    done <<<"$vlist"
  fi
  mkdir -p "$dest"
  tar -xzf "$tgz" -C "$dest"
}
if [[ -f "$TMP/fe.tgz" ]]; then
  FE_SHA="$(manifest_val '["fe"]["sha256"]')"
  if [[ -n "$FE_SHA" ]]; then
    [[ "$(sha256sum "$TMP/fe.tgz" | awk '{print $1}')" == "$FE_SHA" ]] || die "FE sha256 mismatch"
  fi
  safe_extract_tar_gz "$TMP/fe.tgz" "$TMP"
  # Re-run safe: replace any previous extraction (mv -f onto a non-empty dir fails).
  [[ -d "$TMP/fe" ]] && { rm -rf "$PREFIX/fe"; mv -f "$TMP/fe" "$PREFIX/fe"; }
fi
if [[ -f "$TMP/admin-spa.tgz" ]]; then
  safe_extract_tar_gz "$TMP/admin-spa.tgz" "$TMP"
  [[ -d "$TMP/admin-spa" ]] && { rm -rf "$PREFIX/admin-spa"; mv -f "$TMP/admin-spa" "$PREFIX/admin-spa"; }
fi

# .env (chmod 600)
cat > "$PREFIX/.env" <<EOF
# Green V7 v${VERSION}  — 管理面/业务面/缓存 连接配置
# 8.0 起 GR_* 为主名; GV5_/GV6_ 别名行同值保留 (7.x 二进制兼容, docs/13 §3)
# 业务面 (探测/分析)
GR_DATABASE_URL=${GR_DATABASE_URL}
GV6_DATABASE_URL=${GR_DATABASE_URL}
GR_BIZ_DATABASE_URL=${GR_BIZ_DATABASE_URL}
GV6_BIZ_DATABASE_URL=${GR_BIZ_DATABASE_URL}
GR_ASSOCIATION_DATABASE_URL=${GR_ASSOCIATION_DATABASE_URL}
GV6_ASSOCIATION_DATABASE_URL=${GR_ASSOCIATION_DATABASE_URL}
# 管理面 (面板/审计/集群 OTA)
GR_ADMIN_DATABASE_URL=${GR_ADMIN_DATABASE_URL}
GV6_ADMIN_DATABASE_URL=${GR_ADMIN_DATABASE_URL}
# 缓存 (可选)
GR_REDIS_URL=${GR_REDIS_URL}
GV5_REDIS_URL=${GR_REDIS_URL}
# 面板登录（本地管理员，无需官网账号）
GR_ADMIN_USER=${GR_ADMIN_USER}
GV5_ADMIN_USER=${GR_ADMIN_USER}
# 运行参数 (默认仅本机回环监听)
GR_BIND=127.0.0.1:28680
GV6_BIND=127.0.0.1:28680
GR_PROBE_BIND=127.0.0.1:28765
GV6_PROBE_BIND=127.0.0.1:28765
GR_GATEWAY_BIND=127.0.0.1:28766
GV6_GATEWAY_BIND=127.0.0.1:28766
GR_ANALYZE_WORKERS=${GR_ANALYZE_WORKERS}
GV6_ANALYZE_WORKERS=${GR_ANALYZE_WORKERS}
GR_CLUSTER_KEY=${GR_CLUSTER_KEY}
GV6_CLUSTER_KEY=${GR_CLUSTER_KEY}
GR_RESULT_TOKEN=${GR_RESULT_TOKEN}
GV6_RESULT_TOKEN=${GR_RESULT_TOKEN}
GR_OPS_TOKEN=${GR_OPS_TOKEN}
GV6_OPS_TOKEN=${GR_OPS_TOKEN}
GR_PUBKEY_PATH=${PREFIX}/ota_ed25519.pk
GV6_PUBKEY_PATH=${PREFIX}/ota_ed25519.pk
GR_DATA_DIR=${PREFIX}/data
GV6_DATA_DIR=${PREFIX}/data
GR_MODULES_DIR=${PREFIX}/modules
GV6_MODULES_DIR=${PREFIX}/modules
GR_ADMIN_SPA=${PREFIX}/admin-spa
GV6_ADMIN_SPA=${PREFIX}/admin-spa
GR_STATIC_DIR=${PREFIX}/fe
GV6_STATIC_DIR=${PREFIX}/fe
EOF
# Cookie transport flags are optional: only persist when explicitly supplied so
# production installs keep the 8.0 Secure-by-default behavior.
if [[ -n "${GR_COOKIE_SECURE:-}" ]]; then
  printf 'GR_COOKIE_SECURE=%s\nGV6_COOKIE_SECURE=%s\n' "$GR_COOKIE_SECURE" "$GR_COOKIE_SECURE" >> "$PREFIX/.env"
fi
if [[ -n "${GR_DEV_INSECURE_COOKIE:-}" ]]; then
  printf 'GR_DEV_INSECURE_COOKIE=%s\nGV6_DEV_INSECURE_COOKIE=%s\n' "$GR_DEV_INSECURE_COOKIE" "$GR_DEV_INSECURE_COOKIE" >> "$PREFIX/.env"
fi
chmod 600 "$PREFIX/.env"

# ---------- gr-cli install: data dir + console 引导 (需先载入 .env 的 DSN) ----------
say "initializing data dir..."
( set -a; # shellcheck disable=SC1091
  source "$PREFIX/.env"; set +a
  "$PREFIX/bin/gr-cli" install --data-dir "$PREFIX/data" ) | tee "$TMP/gr-cli-install.out"
CONSOLE="$(grep -oE 'console_path=/?[A-Za-z0-9_/-]+' "$TMP/gr-cli-install.out" | head -1 | cut -d= -f2-)"
[[ -n "$CONSOLE" ]] || CONSOLE="<console from gr-cli install output>"

# ---------- 模块 stage(验签) + activate ----------
if [[ -x "$PREFIX/bin/gr-cli" ]]; then
  [[ "$(python3 -c 'import json,sys; print(len(json.load(open(sys.argv[1])).get("modules", [])))' "$TMP/manifest.json")" == "6" ]] \
    || die "signed manifest must contain exactly six public modules"
  for name in identity brain analyze ingest edge probe_assets; do
    mkdir -p "$PREFIX/dist/release-$VERSION"
    ASSET="${name}.artifact.json"
    [[ "$PY3" == "1" ]] || die "python3 is required for signed V7 module manifests"
    if ! python3 - "$TMP/manifest.json" "$name" "$PREFIX/dist/release-$VERSION/$ASSET" <<'PY'
import json, sys
manifest, name, out = sys.argv[1:]
mods = [m for m in json.load(open(manifest)).get("modules", []) if m.get("name") == name]
if len(mods) != 1:
    raise SystemExit("required module missing or duplicated: " + name)
with open(out, "w") as f:
    json.dump(mods[0], f)
PY
    then
      die "required module $name missing from signed manifest"
    fi
    SO_NAME="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("asset",""))' "$PREFIX/dist/release-$VERSION/$ASSET")"
    [[ "$SO_NAME" == *"-${TRIPLE}.so" ]] || die "module $name asset has wrong architecture: $SO_NAME"
    curl -fsSL -o "$PREFIX/dist/release-$VERSION/$SO_NAME" "$BASE/$SO_NAME" 2>/dev/null || true
    [[ -f "$PREFIX/dist/release-$VERSION/$SO_NAME" ]] || die "module asset missing: $name/$SO_NAME"
    say "staging module $name (signature verify)..."
    "$PREFIX/bin/gr-cli" stage --modules-dir "$PREFIX/modules" \
      --pubkey "$PREFIX/ota_ed25519.pk" \
      --artifact-json "$PREFIX/dist/release-$VERSION/$ASSET" \
      --so "$PREFIX/dist/release-$VERSION/$SO_NAME" \
      --runtime-version "$VERSION" \
      || die "module $name stage failed"
    "$PREFIX/bin/gr-cli" activate --modules-dir "$PREFIX/modules" --name "$name" --version "$VERSION" \
      || die "module $name activation failed"
  done
fi

# 必填连接校验 (业务面/管理面至少主库齐备, 否则启动必失败)
for v in GR_DATABASE_URL GR_BIZ_DATABASE_URL GR_ASSOCIATION_DATABASE_URL GR_ADMIN_DATABASE_URL; do
  [[ -n "${!v:-}" ]] || die "$v 未填写 — 数据库连接信息必填 (业务面: GR_DATABASE_URL/GR_BIZ_DATABASE_URL/GR_ASSOCIATION_DATABASE_URL, 管理面: GR_ADMIN_DATABASE_URL)"
done

# ---------- systemd (配置全部走 $PREFIX/.env, 见 EnvironmentFile) ----------
if [[ "$NO_SYSTEMD" != "1" ]] && have_sudo; then
  if ! getent passwd greenv7 >/dev/null 2>&1; then
    if [[ -n "${SUDO_PASS:-}" ]]; then
      printf '%s\n' "$SUDO_PASS" | sudo -S useradd --system --home-dir "$PREFIX" --shell /usr/sbin/nologin greenv7
    else
      sudo useradd --system --home-dir "$PREFIX" --shell /usr/sbin/nologin greenv7
    fi
  fi
  if [[ -n "${SUDO_PASS:-}" ]]; then
    printf '%s\n' "$SUDO_PASS" | sudo -S chown -R greenv7:greenv7 "$PREFIX"
  else
    sudo chown -R greenv7:greenv7 "$PREFIX"
  fi
  sed "s|{PREFIX}|$PREFIX|g" "$SCRIPT_DIR/systemd/green-v7.service" > "$TMP/green-v7.service"
  if [[ -n "${SUDO_PASS:-}" ]]; then
    printf '%s\n' "$SUDO_PASS" | sudo -S sh -c "install -m 0644 \"$TMP/green-v7.service\" /etc/systemd/system/green-v7.service && systemctl daemon-reload && systemctl enable green-v7 && systemctl restart green-v7" >/dev/null
  else
    sudo sh -c "install -m 0644 \"$TMP/green-v7.service\" /etc/systemd/system/green-v7.service && systemctl daemon-reload && systemctl enable green-v7 && systemctl restart green-v7"
  fi
  say "systemd service green-v7 enabled"
else
  # 无 systemd (runner / 容器 / 无 sudo): 后台拉起并记录 PID/日志
  say "no systemd — starting gr-service in background (pid + log under $PREFIX/log)"
  mkdir -p "$PREFIX/log"
  # 重跑幂等: 先停掉本 prefix 之前的实例, 再启动新的 (健康门校验新 pid)。
  # pid 文件可能已是死进程 (上次启动失败被覆盖), 故再按二进制路径精确匹配。
  OLD_PID="$(cat "$PREFIX/log/gr-service.pid" 2>/dev/null || true)"
  if [[ -n "$OLD_PID" ]] && kill -0 "$OLD_PID" 2>/dev/null; then
    say "stopping previous instance pid=$OLD_PID"
    kill "$OLD_PID" 2>/dev/null || true
    sleep 1
    kill -0 "$OLD_PID" 2>/dev/null && kill -9 "$OLD_PID" 2>/dev/null || true
  fi
  if pkill -f "^$PREFIX/bin/gr-service" 2>/dev/null; then
    say "stopped stale instance(s) of $PREFIX/bin/gr-service"
    sleep 1
  fi
  ( set -a; # shellcheck disable=SC1091
    source "$PREFIX/.env"; set +a
    nohup "$PREFIX/bin/gr-service" >"$PREFIX/log/gr-service.log" 2>&1 &
    echo $! > "$PREFIX/log/gr-service.pid" )
fi

# ---------- 健康门 (端口取 .env GV6_BIND, 默认 127.0.0.1:28680) ----------
# 必须先证明 *本次启动的进程* 活着: 若端口被旧实例/残留进程占用, 新进程会立刻
# EADDRINUSE 退出, 而 curl 可能仍命中旧进程 → 误报安装成功 (实测复现过)。
ENV_BIND="$(grep -E '^(GR|GV6)_BIND=' "$PREFIX/.env" 2>/dev/null | head -1 | cut -d= -f2- || true)"
HEALTH="http://${ENV_BIND:-127.0.0.1:28680}/v1/health"
ok=0
for i in $(seq 1 30); do
  if curl -fsS --max-time 3 "$HEALTH" >/dev/null 2>&1; then ok=1; break; fi
  sleep 1
done
if [[ "$ok" == "1" ]]; then
  # Give a dying process a window to fail (bind error surfaces ~100ms in;
  # without this the health check can pass before EADDRINUSE exits).
  sleep 2
  if [[ "$NO_SYSTEMD" == "1" ]]; then
    NEW_PID="$(cat "$PREFIX/log/gr-service.pid" 2>/dev/null || true)"
    if [[ -n "$NEW_PID" ]] && ! kill -0 "$NEW_PID" 2>/dev/null; then
      say "health URL responded but our gr-service pid=$NEW_PID is dead — stale process on port? see $PREFIX/log/gr-service.log"
      ok=0
    fi
    if [[ "$ok" == "1" ]] && grep -aqE "Address already in use|os error 98" "$PREFIX/log/gr-service.log" 2>/dev/null; then
      say "gr-service log shows port conflict (EADDRINUSE) — health response came from a different process"
      ok=0
    fi
  elif ! systemctl is-active green-v7 >/dev/null 2>&1 \
    && ! systemctl is-active green-v6 >/dev/null 2>&1; then
    say "systemd unit green-v7/green-v6 is not active — health response came from a different process"
    ok=0
  fi
fi
[[ "$ok" == "1" ]] || die "health check failed on $HEALTH — check $PREFIX/.env DSNs and service log (port conflict: stop the stale gr-service first)"

cat > "$PREFIX/install-state.env" <<EOF
VERSION=${VERSION}
ARCH=${ARCH}
TRIPLE=${TRIPLE}
CONSOLE=${CONSOLE}
HEALTH=${HEALTH}
PREFIX=${PREFIX}
EOF

echo
echo "=============================================="
echo " Green V7 v${VERSION} 安装完成"
echo "  安装目录 : $PREFIX"
echo "  console  : http://127.0.0.1:28680${CONSOLE}"
  echo "  管理面登录: 浏览器打开上述地址 → 使用本地管理员（见 $PREFIX/data/admin/admin_bootstrap_once.txt）"
  [[ -n "${GV6_OFFICIAL_URL:-}" ]] && echo "  可选 OAuth: ${GV6_OFFICIAL_URL}"
echo "  探针引导凭据: cat $PREFIX/data/admin/admin_bootstrap_once.txt"
echo "  连接信息 : $PREFIX/.env (管理面: GR_ADMIN_DATABASE_URL / 业务面: GR_DATABASE_URL, GR_BIZ_DATABASE_URL, GR_ASSOCIATION_DATABASE_URL / 缓存: GR_REDIS_URL; 7.x 兼容别名 GV5_/GV6_ 同值保留)"
echo "  升级     : VERSION=x.y.z bash ${SCRIPT_DIR}/release/update_runtime_from_github.sh"
echo "=============================================="
