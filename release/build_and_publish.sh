#!/usr/bin/env bash
# Build modules, harden, sign, publish assets (host repo: greenpng/install).
set -euo pipefail
# 仓库根解析: 从脚本位置向上找 Cargo.toml (编号布局 / 扁平发行布局两用)
ROOT="$(cd "$(dirname "$0")" && pwd)"
while [[ "$ROOT" != "/" && ! -f "$ROOT/Cargo.toml" ]]; do ROOT="$(dirname "$ROOT")"; done
[[ -f "$ROOT/Cargo.toml" ]] || { echo "[FATAL] cannot locate workspace root (Cargo.toml) from $0" >&2; exit 1; }
cd "$ROOT"
# 布局可移植: greenpng 工作区(02-probe-analysis/...) 或扁平发行仓 gr-server(probe/ panel/ scripts/ 在根)。
# fe-<VER>.tgz 的成员前缀在两种布局下都是 probe/fe/, 内容一致。
if [[ -d "$ROOT/02-probe-analysis/probe/fe" ]]; then
  AREA="$ROOT/02-probe-analysis"
elif [[ -d "$ROOT/probe/fe" ]]; then
  AREA="$ROOT"
else
  echo "[release] cannot locate probe/fe (tried 02-probe-analysis/probe/fe and probe/fe)" >&2
  exit 1
fi
VERSION="$(cat VERSION | tr -d '[:space:]')"
OUT="$ROOT/dist/release-$VERSION"
mkdir -p "$OUT" keys

# P0-1: per-release random build id. Every release differs, so a patch derived
# from a previous binary cannot be reused. Also exported into every crate
# (GR_BUILD_ID, 旧 GV6_BUILD_ID) and written into the manifest + dist/build_id.
GR_BUILD_ID="${GR_BUILD_ID:-${GV6_BUILD_ID:-$(openssl rand -hex 12)}}"
export GR_BUILD_ID
export GV6_BUILD_ID="$GR_BUILD_ID"   # legacy compile-time symbol name
# P0-4: per-release obfuscation salt → string tables differ between releases.
GR_OBF_SALT="${GR_OBF_SALT:-${GV6_OBF_SALT:-$(openssl rand -hex 16)}}"
export GR_OBF_SALT
export GV6_OBF_SALT="$GR_OBF_SALT"   # legacy compile-time symbol name
echo -n "$GR_BUILD_ID" > "$OUT/build_id"
echo "[release] build_id=$GR_BUILD_ID (rotated per release)"

if [[ ! -f keys/ota_ed25519.sk ]]; then
  if [[ -n "${GR_OTA_SIGNING_KEY:-${GV6_OTA_SIGNING_KEY:-}}" ]]; then
    printf '%s' "${GR_OTA_SIGNING_KEY:-${GV6_OTA_SIGNING_KEY:-}}" > keys/ota_ed25519.sk
    chmod 600 keys/ota_ed25519.sk
  elif [[ "${GR_ALLOW_KEYGEN:-${GV6_ALLOW_KEYGEN:-0}}" == "1" ]]; then
    cargo run -q -p gr-cli -- keygen --out-dir keys
  else
    echo "[release] missing keys/ota_ed25519.sk (set GR_OTA_SIGNING_KEY/GV6_OTA_SIGNING_KEY or GR_ALLOW_KEYGEN=1)" >&2
    exit 1
  fi
fi

# P0-2: per-release (ephemeral) signing keypair. The root key certifies it via
# `gr-cli sign-cert`; the release secret never leaves this run and is NOT published.
# Each release uses a fresh key, so signatures from previous releases cannot be
# replayed and cracking one release does not help the next.
# Re-running the SAME version reuses the existing key (idempotent): rotating
# mid-version would make nodes that already bound the old key reject the new
# manifest and modules.
RELKEY_DIR="$OUT/keys-release"
mkdir -p "$RELKEY_DIR"
RELEASE_SK="$RELKEY_DIR/ota_ed25519.sk"
if [[ ! -f "$RELEASE_SK" ]]; then
  cargo run -q -p gr-cli -- keygen --out-dir "$RELKEY_DIR" >/dev/null
fi
RELEASE_PK_HEX="$(python3 -c "import pathlib; print(pathlib.Path(r'$RELKEY_DIR/ota_ed25519.pk').read_bytes().hex())")"
if [[ -z "$RELEASE_PK_HEX" || ! -f "$RELEASE_SK" ]]; then
  echo "[release] FAILED to generate per-release keypair" >&2
  exit 1
fi
RELEASE_CERT="$(cargo run -q -p gr-cli -- sign-cert \
  --secret-key keys/ota_ed25519.sk \
  --version "$VERSION" \
  --build-id "$GR_BUILD_ID" \
  --release-pubkey "$RELEASE_PK_HEX")"
if [[ -z "$RELEASE_CERT" || "$RELEASE_CERT" == *error* ]]; then
  echo "[release] FAILED to sign release cert: $RELEASE_CERT" >&2
  exit 1
