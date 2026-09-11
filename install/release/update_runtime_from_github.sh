#!/usr/bin/env bash
# Pull the greenpng WHOLE BUNDLE from a GitHub release, verify it, extract
# runtime/CLI/FE from inside and restart the service (rollback on health fail).
# Module .so updates should use admin panel OTA (hot); this path is for rare
# runtime / in-tree plane changes that require process restart.
#
# Usage on the server (or via SSH):
#   VERSION=1.0.1 bash release/update_runtime_from_github.sh
# Env:
#   VERSION          default: from /opt/greenpng/VERSION or required
#   INSTALL_ROOT     default: /opt/greenpng
#   RELEASE_REPO     default: greenpng/gr-server
#   REQUIRE_SHA      default: 1 — fail if manifest lacks runtime.sha256
#   HEALTH_URL       default: http://127.0.0.1:28680/v1/health
set -euo pipefail

INSTALL_ROOT="${INSTALL_ROOT:-/opt/greenpng}"
RELEASE_REPO="${RELEASE_REPO:-greenpng/gr-server}"
REQUIRE_SHA="${REQUIRE_SHA:-1}"
OTA_ROOT_PUBKEY_SHA256="${GR_OTA_ROOT_PUBKEY_SHA256:-4a2296d33e66838d8a8cbd697a686bfb79c93a3d7da8a8f4cd60e949b297ea0d}"
HEALTH_URL="${HEALTH_URL:-http://127.0.0.1:28680/v1/health}"
VERSION="${VERSION:-}"
host_arch="$(uname -m)"
case "$host_arch" in
  x86_64|amd64) host_arch=x86_64 ;;
  aarch64|arm64) host_arch=aarch64 ;;
esac
ARCH_TRIPLE="${ARCH_TRIPLE:-${host_arch}-linux-gnu}"
if [[ -z "$VERSION" && -f "$INSTALL_ROOT/VERSION" ]]; then
  VERSION="$(tr -d '[:space:]' < "$INSTALL_ROOT/VERSION")"
fi
if [[ -z "$VERSION" ]]; then
  echo "USAGE: VERSION=x.y.z $0" >&2
  exit 2
fi
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "greenpng updater requires a X.Y.Z version" >&2; exit 2; }
# greenpng 线 = 1.x (整包制)。旧 6/7/8 线无就地升级路径 — 拒之于任何 fetch 之前。
# (unsigned-fixture 模式保留宽松版本: 夹具树用历史版本号做升级/回滚合同测试)
if [[ "${GR_ALLOW_UNSIGNED_FIXTURE:-0}" != "1" ]]; then
  [[ "${VERSION%%.*}" == "1" ]] \
    || { echo "greenpng line is 1.x only — legacy $VERSION has no in-place upgrade (reinstall via install.sh)" >&2; exit 2; }
fi

