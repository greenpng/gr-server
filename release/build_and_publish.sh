#!/usr/bin/env bash
# Build the whole-bundle release (greenpng-<VER>-<ARCH>.tar.gz) for the HOST
# arch, sign the in-bundle manifest, emit the shared FE tarball + index + pk.
#
# Asset model (greenpng 1.0.0+): a release ships ≈5 assets —
#   greenpng-<VER>-<ARCH>.tar.gz   whole bundle (bin/ modules/ fe/ admin/
#                                 manifest.json + ota_ed25519.pk inside)
#   fe-<VER>.tgz                   shared FE tarball (FE-only hot updates)
#   manifest-index.json            arch → bundle name + sha256 (+ fe entry)
#   ota_ed25519.pk                 OTA root public key
# Legacy per-file assets (gr-service-*, libgr_*.so, admin-spa.tgz …) are gone;
# hot updates extract from the whole bundle.
#
# Env: GR_OTA_SIGNING_KEY | GR_ALLOW_KEYGEN=1, GR_FE_MASTER_TGZ (CI shared FE),
#      SKIP_PUBLISH=1 (no upload), GR_RELEASE_REPO (default greenpng/gr-server).
set -euo pipefail
# 仓库根解析: 从脚本位置向上找 Cargo.toml (编号布局 / 扁平发行布局两用)
ROOT="$(cd "$(dirname "$0")" && pwd)"
while [[ "$ROOT" != "/" && ! -f "$ROOT/Cargo.toml" ]]; do ROOT="$(dirname "$ROOT")"; done
[[ -f "$ROOT/Cargo.toml" ]] || { echo "[FATAL] cannot locate workspace root (Cargo.toml) from $0" >&2; exit 1; }
cd "$ROOT"
# 布局可移植: greenpng 工作区(02-probe-analysis/...) 或扁平发行仓 gr-server(probe/ panel/ scripts/ 在根)。
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

# P0-1: per-release random build id (rotated every release; embedded in every
# crate via GR_BUILD_ID and bound into the signed manifest).
GR_BUILD_ID="${GR_BUILD_ID:-$(openssl rand -hex 12)}"
export GR_BUILD_ID
# P0-4: per-release obfuscation salt → string tables differ between releases.
GR_OBF_SALT="${GR_OBF_SALT:-$(openssl rand -hex 16)}"
export GR_OBF_SALT
echo "[release] build_id=$GR_BUILD_ID (rotated per release)"

if [[ ! -f keys/ota_ed25519.sk ]]; then
  if [[ -n "${GR_OTA_SIGNING_KEY:-}" ]]; then
    printf '%s' "${GR_OTA_SIGNING_KEY:-}" > keys/ota_ed25519.sk
    chmod 600 keys/ota_ed25519.sk
  elif [[ "${GR_ALLOW_KEYGEN:-0}" == "1" ]]; then
    cargo run -q -p gr-cli -- keygen --out-dir keys
  else
    echo "[release] missing keys/ota_ed25519.sk (set GR_OTA_SIGNING_KEY or GR_ALLOW_KEYGEN=1)" >&2
    exit 1
  fi
fi

# P0-2: per-release (ephemeral) signing keypair; root certifies it via
# `gr-cli sign-cert`. The release secret never leaves this run and is NOT
# published. Re-running the SAME version reuses the existing key (idempotent).
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
echo "[release] per-release key rotated; cert=$(printf '%.16s' "$RELEASE_CERT")…"

# SSOT: product + FE all use root VERSION. Rebuild min/pin so the FE build
# stamp matches VERSION — do not tar a stale stamp.
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
# plugins: export gr_module_entry only when feature=plugin (avoids static link clashes)
for p in gr-module-identity gr-module-brain gr-module-analyze gr-module-ingest gr-module-edge gr-module-probe-assets; do
  cargo build --release -p "$p" --features plugin
done

ARCH="$(uname -m)"
case "$ARCH" in x86_64|amd64) ARCH=x86_64 ;; aarch64|arm64) ARCH=aarch64 ;; esac
TRIPLE="${ARCH}-linux-gnu"
BUNDLE_NAME="greenpng-${VERSION}-${ARCH}"
BUNDLE_DIR="$OUT/bundle/$BUNDLE_NAME"
rm -rf "$OUT/bundle"
mkdir -p "$BUNDLE_DIR/bin" "$BUNDLE_DIR/modules"