fi
echo -n "$RELEASE_CERT" > "$OUT/release_cert.sig"
echo -n "$RELEASE_PK_HEX" > "$OUT/release_pubkey.hex"
echo "[release] per-release key rotated; cert=$(printf '%.16s' "$RELEASE_CERT")…"

# SSOT: product + FE + cool tickets all use root VERSION (7.x/8.x semver).
# Rebuild min/pin so __GV5_BUILD_IMPL__ matches VERSION — do not tar a stale stamp.
echo -n "$VERSION" > "$AREA/probe/fe/VERSION"
echo -n "$VERSION" > "$ROOT/VERSION.probe"
echo "[release] product_version SSOT=$VERSION (probe/fe/VERSION + VERSION.probe synced)"
if [[ -x "$AREA/scripts/fe/rebuild_bundles.sh" ]]; then
  echo "[release] rebuild FE bundles so pin/entry stamp == $VERSION"
  bash "$AREA/scripts/fe/rebuild_bundles.sh"
fi
export GR_RELEASE_VERSION="$VERSION"
export GR_MODULE_VERSION="$VERSION"

echo "[release] build workspace release + cdylib modules"
cargo build --release -p gr-service -p gr-cli -p gr-harden
# plugins: export gv6_module_entry only when feature=plugin (avoids static link clashes)
for p in gr-module-identity gr-module-brain gr-module-analyze gr-module-ingest gr-module-edge gr-module-probe-assets; do
  cargo build --release -p "$p" --features plugin
done

ARCH="$(uname -m)"
TRIPLE="${ARCH}-linux-gnu"
MANIFEST_MODULES="[]"

stage_mod() {
  local name="$1"
  local crate="$2"
  local so
  so="$(ls -1 target/release/lib${crate//-/_}.so 2>/dev/null | head -1 || true)"
  if [[ -z "$so" ]]; then
    # rustc names: libgr_module_identity.so
    so="$(ls -1 target/release/libgr_module_${name//_/-}.so 2>/dev/null | head -1 || true)"
  fi
  # try common patterns
  if [[ -z "$so" || ! -f "$so" ]]; then
    so=$(find target/release -maxdepth 1 -name "libgr_module_${name}*.so" | head -1 || true)
  fi
  if [[ -z "$so" || ! -f "$so" ]]; then
    echo "[release] WARN missing so for $name"
    return 0
  fi
  local asset="libgr_${name}-${VERSION}-${TRIPLE}.so"
  cp -f "$so" "$OUT/$asset"
  # R-04: .gv6m XOR envelope is NOT production security — skip by default.
  # Lab-only: ALLOW_INSECURE_GV6M=1 builds optional sidecar (requires --allow-insecure-xor).
  if [[ "${ALLOW_INSECURE_GV6M:-0}" == "1" ]]; then
    cargo run -q -p gr-harden -- --allow-insecure-xor --input "$OUT/$asset" --output "$OUT/${asset}.gv6m" || true
  fi
  local art
  art=$(cargo run -q -p gr-cli -- sign-module --name "$name" --version "$VERSION" --so "$OUT/$asset" --secret-key keys/ota_ed25519.sk --domain "$name" --release-key "$RELEASE_SK")
  echo "$art" > "$OUT/${name}.artifact.json"
  MANIFEST_MODULES=$(python3 - <<PY
import json,sys
mods=json.loads('''$MANIFEST_MODULES''')
art=json.loads('''$art''')
mods.append(art)
print(json.dumps(mods))
PY
)
  echo "[release] staged $asset"
}

for pair in identity:gr_module_identity brain:gr_module_brain analyze:gr_module_analyze ingest:gr_module_ingest edge:gr_module_edge probe_assets:gr_module_probe_assets; do
  stage_mod "${pair%%:*}" "${pair##*:}"
done

# also ship service + CLI binaries (hardened strip already via release profile)
SVC_ASSET="gr-service-${VERSION}-${TRIPLE}"
CLI_ASSET="gr-cli-${VERSION}-${TRIPLE}"
cp -f target/release/gr-service "$OUT/$SVC_ASSET"
cp -f target/release/gr-cli "$OUT/$CLI_ASSET"
chmod +x "$OUT/$SVC_ASSET" "$OUT/$CLI_ASSET"
# 公钥随产物分发 (install.sh 从 release 资产读取公钥验签, 缺失即失败)
if [[ -f keys/ota_ed25519.pk ]]; then
  cp -f keys/ota_ed25519.pk "$OUT/ota_ed25519.pk"
