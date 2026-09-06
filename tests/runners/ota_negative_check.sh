#!/usr/bin/env bash
# V7 OTA manifest-verification NEGATIVE tests.
#
# Targets the canonical updater 04-release-github-ci/release/update_runtime_from_github.sh, which
# enforces (in order):
#   - V7 major version (VERSION must match ^7\.[0-9]+\.[0-9]+$)
#   - signed manifest present
#   - OTA root public key fingerprint (sha256 of ota_ed25519.pk == pinned)
#   - manifest root ed25519 signature (canonical body, openssl pkeyutl)
#   - runtime asset architecture (asset name suffix == host triple)
#   - runtime ELF architecture
#   - runtime asset sha256 (R-01)
#
# Scenarios — each must be REJECTED and must NOT mutate the install root:
#   1. V7 major enforcement        — VERSION 6.x / 8.x rejected before any fetch
#   2. tampered manifest           — body changed after signing → sig mismatch
#   3. missing signature           — manifest.sig absent
#   4. invalid signature           — manifest.sig is well-formed base64 garbage
#   5. wrong root key / key subst. — served pubkey fingerprint != pinned
#   6. hash asset tampering        — signed manifest, but asset bytes != sha
#   7. architecture mismatch       — manifest runtime.asset named for other arch
#   8. production unsigned bypass  — fixture escape hatch is hard-disabled
#
# A positive baseline (good signed release) MUST install first, proving the
# rejections above are meaningful (not "everything fails").
#
# Fixture manifests are signed with the OTA root SECRET key, so this runs where
# keys/ota_ed25519.sk is available (local / private runners). It skips cleanly
# (exit 0) when the key or python cryptography is absent (e.g. fresh public
# checkout), so it never breaks a runner that cannot sign fixtures.
set -euo pipefail

# 仓库根解析: 从脚本位置向上找 Cargo.toml (编号布局 / 扁平发行布局两用)
ROOT="$(cd "$(dirname "$0")" && pwd)"
while [[ "$ROOT" != "/" && ! -f "$ROOT/Cargo.toml" ]]; do ROOT="$(dirname "$ROOT")"; done
[[ -f "$ROOT/Cargo.toml" ]] || { echo "[FATAL] cannot locate workspace root (Cargo.toml) from $0" >&2; exit 1; }
UPDATER="${UPDATER:-$ROOT/04-release-github-ci/release/update_runtime_from_github.sh}"
SK="$ROOT/keys/ota_ed25519.sk"
PK="$ROOT/keys/ota_ed25519.pk"

# Skip gracefully when we cannot sign fixtures.
if [[ ! -f "$SK" ]]; then
  echo "SKIP: $SK not present (OTA root secret key required to sign fixtures)" >&2
  exit 0
fi
python3 -c "import cryptography" 2>/dev/null \
  || { echo "SKIP: python cryptography not installed" >&2; exit 0; }
[[ -f "$UPDATER" ]] || { echo "missing updater: $UPDATER" >&2; exit 1; }
[[ -f "$PK" ]]     || { echo "missing public key: $PK" >&2; exit 1; }

TMP="$(mktemp -d /tmp/v7-ota-neg.XXXXXX)"
PORT="$((19000 + RANDOM % 1000))"
SERVER_PID=""
cleanup() { [[ -n "$SERVER_PID" ]] && kill "$SERVER_PID" 2>/dev/null || true; rm -rf "$TMP"; }
trap cleanup EXIT

host_arch="$(uname -m)"
case "$host_arch" in
  x86_64|amd64) host_arch=x86_64 ;;
  aarch64|arm64) host_arch=aarch64 ;;
  *) echo "unsupported host architecture: $host_arch" >&2; exit 1 ;;
esac
TRIPLE="${host_arch}-linux-gnu"
case "$host_arch" in
  x86_64)   OTHER_TRIPLE="aarch64-linux-gnu" ;;
  aarch64)  OTHER_TRIPLE="x86_64-linux-gnu" ;;