# ---- binaries ----
cp -f target/release/gr-service "$BUNDLE_DIR/bin/gr-service"
cp -f target/release/gr-cli "$BUNDLE_DIR/bin/gr-cli"
chmod +x "$BUNDLE_DIR/bin/gr-service" "$BUNDLE_DIR/bin/gr-cli"

# ---- modules (signed) ----
MANIFEST_MODULES="[]"
stage_mod() {
  local name="$1" crate="$2" so asset art
  crate="$2"
  so="$(ls -1 target/release/lib${crate//-/_}.so 2>/dev/null | head -1 || true)"
  if [[ -z "$so" || ! -f "$so" ]]; then
    so=$(find target/release -maxdepth 1 -name "libgr_module_${name}*.so" | head -1 || true)
  fi
  if [[ -z "$so" || ! -f "$so" ]]; then
    echo "[release] WARN missing so for $name"
    return 0
  fi
  asset="modules/libgr_${name}-${VERSION}-${TRIPLE}.so"
  cp -f "$so" "$BUNDLE_DIR/$asset"
  art=$(cargo run -q -p gr-cli -- sign-module --name "$name" --version "$VERSION" \
    --so "$BUNDLE_DIR/$asset" --secret-key keys/ota_ed25519.sk --domain "$name" \
    --release-key "$RELEASE_SK")
  MANIFEST_MODULES=$(python3 - <<PY
import json
mods=json.loads('''$MANIFEST_MODULES''')
mods.append(json.loads('''$art'''))
print(json.dumps(mods))
PY
)
  echo "[release] staged $asset"
}
for pair in identity:gr_module_identity brain:gr_module_brain analyze:gr_module_analyze ingest:gr_module_ingest edge:gr_module_edge probe_assets:gr_module_probe_assets; do
  stage_mod "${pair%%:*}" "${pair##*:}"
done

# ---- FE (expanded fe/ in bundle) + shared fe tarball ----
FE_ASSET="fe-${VERSION}.tgz"
if [[ -n "${GR_FE_MASTER_TGZ:-}" && -f "${GR_FE_MASTER_TGZ:-}" ]]; then
  cp -f "${GR_FE_MASTER_TGZ:-}" "$OUT/$FE_ASSET"
else
  # deterministic tar (mtime/uid/order pinned + gzip -n): same tree → same sha
  tar -C "$AREA" --sort=name --mtime='@0' --owner=0 --group=0 --numeric-owner \
      -cf - probe/fe | gzip -n -9 > "$OUT/$FE_ASSET"
fi
rm -rf "$BUNDLE_DIR/fe"
cp -a "$AREA/probe/fe" "$BUNDLE_DIR/fe"

# ---- admin SPA (expanded admin/) ----
rm -rf "$BUNDLE_DIR/admin"
cp -a "$AREA/panel/admin-spa" "$BUNDLE_DIR/admin"

# ---- root public key ----
if [[ -f keys/ota_ed25519.pk ]]; then
  cp -f keys/ota_ed25519.pk "$BUNDLE_DIR/ota_ed25519.pk"
fi

# ---- signed in-bundle manifest ----
python3 - <<PY > "$BUNDLE_DIR/manifest.json"
import hashlib, json, os, pathlib
version = """$VERSION"""
triple = """$TRIPLE"""
arch = """$ARCH"""
bundle_dir = pathlib.Path(r"""$BUNDLE_DIR""")
out = pathlib.Path(r"""$OUT""")
mods = json.loads(r'''$MANIFEST_MODULES''')
# NOTE: manifest module `asset` stays the SIGNED flat name (the v2 sig message
# binds it — never rewrite after signing). Physical location inside the bundle
# tree is modules/<asset>; consumers map that (gr-ota bundle_dir, install.sh,
# release_tree_verify).

def sha(p):
    return hashlib.sha256(p.read_bytes()).hexdigest()

def tree_files(root):
    root = pathlib.Path(root)
    files = {}
    for p in sorted(root.rglob("*")):
        if p.is_file() and not p.is_symlink():
            files[str(p.relative_to(root))] = sha(p)
    return files