BASE="${GR_RELEASE_BASE:-https://github.com/${RELEASE_REPO}/releases/download/v${VERSION}}"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# R-05: list + reject path escapes / symlinks before extract
safe_extract_tar_gz() {
  local tgz="$1"
  local dest="$2"
  local listing
  listing="$(tar -tzf "$tgz")"
  while IFS= read -r path; do
    [[ -z "$path" ]] && continue
    # 仅拒绝真正的路径逃逸 (fe 包含 [[...path]] 模板目录名, 不能误伤 "..")
    if [[ "$path" == /* ]] || [[ "$path" == ../* ]] || [[ "$path" == *"/../"* ]] || [[ "$path" == *"/.." ]]; then
      echo "tar entry rejected (path escape): $path" >&2
      return 1
    fi
  done <<<"$listing"
  local vlist
  if vlist="$(tar -tvzf "$tgz" 2>/dev/null)"; then
    while IFS= read -r line; do
      local t="${line#"${line%%[![:space:]]*}"}"
      if [[ "$t" == l* ]] || [[ "$line" == *" -> "* ]]; then
        echo "tar entry rejected (symlink): $line" >&2
        return 1
      fi
    done <<<"$vlist"
  fi
  mkdir -p "$dest"
  tar -xzf "$tgz" -C "$dest"
}

manifest_val() { # manifest_val <json-path>
  python3 -c "import json,sys; m=json.load(open(sys.argv[1])); print(m$1 or '')" "$TMP/manifest.json" 2>/dev/null || true
}

echo "[runtime-ota] arch=$ARCH_TRIPLE base=$BASE"

# ---------- 整包: index → bundle sha → 安全解包 ----------
if [[ "${GR_ALLOW_UNSIGNED_FIXTURE:-0}" == "1" ]]; then
  case "${GR_DEPLOY_ENV:-lab}" in
    prod|production|live) echo "unsigned fixture mode is forbidden in production" >&2; exit 1 ;;
  esac
  if [[ "${GR_REQUIRE_MANIFEST_SIG:-0}" == "1" ]]; then
    echo "unsigned fixture mode is forbidden when GR_REQUIRE_MANIFEST_SIG=1" >&2
    exit 1
  fi
  echo "[runtime-ota] WARNING: unsigned fixture mode enabled" >&2
  # fixture 树: 平铺 manifest (本地测试用)
  curl -fsSL -o "$TMP/manifest.json" "$BASE/manifest-${ARCH_TRIPLE}.json" 2>/dev/null \
    || curl -fsSL -o "$TMP/manifest.json" "$BASE/manifest.json" \
    || { echo "fixture manifest missing" >&2; exit 1; }
  BUNDLE=""
else
  curl -fsSL -o "$TMP/ota_ed25519.pk" "$BASE/ota_ed25519.pk" \
    || { echo "cannot fetch ota_ed25519.pk" >&2; exit 1; }
  [[ "$(sha256sum "$TMP/ota_ed25519.pk" | awk '{print $1}')" == "$OTA_ROOT_PUBKEY_SHA256" ]] \
    || { echo "OTA root public key fingerprint mismatch" >&2; exit 1; }
  # index 缺席 (404) 不是致命错误 — 回退签名平铺树; 其余网络错误同样走回退再报错
  curl -fsSL -o "$TMP/manifest-index.json" "$BASE/manifest-index.json" 2>/dev/null || true
  if [[ -s "$TMP/manifest-index.json" ]]; then
    BUNDLE_NAME="$(python3 - "$TMP/manifest-index.json" "$host_arch" <<'PY'
import json, sys
idx = json.load(open(sys.argv[1]))
e = (idx.get("architectures") or {}).get(sys.argv[2]) or {}
print(e.get("bundle") or "")
PY
)"
    [[ -n "$BUNDLE_NAME" ]] || { echo "manifest-index has no bundle for arch=$host_arch" >&2; exit 1; }
    [[ "$BUNDLE_NAME" != */* && "$BUNDLE_NAME" != *".."* ]] || { echo "bundle name invalid: $BUNDLE_NAME" >&2; exit 1; }
    BUNDLE_SHA="$(python3 - "$TMP/manifest-index.json" "$host_arch" <<'PY'
import json, sys
idx = json.load(open(sys.argv[1]))
e = (idx.get("architectures") or {}).get(sys.argv[2]) or {}
print(e.get("bundle_sha256") or "")
PY
)"
    [[ "${#BUNDLE_SHA}" == 64 ]] || { echo "manifest-index bundle_sha256 missing/invalid" >&2; exit 1; }
    echo "[runtime-ota] fetch bundle $BUNDLE_NAME"
    curl -fsSL -o "$TMP/bundle.tar.gz" "$BASE/$BUNDLE_NAME"
    GOT_BSHA="$(sha256sum "$TMP/bundle.tar.gz" | awk '{print $1}')"
    [[ "$GOT_BSHA" == "$BUNDLE_SHA" ]] || { echo "bundle sha256 mismatch: got $GOT_BSHA expect $BUNDLE_SHA" >&2; exit 1; }
    echo "[runtime-ota] bundle sha256 verified"
    safe_extract_tar_gz "$TMP/bundle.tar.gz" "$TMP/extract"
    TOP="$(ls -1 "$TMP/extract" | head -1)"
    [[ -n "$TOP" && -f "$TMP/extract/$TOP/manifest.json" ]] || { echo "bundle layout invalid" >&2; exit 1; }
    BUNDLE="$TMP/extract/$TOP"
    cp -f "$BUNDLE/manifest.json" "$TMP/manifest.json"
  else
    # 兼容回退: 签名平铺树 (无 index — 中间格式 / 负面合同夹具)。
    # 验证链与整包完全一致 (pk 钉值 → manifest 根签名 → 逐资产 sha)。
    echo "[runtime-ota] no manifest-index.json — signed flat manifest fallback"
    curl -fsSL -o "$TMP/manifest.json" "$BASE/manifest-${ARCH_TRIPLE}.json" 2>/dev/null \
      || curl -fsSL -o "$TMP/manifest.json" "$BASE/manifest.json" \
      || { echo "cannot fetch manifest-index.json (or flat manifest) from $BASE" >&2; exit 1; }
    BUNDLE=""
  fi

  # Independent root verification of the in-bundle manifest (canonical body).
  # LEGACY body — frozen at the 1.0.7 key set (NO data_tree): 1.0.7-era
  # verifiers (panel OTA runtime path inside the 1.0.7 binary on 178, old
  # copies of this script) rebuild these exact bytes as `sig`. data_tree
  # (1.0.8+) rides the SEPARATE `sig_data` signature verified below — that
  # is what keeps panel OTA upgrades from 1.0.7 working.
  python3 - "$TMP/manifest.json" "$TMP/manifest-body" "$TMP/manifest.sig" "$TMP/manifest-data-body" "$TMP/manifest-data.sig" <<'PY'
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
# Whole-bundle (greenpng 1.0.0+): fe_tree/admin_tree signed when present.
# spec_tree joins in 1.0.2+ (must mirror the Rust canonical body exactly or
# verification of new manifests fails; old manifests without it are unaffected).
# data_tree does NOT join this body (1.0.8+): the legacy body must stay
# byte-identical to what 1.0.7-era mirrors compute — it rides `sig_data`.
for k in ("fe_tree", "admin_tree", "spec_tree"):
    if m.get(k) is not None:
        t = m[k] or {}
        entry = {"files": t.get("files") or {}}
        if t.get("epoch") is not None:
            entry["epoch"] = t["epoch"]
        body[k] = entry