fi
mkdir -p "$OUT/admin-spa"
cp -f "$AREA/panel/admin-spa/"* "$OUT/admin-spa/" 2>/dev/null || true
# FE assets (product_version path + cool invalidate)
FE_ASSET=""
if [[ -d "$AREA/probe/fe" ]]; then
  FE_ASSET="fe-${VERSION}.tgz"
  if [[ -n "${GR_FE_MASTER_TGZ:-${GV6_FE_MASTER_TGZ:-}}" && -f "${GR_FE_MASTER_TGZ:-${GV6_FE_MASTER_TGZ:-}}" ]]; then
    # CI: single shared FE bundle (obfuscation is not byte-deterministic across
    # jobs) — every arch manifest must hash the SAME fe tgz
    cp -f "${GR_FE_MASTER_TGZ:-${GV6_FE_MASTER_TGZ:-}}" "$OUT/$FE_ASSET"
  else
    # -h: dereference symlinks so OTA safe extract (no symlink members) stays simple.
    tar -C "$AREA" -czhf "$OUT/$FE_ASSET" probe/fe
  fi
  echo -n "$VERSION" > "$OUT/VERSION"
  echo -n "$VERSION" > "$OUT/fe.VERSION"
fi

# R-01: full integrity manifest — runtime/FE sha256 + modules (signed) + optional manifest.sig
python3 - <<PY > "$OUT/manifest.json"
import hashlib, json, pathlib
out = pathlib.Path(r"""$OUT""")
version = """$VERSION"""
triple = """$TRIPLE"""
mods = json.loads(r'''$MANIFEST_MODULES''')
svc = out / """$SVC_ASSET"""
svc_sha = hashlib.sha256(svc.read_bytes()).hexdigest() if svc.is_file() else None
cli = out / f"gr-cli-{version}-{triple}"
cli_obj = None
if cli.is_file():
    # P1-4: signed CLI coverage — installer verifies the gr-cli helper against
    # this entry before invoking verify/stage/activate.
    cli_obj = {
        "version": version,
        "abi": 1,
        "asset": cli.name,
        "sha256": hashlib.sha256(cli.read_bytes()).hexdigest(),
    }
fe_name = """$FE_ASSET"""
fe_obj = None
if fe_name:
    fe_path = out / fe_name
    if fe_path.is_file():
        fe_obj = {
            "asset": fe_name,
            "sha256": hashlib.sha256(fe_path.read_bytes()).hexdigest(),
        }
man = {
    "product": "green-v7",
    "channel": "stable",
    "runtime": {
        "version": version,
        "abi": 1,
        "asset": """$SVC_ASSET""",
        "sha256": svc_sha,
    },
    "cli": cli_obj,
    "fe": fe_obj,
    "modules": mods,
}
print(json.dumps(man, indent=2))
PY

# Sign manifest body (R-03). P0-2: manifest binds build_id + per-release
# pubkey + root-signed cert, so downgrade/replay of older signed manifests is
# detectable and every release is tied to its own key.
cargo run -q -p gr-cli -- sign-manifest \
  --manifest "$OUT/manifest.json" \
  --secret-key keys/ota_ed25519.sk \
  --build-id "$GR_BUILD_ID" \
  --release-pubkey "$RELEASE_PK_HEX" \
  --release-cert "$RELEASE_CERT"

echo "[release] artifacts in $OUT"
ls -la "$OUT"
echo "[release] manifest head:"
head -40 "$OUT/manifest.json"

# Pack SPA dir if present (GitHub assets must be files)
if [[ -d "$OUT/admin-spa" ]]; then
  if [[ -n "${GR_ADMIN_MASTER_TGZ:-${GV6_ADMIN_MASTER_TGZ:-}}" && -f "${GR_ADMIN_MASTER_TGZ:-${GV6_ADMIN_MASTER_TGZ:-}}" ]]; then
    cp -f "${GR_ADMIN_MASTER_TGZ:-${GV6_ADMIN_MASTER_TGZ:-}}" "$OUT/admin-spa.tgz"
  else
    tar -C "$OUT" -czf "$OUT/admin-spa.tgz" admin-spa
  fi
fi

# Public assets only: files, exclude signing secret sidecars
mapfile -t RELEASE_ASSETS < <(find "$OUT" -maxdepth 1 -type f ! -name '*.key' | sort)

if [[ "${SKIP_PUBLISH:-0}" == "1" ]]; then
  echo "[release] SKIP_PUBLISH=1 — artifacts only, no upload"
  exit 0
fi

REPO="${GR_RELEASE_REPO:-${GV6_RELEASE_REPO:-greenpng/install}}"
if command -v gh >/dev/null 2>&1; then
  if gh release view "v${VERSION}" --repo "$REPO" >/dev/null 2>&1; then
    echo "[release] release v${VERSION} exists, uploading new assets (no clobber)..."
    gh release upload "v${VERSION}" "${RELEASE_ASSETS[@]}" --repo "$REPO"
  else
    echo "[release] creating release v${VERSION} on $REPO"
    gh release create "v${VERSION}" "${RELEASE_ASSETS[@]}" \
      --repo "$REPO" \
      --title "Green V7 ${VERSION}" \
      --notes "Compiled/hardened modules + runtime. No source. Verify ed25519 signatures before activate."
  fi
else
  echo "[release] gh not available; local dist only"
fi