esac

INST="$TMP/install"
mkdir -p "$INST/bin" "$TMP/srv"

# ---------- fixture helpers ----------
# sign_manifest <file>: sign the canonical manifest body with the root secret
# key and write the base64 `sig` back into the manifest. The canonical body
# mirrors 04-release-github-ci/release/update_runtime_from_github.sh / gr-ota::manifest_sign_message.
sign_manifest() {
  local mf="$1"
  python3 - "$mf" "$SK" <<'PY'
import json, sys, base64
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
mf, sk_path = sys.argv[1], sys.argv[2]
seed = open(sk_path, "rb").read()
assert len(seed) == 32, "OTA root secret key must be 32 bytes"
sk = Ed25519PrivateKey.from_private_bytes(seed)
m = json.load(open(mf))
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
msg = json.dumps(body, sort_keys=True, separators=(",", ":")).encode()
m["sig"] = base64.b64encode(sk.sign(msg)).decode()
json.dump(m, open(mf, "w"))
PY
}

# write_unsigned_manifest <dir> <asset> <sha> : emit a manifest-${TRIPLE}.json
write_unsigned_manifest() {
  local dir="$1" asset="$2" sha="$3"
  cat > "$dir/manifest-${TRIPLE}.json" <<EOF
{"product":"green-v7","channel":"stable","build_id":"","release_pubkey":"","runtime":{"version":"7.0.0","abi":"7","asset":"$asset","sha256":"$sha"},"fe":null,"modules":[]}
EOF
}

# A "good" runtime asset: real ELF for the host arch (so ELF + arch checks pass).
GOOD_BIN="$TMP/good-runtime"
cp /bin/true "$GOOD_BIN"
GOOD_SHA="$(sha256sum "$GOOD_BIN" | awk '{print $1}')"
GOOD_ASSET="gr-service-7.0.0-${TRIPLE}"

# ---------- positive baseline release ----------
GOOD_DIR="$TMP/srv/v7.0.0-good"
mkdir -p "$GOOD_DIR"
cp "$GOOD_BIN" "$GOOD_DIR/$GOOD_ASSET"
cp "$PK"     "$GOOD_DIR/ota_ed25519.pk"
write_unsigned_manifest "$GOOD_DIR" "$GOOD_ASSET" "$GOOD_SHA"
sign_manifest "$GOOD_DIR/manifest-${TRIPLE}.json"

python3 -m http.server "$PORT" --bind 127.0.0.1 --directory "$TMP/srv" >/dev/null 2>&1 &
SERVER_PID=$!
for _ in $(seq 1 40); do
  curl -fsS "http://127.0.0.1:${PORT}/v7.0.0-good/manifest-${TRIPLE}.json" >/dev/null 2>&1 && break
  sleep 0.2
done

# run_upd <version> <base-url> : run the canonical updater; log to /tmp.
run_upd() {
  env VERSION="$1" INSTALL_ROOT="$INST" GR_RELEASE_BASE="$2" REQUIRE_SHA=1 \
    bash "$UPDATER" >/tmp/v7-ota-neg.log 2>&1
}

cur_bin() { [[ -x "$INST/bin/gr-service" ]] && sha256sum "$INST/bin/gr-service" | awk '{print $1}' || echo none; }

# expect_reject <label> <version> <base-url> <prev-sha> <prev-version>
expect_reject() {
  local label="$1" ver="$2" url="$3" prev_sha="$4" prev_ver="$5"
  if run_upd "$ver" "$url"; then
    echo "$label: updater unexpectedly ACCEPTED a bad release" >&2
    cat /tmp/v7-ota-neg.log >&2
    exit 1
  fi
  test "$(cur_bin)" = "$prev_sha"
  test "$(cat "$INST/VERSION")" = "$prev_ver"
  echo "$label"
}