svc = bundle_dir / "bin" / "gr-service"
cli = bundle_dir / "bin" / "gr-cli"
fe_tgz = out / f"fe-{version}.tgz"
man = {
    "product": "greenpng",
    "channel": "stable",
    "arch": arch,
    "triple": triple,
    "runtime": {"version": version, "abi": 1, "asset": "bin/gr-service", "sha256": sha(svc)},
    "cli": {"version": version, "abi": 1, "asset": "bin/gr-cli", "sha256": sha(cli)},
    "fe": {"asset": fe_tgz.name, "sha256": sha(fe_tgz)},
    "fe_tree": {"epoch": version, "files": tree_files(bundle_dir / "fe")},
    "admin_tree": {"files": tree_files(bundle_dir / "admin")},
    "modules": mods,
}
print(json.dumps(man, indent=2))
PY

# Sign manifest body (R-03) — binds build_id + per-release pubkey + cert chain.
cargo run -q -p gr-cli -- sign-manifest \
  --manifest "$BUNDLE_DIR/manifest.json" \
  --secret-key keys/ota_ed25519.sk \
  --build-id "$GR_BUILD_ID" \
  --release-pubkey "$RELEASE_PK_HEX" \
  --release-cert "$RELEASE_CERT"

# ---- deterministic bundle tarball ----
tar -C "$OUT/bundle" --sort=name --mtime='@0' --owner=0 --group=0 --numeric-owner \
    -cf - "$BUNDLE_NAME" | gzip -n -9 > "$OUT/$BUNDLE_NAME.tar.gz"
cp -f "$BUNDLE_DIR/manifest.json" "$OUT/manifest.json"
echo -n "$GR_BUILD_ID" > "$OUT/build_id"

# ---- single-arch manifest-index ----
python3 - <<PY
import hashlib, json, pathlib
version = """$VERSION"""
arch = """$ARCH"""
triple = """$TRIPLE"""
out = pathlib.Path(r"""$OUT""")
bundle = out / f"greenpng-{version}-{arch}.tar.gz"
fe = out / f"fe-{version}.tgz"
index = {
    "product": "greenpng",
    "version": version,
    "architectures": {
        arch: {
            "triple": triple,
            "bundle": bundle.name,
            "bundle_sha256": hashlib.sha256(bundle.read_bytes()).hexdigest(),
            "manifest": "manifest.json",
        }
    },
    "fe": {"asset": fe.name, "sha256": hashlib.sha256(fe.read_bytes()).hexdigest()},
}
(out / "manifest-index.json").write_text(json.dumps(index, indent=2) + "\n")
print(json.dumps(index, indent=2))
PY

# 公钥随产物分发 (install.sh 从 release 资产读取公钥验签, 缺失即失败)
if [[ -f keys/ota_ed25519.pk ]]; then
  cp -f keys/ota_ed25519.pk "$OUT/ota_ed25519.pk"
fi

echo "[release] artifacts in $OUT"
ls -la "$OUT"
echo "[release] bundle manifest head:"
head -32 "$OUT/manifest.json"

# Public assets: bundle + fe tgz + index + pk (exclude signing secrets/trees)
mapfile -t RELEASE_ASSETS < <(find "$OUT" -maxdepth 1 -type f \
  \( -name 'greenpng-*.tar.gz' -o -name 'fe-*.tgz' -o -name 'manifest-index.json' -o -name 'ota_ed25519.pk' \) | sort)

if [[ "${SKIP_PUBLISH:-0}" == "1" ]]; then
  echo "[release] SKIP_PUBLISH=1 — artifacts only, no upload"
  exit 0
fi

REPO="${GR_RELEASE_REPO:-greenpng/gr-server}"
if command -v gh >/dev/null 2>&1; then
  if gh release view "v${VERSION}" --repo "$REPO" >/dev/null 2>&1; then
    echo "[release] release v${VERSION} exists, uploading new assets (no clobber)..."
    gh release upload "v${VERSION}" "${RELEASE_ASSETS[@]}" --repo "$REPO"
  else
    echo "[release] creating release v${VERSION} on $REPO"
    gh release create "v${VERSION}" "${RELEASE_ASSETS[@]}" \
      --repo "$REPO" \
      --title "greenpng ${VERSION}" \
      --notes "Whole-bundle release. Verify manifest-index.json bundle sha256, then the in-bundle signed manifest (ed25519 root key + release cert) before install."
  fi
else
  echo "[release] gh not available; local dist only"
fi