if not m.get("sig"):
    raise SystemExit("manifest signature missing")
open(sys.argv[2], "wb").write(json.dumps(body, sort_keys=True, separators=(",", ":")).encode())
s = m["sig"].replace("-", "+").replace("_", "/")
open(sys.argv[3], "wb").write(base64.b64decode(s + "=" * (-len(s) % 4)))
# EXTENDED body (+ data_tree) → `sig_data` (1.0.8+): required whenever the
# manifest carries a data_tree; absent (file left unwritten) on legacy
# manifests, which skip the extra openssl check below.
if m.get("data_tree") is not None:
    t = m["data_tree"] or {}
    entry = {"files": t.get("files") or {}}
    if t.get("epoch") is not None:
        entry["epoch"] = t["epoch"]
    body["data_tree"] = entry
    if not m.get("sig_data"):
        raise SystemExit("manifest data_tree present but sig_data missing")
    open(sys.argv[4], "wb").write(json.dumps(body, sort_keys=True, separators=(",", ":")).encode())
    sd = m["sig_data"].replace("-", "+").replace("_", "/")
    open(sys.argv[5], "wb").write(base64.b64decode(sd + "=" * (-len(sd) % 4)))
PY
  python3 - "$TMP/ota_ed25519.pk" "$TMP/ota-root.der" <<'PY'
import sys
k = open(sys.argv[1], "rb").read()
if len(k) != 32: raise SystemExit("OTA root key must be 32 bytes")
open(sys.argv[2], "wb").write(bytes.fromhex("302a300506032b6570032100") + k)
PY
  openssl pkey -pubin -inform DER -in "$TMP/ota-root.der" -out "$TMP/ota-root.pem" >/dev/null 2>&1 \
    || { echo "invalid OTA root public key" >&2; exit 1; }
  openssl pkeyutl -verify -pubin -inkey "$TMP/ota-root.pem" -rawin \
    -in "$TMP/manifest-body" -sigfile "$TMP/manifest.sig" >/dev/null 2>&1 \
    || { echo "manifest root signature verification failed" >&2; exit 1; }
  # data_tree 双签名 (1.0.8+): data_tree 走 sig_data (root 钥对扩展体签名)。
  if [[ -s "$TMP/manifest-data-body" ]]; then
    openssl pkeyutl -verify -pubin -inkey "$TMP/ota-root.pem" -rawin \
      -in "$TMP/manifest-data-body" -sigfile "$TMP/manifest-data.sig" >/dev/null 2>&1 \
      || { echo "manifest data_tree signature (sig_data) verification failed" >&2; exit 1; }
    echo "[runtime-ota] manifest data_tree signature (sig_data) verified"
  fi
  echo "[runtime-ota] manifest root signature verified"
  # 产品/版本断言: 签名体绑定 product, 但消费方必须显式拒绝他线 manifest
  PROD="$(manifest_val '["product"]')"
  [[ "$PROD" == "greenpng" ]] || { echo "manifest product is '$PROD' (greenpng required)" >&2; exit 1; }
  MVER="$(manifest_val '["runtime"]["version"]')"
  [[ "$MVER" == "$VERSION" ]] || { echo "manifest runtime.version '$MVER' != requested '$VERSION'" >&2; exit 1; }
fi