# ---------- 0) positive baseline: good signed release installs ----------
run_upd "7.0.0" "http://127.0.0.1:${PORT}/v7.0.0-good" \
  || { echo "baseline good install failed"; cat /tmp/v7-ota-neg.log; exit 1; }
test "$(cur_bin)" = "$GOOD_SHA"
test "$(cat "$INST/VERSION")" = "7.0.0"
echo "V7_OTA_NEG_BASELINE_GOOD_OK"

# ---------- 1) V7 major enforcement (6.x and 8.x rejected pre-fetch) ----------
for badver in 6.5.0 8.0.0; do
  if VERSION="$badver" INSTALL_ROOT="$INST" bash "$UPDATER" >/tmp/v7-ota-neg.log 2>&1; then
    echo "V7 major: $badver unexpectedly accepted" >&2
    cat /tmp/v7-ota-neg.log >&2
    exit 1
  fi
done
test "$(cur_bin)" = "$GOOD_SHA"
test "$(cat "$INST/VERSION")" = "7.0.0"
echo "V7_OTA_NEG_MAJOR_ENFORCE_OK"

# Fixture bypass is test-only and must never disable production verification.
if GR_DEPLOY_ENV=production GR_ALLOW_UNSIGNED_FIXTURE=1 VERSION=7.0.0 \
  INSTALL_ROOT="$INST" GR_RELEASE_BASE="http://127.0.0.1:${PORT}/v7.0.0-good" \
  bash "$UPDATER" >/tmp/v7-ota-neg.log 2>&1; then
  echo "production unsigned fixture mode unexpectedly accepted" >&2
  exit 1
fi
grep -q "unsigned fixture mode is forbidden in production" /tmp/v7-ota-neg.log
test "$(cur_bin)" = "$GOOD_SHA"
echo "V7_OTA_NEG_PROD_BYPASS_REJECT_OK"

# ---------- 2) tampered manifest (body altered after signing) ----------
D="$TMP/srv/v7.0.1-tampered"
mkdir -p "$D"
cp "$GOOD_BIN" "$D/$GOOD_ASSET"
cp "$PK"     "$D/ota_ed25519.pk"
write_unsigned_manifest "$D" "$GOOD_ASSET" "$GOOD_SHA"
sign_manifest "$D/manifest-${TRIPLE}.json"
# mutate runtime.sha256 AFTER signing → signature no longer matches body
python3 - "$D/manifest-${TRIPLE}.json" <<'PY'
import json, sys
mf = sys.argv[1]
m = json.load(open(mf))
m["runtime"]["sha256"] = "deadbeef" * 8
json.dump(m, open(mf, "w"))
PY
expect_reject "V7_OTA_NEG_TAMPERED_MANIFEST_OK" \
  "7.0.1" "http://127.0.0.1:${PORT}/v7.0.1-tampered" "$GOOD_SHA" "7.0.0"

# ---------- 3) missing signature ----------
D="$TMP/srv/v7.0.2-nosig"
mkdir -p "$D"
cp "$GOOD_BIN" "$D/$GOOD_ASSET"
cp "$PK"     "$D/ota_ed25519.pk"
write_unsigned_manifest "$D" "$GOOD_ASSET" "$GOOD_SHA"
# deliberately NOT signed → no `sig` field
expect_reject "V7_OTA_NEG_MISSING_SIG_OK" \
  "7.0.2" "http://127.0.0.1:${PORT}/v7.0.2-nosig" "$GOOD_SHA" "7.0.0"

