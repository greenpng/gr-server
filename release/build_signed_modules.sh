#!/usr/bin/env bash
# Build + sign business modules for a release version (default: root VERSION).
# Does not rebuild full gr-service (keeps machine load low).
# Hardened flow: double-sign (root + per-release key) and emit a manifest bound
# to build_id/release_pubkey/release_cert (see 07-HARDENING.md).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
VERSION="${1:-$(tr -d '[:space:]' < VERSION)}"
OUT="$ROOT/dist/release-$VERSION"
JOBS="${CARGO_BUILD_JOBS:-2}"
TRIPLE="$(uname -m)-linux-gnu"
mkdir -p "$OUT" keys

if [[ ! -f keys/ota_ed25519.sk ]]; then
  if [[ -n "${GR_OTA_SIGNING_KEY:-${GV6_OTA_SIGNING_KEY:-}}" ]]; then
    printf '%s' "${GR_OTA_SIGNING_KEY:-${GV6_OTA_SIGNING_KEY:-}}" > keys/ota_ed25519.sk
    chmod 600 keys/ota_ed25519.sk
  elif [[ "${GR_ALLOW_KEYGEN:-${GV6_ALLOW_KEYGEN:-0}}" == "1" ]]; then
    cargo run -q -p gr-cli --jobs "$JOBS" -- keygen --out-dir keys
  else
    echo "[release] missing keys/ota_ed25519.sk (set GR_OTA_SIGNING_KEY/GV6_OTA_SIGNING_KEY or GR_ALLOW_KEYGEN=1)" >&2
    exit 1
  fi
fi
cp -f keys/ota_ed25519.pk "$OUT/ota_ed25519.pk"

# P0-1/P0-4: same release identity on re-runs (build_id + obf salt + release key).
if [[ -f "$OUT/build_id" ]]; then
  export GR_BUILD_ID="$(tr -d '[:space:]' < "$OUT/build_id")"
fi
GR_BUILD_ID="${GR_BUILD_ID:-${GV6_BUILD_ID:-$(openssl rand -hex 12)}}"
export GR_BUILD_ID
export GV6_BUILD_ID="$GR_BUILD_ID"   # legacy compile-time symbol name
GR_OBF_SALT="${GR_OBF_SALT:-${GV6_OBF_SALT:-$(openssl rand -hex 16)}}"
export GR_OBF_SALT
export GV6_OBF_SALT="$GR_OBF_SALT"   # legacy compile-time symbol name
RELKEY_DIR="$OUT/keys-release"
RELEASE_SK="$RELKEY_DIR/ota_ed25519.sk"
if [[ ! -f "$RELEASE_SK" ]]; then
  mkdir -p "$RELKEY_DIR"
  cargo run -q -p gr-cli --jobs "$JOBS" -- keygen --out-dir "$RELKEY_DIR" >/dev/null
fi
RELEASE_PK_HEX="$(python3 -c "import pathlib; print(pathlib.Path(r'$RELKEY_DIR/ota_ed25519.pk').read_bytes().hex())")"
RELEASE_CERT="$(cargo run -q -p gr-cli -- sign-cert \
  --secret-key keys/ota_ed25519.sk \
  --version "$VERSION" --build-id "$GR_BUILD_ID" --release-pubkey "$RELEASE_PK_HEX")"
echo -n "$GR_BUILD_ID" > "$OUT/build_id"
echo -n "$RELEASE_CERT" > "$OUT/release_cert.sig"
echo -n "$RELEASE_PK_HEX" > "$OUT/release_pubkey.hex"

echo "[release] build plugin .so (jobs=$JOBS) version=$VERSION build_id=$GR_BUILD_ID"
export GR_RELEASE_VERSION="$VERSION"
export GR_MODULE_VERSION="$VERSION"
for p in gr-module-identity gr-module-brain gr-module-analyze gr-module-ingest gr-module-edge gr-module-probe-assets; do
  cargo build --release -p "$p" --features plugin --jobs "$JOBS"
done

# map name -> so pattern
mods=(
  "identity:libgr_module_identity.so"
  "brain:libgr_module_brain.so"
  "analyze:libgr_module_analyze.so"
  "ingest:libgr_module_ingest.so"
  "edge:libgr_module_edge.so"
  "probe_assets:libgr_module_probe_assets.so"
)

MANIFEST_MODULES='[]'
for pair in "${mods[@]}"; do
  name="${pair%%:*}"
  so_name="${pair##*:}"
  src=$(find target/release -maxdepth 1 -name "$so_name" | head -1 || true)
  if [[ -z "$src" || ! -f "$src" ]]; then
    echo "[release] WARN missing $so_name"
    continue
  fi
  asset="libgr_${name}-${VERSION}-${TRIPLE}.so"
  cp -f "$src" "$OUT/$asset"
  art=$(cargo run -q -p gr-cli --jobs "$JOBS" -- sign-module \
    --name "$name" --version "$VERSION" --so "$OUT/$asset" \
    --secret-key keys/ota_ed25519.sk --domain "$name" \
    --release-key "$RELEASE_SK")
  echo "$art" >"$OUT/${name}.artifact.json"
  MANIFEST_MODULES=$(python3 -c "import json; m=json.loads('''$MANIFEST_MODULES'''); m.append(json.loads('''$art''')); print(json.dumps(m))")
  echo "[release] signed $asset (double-signed)"
done

python3 - <<PY >"$OUT/manifest.json"
import json
mods=json.loads('''$MANIFEST_MODULES''')
print(json.dumps({
  "product": "green-v7",
  "channel": "stable",
  "build_id": "$GR_BUILD_ID",
  "release_pubkey": "$RELEASE_PK_HEX",
  "release_cert": "$RELEASE_CERT",
  "runtime": {"version": "$VERSION", "abi": 1},
  "modules": mods
}, indent=2))
PY
cargo run -q -p gr-cli -- sign-manifest --manifest "$OUT/manifest.json" \
  --secret-key keys/ota_ed25519.sk \
  --build-id "$GR_BUILD_ID" \
  --release-pubkey "$RELEASE_PK_HEX" \
  --release-cert "$RELEASE_CERT"

# ship admin spa snapshot
mkdir -p "$OUT/admin-spa"
cp -a 02-probe-analysis/panel/admin-spa/. "$OUT/admin-spa/" 2>/dev/null || true

echo "[release] artifacts in $OUT"
ls -la "$OUT" | head -40

REPO="${GR_RELEASE_REPO:-${GV6_RELEASE_REPO:-greenpng/install}}"
if [[ "${PUBLISH:-1}" == "1" ]] && command -v gh >/dev/null; then
  if gh release view "v${VERSION}" --repo "$REPO" >/dev/null 2>&1; then
    echo "[release] upload to existing v${VERSION}"
    gh release upload "v${VERSION}" "$OUT"/* --repo "$REPO"
  else
    echo "[release] create v${VERSION}"
    gh release create "v${VERSION}" "$OUT"/* \
      --repo "$REPO" \
      --title "Green V7 ${VERSION}" \
      --notes "Signed business modules for Green V7. Verify ed25519 chain (root ota_ed25519.pk + release cert) + sha256 before activate. Runtime ABI 1."
  fi
  echo "[release] published https://github.com/${REPO}/releases/tag/v${VERSION}"
else
  echo "[release] local only (PUBLISH=0 or no gh)"
fi