# ---------- runtime / cli / fe: 取自包内 (fixture 模式走平铺资产) ----------
ASSET_SVC="$(manifest_val '["runtime"]["asset"]')"
[[ -n "$ASSET_SVC" ]] || { echo "manifest runtime.asset missing" >&2; exit 1; }
[[ "$ASSET_SVC" != /* && "$ASSET_SVC" != *..* ]] || { echo "runtime asset path invalid: $ASSET_SVC" >&2; exit 1; }

if [[ -n "$BUNDLE" ]]; then
  cp -f "$BUNDLE/$ASSET_SVC" "$TMP/gr-service"
  CLI_ASSET="$(manifest_val '["cli"]["asset"]')"
  CLI_SHA="$(manifest_val '["cli"]["sha256"]')"
  [[ -n "$CLI_ASSET" && -n "$CLI_SHA" ]] || { echo "manifest cli entry missing (signed CLI coverage required)" >&2; exit 1; }
  cp -f "$BUNDLE/$CLI_ASSET" "$TMP/gr-cli"
  chmod +x "$TMP/gr-cli"
  GOT_CLI="$(sha256sum "$TMP/gr-cli" | awk '{print $1}')"
  [[ "$GOT_CLI" == "$CLI_SHA" ]] || { echo "cli sha256 mismatch" >&2; exit 1; }
  echo "[runtime-ota] cli sha256 verified ($CLI_ASSET)"
else
  # fixture: 平铺命名
  [[ "$ASSET_SVC" == *"-${ARCH_TRIPLE}" ]] || { echo "runtime asset wrong architecture: $ASSET_SVC" >&2; exit 1; }
  curl -fsSL -o "$TMP/gr-service" "$BASE/$ASSET_SVC"
  for cand in "gr-cli-${VERSION}-${ARCH_TRIPLE}"; do
    curl -fsSL -o "$TMP/gr-cli" "$BASE/$cand" 2>/dev/null && chmod +x "$TMP/gr-cli" && break
  done
fi
chmod +x "$TMP/gr-service"

# sanity: ELF
file "$TMP/gr-service" | grep -qi ELF || {
  echo "downloaded asset is not ELF binary" >&2
  head -c 200 "$TMP/gr-service" >&2 || true
  exit 1
}
if [[ "$host_arch" == "x86_64" ]]; then
  file "$TMP/gr-service" | grep -q 'x86-64' || { echo "runtime ELF architecture mismatch" >&2; exit 1; }
elif [[ "$host_arch" == "aarch64" ]]; then
  file "$TMP/gr-service" | grep -q 'ARM aarch64' || { echo "runtime ELF architecture mismatch" >&2; exit 1; }
fi

# R-01: runtime sha256 from the signed manifest
EXPECT_SHA="$(manifest_val '["runtime"]["sha256"]')"
if [[ -n "$EXPECT_SHA" ]]; then
  GOT_SHA="$(sha256sum "$TMP/gr-service" | awk '{print $1}')"
  [[ "$GOT_SHA" == "$EXPECT_SHA" ]] || { echo "runtime sha256 mismatch: got $GOT_SHA expect $EXPECT_SHA" >&2; exit 1; }
  echo "[runtime-ota] runtime sha256 verified"
elif [[ "$REQUIRE_SHA" == "1" ]]; then
  echo "runtime sha256 missing from manifest (set REQUIRE_SHA=0 only for lab)" >&2
  exit 1
fi

echo "[runtime-ota] install into $INSTALL_ROOT/bin (backup previous)"
mkdir -p "$INSTALL_ROOT/bin" "$INSTALL_ROOT/bin/releases/${VERSION}" "$INSTALL_ROOT/dist/release-${VERSION}"
BAK=""
if [[ -x "$INSTALL_ROOT/bin/gr-service" ]]; then
  BAK="$INSTALL_ROOT/bin/gr-service.bak.$(date -u +%Y%m%dT%H%M%SZ)"
  cp -a "$INSTALL_ROOT/bin/gr-service" "$BAK" || true
fi
# versioned slot then install
install -m 0755 "$TMP/gr-service" "$INSTALL_ROOT/bin/releases/${VERSION}/gr-service"
install -m 0755 "$TMP/gr-service" "$INSTALL_ROOT/bin/gr-service"
if [[ -f "$TMP/gr-cli" ]]; then
  install -m 0755 "$TMP/gr-cli" "$INSTALL_ROOT/bin/gr-cli"
fi
cp -f "$TMP/manifest.json" "$INSTALL_ROOT/dist/release-${VERSION}/manifest.json"
# iss/audit OPR-01: VERSION stamp is DEFERRED to the health gate at the end of
# this script. The previous eager write left a new-VERSION / old-binary split
# brain whenever HEALTH_FAIL rolled the binary back: auto_upgrade_check's
# monotonic gate then read the new version as current and never retried — a
# permanently locked-out, half-upgraded host. FE/spec trees are restored from
# their .bak snapshots on rollback (see rollback_runtime_state below).

# FE: 整包内的 fe/ 树 (fe_tree 逐文件 sha) — 与共享 fe tgz 等价但免二次下载
# fe/VERSION is stamped together with VERSION after HEALTH_OK.
if [[ -n "$BUNDLE" && -d "$BUNDLE/fe" ]]; then
  python3 - "$BUNDLE" "$TMP/manifest.json" <<'PY'
import hashlib, json, pathlib, sys
bundle = pathlib.Path(sys.argv[1])
man = json.load(open(sys.argv[2]))
tree = (man.get("fe_tree") or {}).get("files") or {}
root = bundle / "fe"
bad = [rel for rel, want in tree.items()
       if not (root / rel).is_file() or hashlib.sha256((root / rel).read_bytes()).hexdigest() != want]
if bad:
    raise SystemExit(f"fe_tree sha mismatch: {bad[:3]}")
print(f"fe_tree: {len(tree)} files verified")
PY
  [[ $? -eq 0 ]] || { echo "bundle fe_tree verification failed" >&2; exit 1; }
  if [[ -d "$INSTALL_ROOT/fe" ]]; then
    FE_BAK="$INSTALL_ROOT/fe.bak.$(date -u +%Y%m%dT%H%M%SZ)"
    mv "$INSTALL_ROOT/fe" "$FE_BAK" || true
  fi
  cp -a "$BUNDLE/fe" "$INSTALL_ROOT/fe"
  echo "[runtime-ota] FE tree installed from bundle"
elif [[ -z "$BUNDLE" ]]; then
  # fixture / flat release: 共享 fe tgz 资产 + manifest fe.sha256 校验 (R-01)
  FE_TGZ="fe-${VERSION}.tgz"
  if curl -fsSL -o "$TMP/fe.tgz" "$BASE/$FE_TGZ" 2>/dev/null; then
    FE_SHA="$(manifest_val '["fe"]["sha256"]')"
    if [[ -n "$FE_SHA" ]]; then
      GOT_FE="$(sha256sum "$TMP/fe.tgz" | awk '{print $1}')"
      [[ "$GOT_FE" == "$FE_SHA" ]] || { echo "FE sha256 mismatch: got $GOT_FE expect $FE_SHA" >&2; exit 1; }
      echo "[runtime-ota] FE sha256 verified"
    elif [[ "$REQUIRE_SHA" == "1" ]]; then
      echo "FE sha256 missing from manifest (set REQUIRE_SHA=0 only for lab)" >&2
      exit 1
    fi
    STAGE="$TMP/fe-extract"
    safe_extract_tar_gz "$TMP/fe.tgz" "$STAGE"
    if [[ -d "$INSTALL_ROOT/fe" ]]; then
      FE_BAK="$INSTALL_ROOT/fe.bak.$(date -u +%Y%m%dT%H%M%SZ)"
      mv "$INSTALL_ROOT/fe" "$FE_BAK" || true
    fi
    if [[ -d "$STAGE/fe" ]]; then
      mv "$STAGE/fe" "$INSTALL_ROOT/fe"
    else
      mkdir -p "$INSTALL_ROOT/fe"
      cp -a "$STAGE"/. "$INSTALL_ROOT/fe/"
    fi
    echo "[runtime-ota] FE installed from flat asset"
  else
    echo "[runtime-ota] no $FE_TGZ on release (FE not updated this run)"
  fi
fi
# fe/VERSION stamp deferred with VERSION (OPR-01) — see stamp_version after the health gate.

# spec: 1.0.2+ 整包带 spec_tree (analyze 运行时数据); 验树→备份→换树 (与 fe 同型)。
# 1.0.0/1.0.1 包无 spec_tree → 沿用现有 spec/ 不动 (老包物理带 spec 但未签名, 不盲信)。
if [[ -n "$BUNDLE" && -d "$BUNDLE/spec" ]] && python3 -c "import json,sys; sys.exit(0 if (json.load(open('$TMP/manifest.json')).get('spec_tree') or {}).get('files') else 1)"; then
  python3 - "$BUNDLE" "$TMP/manifest.json" <<'PY'
import hashlib, json, pathlib, sys
bundle = pathlib.Path(sys.argv[1])
man = json.load(open(sys.argv[2]))
tree = (man.get("spec_tree") or {}).get("files") or {}
root = bundle / "spec"
bad = [rel for rel, want in tree.items()
       if not (root / rel).is_file() or hashlib.sha256((root / rel).read_bytes()).hexdigest() != want]
if bad:
    raise SystemExit(f"spec_tree sha mismatch: {bad[:3]}")
print(f"spec_tree: {len(tree)} files verified")
PY
  [[ $? -eq 0 ]] || { echo "bundle spec_tree verification failed" >&2; exit 1; }
  if [[ -d "$INSTALL_ROOT/spec" ]]; then
    SPEC_BAK="$INSTALL_ROOT/spec.bak.$(date -u +%Y%m%dT%H%M%SZ)"
    mv "$INSTALL_ROOT/spec" "$SPEC_BAK" || true
  fi
  cp -a "$BUNDLE/spec" "$INSTALL_ROOT/spec"
  echo "[runtime-ota] spec tree installed from bundle"
else
  echo "[runtime-ota] spec_tree not in bundle (pre-1.0.2) — keeping existing $INSTALL_ROOT/spec"
fi

# data: 1.0.8+ 整包带 data_tree (r100 模板 + geoip mmdb)。验树后 overlay 落
# $INSTALL_ROOT/data — 该目录承载运行期状态 (admin bootstrap /
# auto_upgrade_state), 只覆盖产品文件, 永不整树删除 (与 spec 换树不同型)。
if [[ -n "$BUNDLE" && -d "$BUNDLE/data" ]] && python3 -c "import json,sys; sys.exit(0 if (json.load(open('$TMP/manifest.json')).get('data_tree') or {}).get('files') else 1)"; then
  python3 - "$BUNDLE" "$TMP/manifest.json" <<'PY'
import hashlib, json, pathlib, sys
bundle = pathlib.Path(sys.argv[1])
man = json.load(open(sys.argv[2]))
tree = (man.get("data_tree") or {}).get("files") or {}
root = bundle / "data"
bad = [rel for rel, want in tree.items()
       if not (root / rel).is_file() or hashlib.sha256((root / rel).read_bytes()).hexdigest() != want]
if bad:
    raise SystemExit(f"data_tree sha mismatch: {bad[:3]}")
print(f"data_tree: {len(tree)} files verified")
PY
  [[ $? -eq 0 ]] || { echo "bundle data_tree verification failed" >&2; exit 1; }
  mkdir -p "$INSTALL_ROOT/data/geo"
  [[ -f "$BUNDLE/data/r100_templates.json" ]] && install -m 0644 "$BUNDLE/data/r100_templates.json" "$INSTALL_ROOT/data/r100_templates.json"
  for _m in dbip-asn-lite.mmdb dbip-country-lite.mmdb; do
    [[ -f "$BUNDLE/data/geo/$_m" ]] && install -m 0644 "$BUNDLE/data/geo/$_m" "$INSTALL_ROOT/data/geo/$_m"
  done
  echo "[runtime-ota] data tree installed (overlay onto $INSTALL_ROOT/data)"
else
  echo "[runtime-ota] data_tree not in bundle (pre-1.0.8) — keeping existing $INSTALL_ROOT/data"
fi

# Point OTA module channel at this tag (panel still installs so separately).
# 178 v1.0.6 incident: a local http:// mirror base was written into the prod
# .env and the R-02 boot guard then refused to start the binary (crash loop
# until .env was repaired by hand). Production .env only ever receives an
# https:// pointer; other bases (lab mirror, file) are runtime-only and are
# NOT persisted — the health gate below still validates the running process.
ENVF="$INSTALL_ROOT/.env"
OLD_RELEASE_URL=""
if [[ -f "$ENVF" ]]; then
  # iss/audit OPR-01: snapshot the previous release pointer so the rollback
  # path can restore it together with the binary/fe/spec trees.
  OLD_RELEASE_URL="$(grep -m1 '^GR_RELEASE_URL=' "$ENVF" 2>/dev/null | cut -d= -f2- || true)"
  if [[ "$BASE" == https://* ]]; then
    python3 - "$ENVF" "$BASE" <<'PYENV'
import sys
p, base = sys.argv[1], sys.argv[2]
lines = [l for l in open(p).read().splitlines(keepends=True) if not l.startswith(("GR_RELEASE_URL=",))]
lines.append(f"GR_RELEASE_URL={base}\n")
open(p, "w").writelines(lines)
PYENV
  else
    echo "[runtime-ota] non-https base — GR_RELEASE_URL in .env left untouched (http pointer would fail the R-02 prod boot guard)" >&2
  fi
fi

# iss/audit OPR-01 helpers: full-state rollback + gated version stamp.
# rollback_runtime_state restores fe/spec trees from their .bak snapshots and
# the .env release pointer (binary restore stays at each call site, which owns
# $BAK). stamp_runtime_version writes VERSION + fe/VERSION — ONLY after the
# health gate confirms the new binary serves.
rollback_runtime_state() {
  if [[ -n "${FE_BAK:-}" && -d "${FE_BAK:-}" ]]; then
    rm -rf "$INSTALL_ROOT/fe"
    mv "$FE_BAK" "$INSTALL_ROOT/fe" || true
    echo "[runtime-ota] rollback: fe tree restored from ${FE_BAK}" >&2
  fi
  if [[ -n "${SPEC_BAK:-}" && -d "${SPEC_BAK:-}" ]]; then
    rm -rf "$INSTALL_ROOT/spec"
    mv "$SPEC_BAK" "$INSTALL_ROOT/spec" || true
    echo "[runtime-ota] rollback: spec tree restored from ${SPEC_BAK}" >&2
  fi
  if [[ -n "$OLD_RELEASE_URL" && -f "$ENVF" ]]; then
    python3 - "$ENVF" "$OLD_RELEASE_URL" <<'PYENV'
import sys
p, url = sys.argv[1], sys.argv[2]
lines = [l for l in open(p).read().splitlines(keepends=True) if not l.startswith(("GR_RELEASE_URL=",))]
lines.append(f"GR_RELEASE_URL={url}\n")
open(p, "w").writelines(lines)
PYENV
    echo "[runtime-ota] rollback: GR_RELEASE_URL restored" >&2
  fi
}

stamp_runtime_version() {
  echo "$VERSION" > "$INSTALL_ROOT/VERSION"
  [[ -d "$INSTALL_ROOT/fe" ]] && echo "$VERSION" > "$INSTALL_ROOT/fe/VERSION"
  echo "[runtime-ota] version stamped: $VERSION (health-gated)" >&2
}

# 属主修正: 本脚本常以 root 跑, cp -a / install 换入的树会保持 root 属主,
# 而服务进程跑在专用用户下 (User=greenpng) → 写不了 fe/OPAQUE_MAP.json →
# /dist/<hash>.min.js 全部 404, 收集器包加载失败 (v1.0.2 178 实测回归);
# bin/releases/<v>/ 与 gr-service.bak.* 若留 root 属主, 下一次服务用户发起的
# 面板 OTA (install-runtime 写版本槽 + .bak 回滚副本) 直接 EACCES
# (v1.0.8 178 实测 os error 13) → bin 整树递归归还服务用户。
# systemd 部署时把换入的树/文件归还服务用户。
if [[ "$(id -u)" == "0" && -d "$INSTALL_ROOT" ]]; then
  SVC_USER="$(systemctl show greenpng -p User --value 2>/dev/null || true)"
  SVC_GROUP="$(systemctl show greenpng -p Group --value 2>/dev/null || true)"
  if [[ -n "$SVC_USER" && "$SVC_USER" != "root" && "$SVC_USER" != "0" ]]; then
    [[ -z "$SVC_GROUP" || "$SVC_GROUP" == "root" ]] && SVC_GROUP="$SVC_USER"
    chown -R "$SVC_USER:$SVC_GROUP" "$INSTALL_ROOT/bin" 2>/dev/null || true
    chown -R "$SVC_USER:$SVC_GROUP" "$INSTALL_ROOT/fe" 2>/dev/null || true
    [[ -d "$INSTALL_ROOT/spec" ]] && chown -R "$SVC_USER:$SVC_GROUP" "$INSTALL_ROOT/spec" 2>/dev/null || true
    chown "$SVC_USER:$SVC_GROUP" "$INSTALL_ROOT/VERSION" 2>/dev/null || true
    # data overlay 产品文件 (r100 模板 / geoip mmdb): root 落的文件归还服务
    # 用户, 之后 boot 自举 / 幂等重装才能以服务身份覆写。
    chown "$SVC_USER:$SVC_GROUP" "$INSTALL_ROOT/data/r100_templates.json" 2>/dev/null || true
    for _m in dbip-asn-lite.mmdb dbip-country-lite.mmdb; do
      chown "$SVC_USER:$SVC_GROUP" "$INSTALL_ROOT/data/geo/$_m" 2>/dev/null || true
    done
    # Module OTA trees: root-run `gr-cli module update` writes modules/staging,
    # modules/versions/<m>/<v> and modules/active markers; the service user must
    # own them or the next service-user CLI/panel OTA run fails with io EACCES
    # and the service cannot read root-600 active markers (v1.0.4 178 hit both).
    [[ -d "$INSTALL_ROOT/modules" ]] && chown -R "$SVC_USER:$SVC_GROUP" "$INSTALL_ROOT/modules" 2>/dev/null || true
    # Release manifests under dist/release-*/ are read by module OTA (binding +
    # rollback lookups); root-run updaters/CLIs create them root-owned.
    for d in "$INSTALL_ROOT"/dist/release-*; do
      [[ -d "$d" ]] && chown -R "$SVC_USER:$SVC_GROUP" "$d" 2>/dev/null || true
    done
    # gr-cli whole-bundle cache (temp-dir default). A root-run CLI leaves it
    # root-owned and blocks every later service-user module update. Best-effort.
    [[ -d /tmp/gr-ota-bundle ]] && chown -R "$SVC_USER:$SVC_GROUP" /tmp/gr-ota-bundle 2>/dev/null || true
    echo "[runtime-ota] installed trees chowned → $SVC_USER (bin/fe/spec/data/modules/dist/VERSION/cache)"
  fi
fi

echo "[runtime-ota] restart greenpng + health gate"
UNIT="greenpng"
if systemctl is-enabled "$UNIT" >/dev/null 2>&1 || systemctl cat "$UNIT" >/dev/null 2>&1; then
  systemctl restart "$UNIT"
  ok=0
  for i in 1 2 3 4 5 6 7 8; do
    sleep 2
    if curl -fsS "$HEALTH_URL" >/tmp/gr-runtime-ota-health.json 2>/dev/null; then
      ok=1
      break
    fi
  done
  if [[ "$ok" != "1" ]]; then
    echo "[runtime-ota] HEALTH_FAIL — rolling back binary" >&2
    if [[ -n "$BAK" && -x "$BAK" ]]; then
      install -m 0755 "$BAK" "$INSTALL_ROOT/bin/gr-service"
      rollback_runtime_state
      systemctl restart "$UNIT" || true
      sleep 3
      if curl -fsS "$HEALTH_URL" >/tmp/gr-runtime-ota-health-rollback.json 2>/dev/null; then
        echo "[runtime-ota] ROLLBACK_HEALTH_OK" >&2
      else
        echo "[runtime-ota] ROLLBACK_HEALTH_FAIL" >&2
      fi
    else
      rollback_runtime_state
    fi
    # VERSION intentionally untouched: it was never advanced before the gate,
    # so auto_upgrade_check still sees the old version and retries the target.
    exit 1
  fi
  systemctl is-active "$UNIT"
  echo "[runtime-ota] HEALTH_OK"
  stamp_runtime_version
else
  # 无 systemd (runner / 容器 / lab): 仅管理 install.sh 后台拉起的实例
  # (pid 文件存在时)。夹具树 / 未由本机管理的安装保持旧行为 (仅提示)。
  PIDFILE="$INSTALL_ROOT/log/gr-service.pid"
  if [[ -f "$PIDFILE" && -f "$ENVF" ]]; then
    OLD_PID="$(cat "$PIDFILE" 2>/dev/null || true)"
    # 竞态实测 (v1.0.0→v1.0.1 gate-b B3): 旧 all-in-one 实例 TERM 后 >1s
    # 才退出; 旧实现仅 sleep 1 即启新实例 → 新实例绑 28766/28765 后在
    # 28680 EADDRINUSE 死亡, 健康门 curl 命中未死透的旧实例 → 假阳性
    # HEALTH_OK, 随后旧实例退出 → 全端口无人监听。
    stop_bg_instance() { # TERM → 等退出 (≤10s) → KILL 兜底
      local pid="${1:-}"
      [[ -n "$pid" && -d "/proc/$pid" ]] || return 0
      kill "$pid" 2>/dev/null || true
      local i
      for i in $(seq 1 20); do
        [[ -d "/proc/$pid" ]] || return 0
        sleep 0.5
      done
      kill -9 "$pid" 2>/dev/null || true
      sleep 0.5
    }
    wait_health_port_free() { # 健康口无人应答才算释放 (兜底非本pid持有者)
      local i
      for i in $(seq 1 20); do
        curl -fsS --max-time 2 "$HEALTH_URL" >/dev/null 2>&1 || return 0
        sleep 0.5
      done
      return 1
    }
    stop_bg_instance "$OLD_PID"
    wait_health_port_free || echo "[runtime-ota] WARN: health port still answering after stop — proceeding" >&2
    pkill -9 -f "^$INSTALL_ROOT/bin/gr-service" 2>/dev/null || true
    sleep 0.5
    ( set -a; # shellcheck disable=SC1091
      source "$ENVF"; set +a
      nohup "$INSTALL_ROOT/bin/gr-service" >>"$INSTALL_ROOT/log/gr-service.log" 2>&1 &
      echo $! > "$PIDFILE" )
    NEW_PID="$(cat "$PIDFILE" 2>/dev/null || echo '')"
    ok=0
    for i in 1 2 3 4 5 6 7 8; do
      sleep 2
      # 新实例必须活着: 死了立即判失败 (不再被旧实例残留响应骗过)
      if [[ -n "$NEW_PID" ]] && ! [[ -d "/proc/$NEW_PID" ]]; then
        echo "[runtime-ota] new instance pid=$NEW_PID exited early" >&2
        break
      fi
      if curl -fsS "$HEALTH_URL" >/tmp/gr-runtime-ota-health.json 2>/dev/null; then
        ok=1; break
      fi
    done
    if [[ "$ok" != "1" ]]; then
      echo "[runtime-ota] HEALTH_FAIL — rolling back binary" >&2
      if [[ -n "$BAK" && -x "$BAK" ]]; then
        install -m 0755 "$BAK" "$INSTALL_ROOT/bin/gr-service"
        rollback_runtime_state
        stop_bg_instance "$NEW_PID"
        wait_health_port_free || true
        pkill -9 -f "^$INSTALL_ROOT/bin/gr-service" 2>/dev/null || true
        ( set -a; source "$ENVF"; set +a
          nohup "$INSTALL_ROOT/bin/gr-service" >>"$INSTALL_ROOT/log/gr-service.log" 2>&1 &
          echo $! > "$PIDFILE" )
        sleep 3
        if curl -fsS "$HEALTH_URL" >/tmp/gr-runtime-ota-health-rollback.json 2>/dev/null; then
          echo "[runtime-ota] ROLLBACK_HEALTH_OK" >&2
        else
          echo "[runtime-ota] ROLLBACK_HEALTH_FAIL" >&2
        fi
      else
        rollback_runtime_state
      fi
      # VERSION never advanced before the gate — auto-upgrade retries the target.
      exit 1
    fi
    echo "[runtime-ota] HEALTH_OK (background instance pid=$(cat "$PIDFILE" 2>/dev/null || echo '?'))"
    stamp_runtime_version
  else
    echo "WARN: greenpng unit not found; binary installed, restart manually" >&2
    # Unmanaged instance (no systemd, no pidfile): no health gate will ever
    # run, so stamp the installed version to keep the old eager-write semantics
    # for this path (auto_upgrade_check itself only runs under systemd).
    stamp_runtime_version
  fi
fi

echo "[runtime-ota] done VERSION=$VERSION"