# ---------- 4) invalid signature (valid base64, wrong bytes) ----------
D="$TMP/srv/v7.0.3-badsig"
mkdir -p "$D"
cp "$GOOD_BIN" "$D/$GOOD_ASSET"
cp "$PK"     "$D/ota_ed25519.pk"
write_unsigned_manifest "$D" "$GOOD_ASSET" "$GOOD_SHA"
sign_manifest "$D/manifest-${TRIPLE}.json"
python3 - "$D/manifest-${TRIPLE}.json" <<'PY'
import base64, json, sys
mf = sys.argv[1]
m = json.load(open(mf))
# 64 zero bytes is a well-formed ed25519-length signature that cannot verify
m["sig"] = base64.b64encode(b"\x00" * 64).decode()
json.dump(m, open(mf, "w"))
PY
expect_reject "V7_OTA_NEG_INVALID_SIG_OK" \
  "7.0.3" "http://127.0.0.1:${PORT}/v7.0.3-badsig" "$GOOD_SHA" "7.0.0"

# ---------- 5) wrong root key / key substitution ----------
D="$TMP/srv/v7.0.4-wrongkey"
mkdir -p "$D"
cp "$GOOD_BIN" "$D/$GOOD_ASSET"
# Generate a fresh attacker keypair; serve ITS public key + sign manifest with
# ITS secret key. The pinned fingerprint check rejects before signature.
MANIFEST="manifest-${TRIPLE}.json" GOOD_ASSET="$GOOD_ASSET" GOOD_SHA="$GOOD_SHA" \
python3 - "$D" <<'PY'
import base64, json, os, sys
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat
d = sys.argv[1]
sk = Ed25519PrivateKey.generate()
pk = sk.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw)
open(os.path.join(d, "ota_ed25519.pk"), "wb").write(pk)
m = {
  "product": "green-v7", "channel": "stable", "build_id": "",
  "release_pubkey": "", "fe": None, "modules": [],
  "runtime": {"version": "7.0.0", "abi": "7",
              "asset": os.environ["GOOD_ASSET"], "sha256": os.environ["GOOD_SHA"]},
}
body = json.dumps({
  "product": m["product"], "channel": m["channel"], "build_id": "",
  "release_pubkey": "",
  "runtime": {k: m["runtime"][k] for k in ("version", "abi", "asset", "sha256")},
  "fe": None, "modules": [],
}, sort_keys=True, separators=(",", ":")).encode()
m["sig"] = base64.b64encode(sk.sign(body)).decode()
json.dump(m, open(os.path.join(d, os.environ["MANIFEST"]), "w"))
PY
expect_reject "V7_OTA_NEG_WRONG_ROOT_KEY_OK" \
  "7.0.4" "http://127.0.0.1:${PORT}/v7.0.4-wrongkey" "$GOOD_SHA" "7.0.0"

# ---------- 6) hash asset tampering (signed manifest, tampered asset bytes) ----------
D="$TMP/srv/v7.0.5-hash"
mkdir -p "$D"
cp /bin/false "$D/$GOOD_ASSET"   # different bytes → sha != manifest sha
cp "$PK"       "$D/ota_ed25519.pk"
write_unsigned_manifest "$D" "$GOOD_ASSET" "$GOOD_SHA"   # sha still GOOD_SHA
sign_manifest "$D/manifest-${TRIPLE}.json"
expect_reject "V7_OTA_NEG_HASH_TAMPER_OK" \
  "7.0.5" "http://127.0.0.1:${PORT}/v7.0.5-hash" "$GOOD_SHA" "7.0.0"

# ---------- 7) architecture mismatch (asset named for other arch) ----------
D="$TMP/srv/v7.0.6-arch"
mkdir -p "$D"
BAD_ASSET="gr-service-7.0.0-${OTHER_TRIPLE}"
cp "$GOOD_BIN" "$D/$BAD_ASSET"
cp "$PK"       "$D/ota_ed25519.pk"
write_unsigned_manifest "$D" "$BAD_ASSET" "$GOOD_SHA"
sign_manifest "$D/manifest-${TRIPLE}.json"
expect_reject "V7_OTA_NEG_ARCH_MISMATCH_OK" \
  "7.0.6" "http://127.0.0.1:${PORT}/v7.0.6-arch" "$GOOD_SHA" "7.0.0"

echo "V7_OTA_NEG_ALL_PASS"
