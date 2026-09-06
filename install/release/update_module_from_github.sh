#!/usr/bin/env bash
# Pull ONE Green V7 module (.so) from a signed GitHub release and (optionally)
# activate it on the hot path — the free-tier per-module updater.
#
# Paid deployments can use the admin panel OTA instead (POST /api/ota/install
# {name, version, activate}); this script is the equivalent for free nodes and
# for scripted/CI upgrade flows. It performs the same verification chain as the
# panel path: manifest root-chain verification → release-key binding →
# artifact double-signature (root sig + release sig2) → sha256 → runtime
# compat gate (abi / requires_major / min+max runtime minor) → atomic stage →
# optional activate. Rollback = `gr-cli activate --name <m> --version <old>`.
#
# Usage on the server (or via SSH):
#   MODULE=identity bash install/release/update_module_from_github.sh
#   MODULE=identity VERSION=8.0.0 bash install/release/update_module_from_github.sh
# Env:
#   MODULE           module name: identity brain analyze ingest edge probe_assets
#   VERSION          target module version; empty = highest in manifest
#   INSTALL_ROOT     default: /opt/green-v7
#   RELEASE_REPO     default: greenpng/gr-server (历史版本可用 ENV 覆盖回 greenpng/install)
#   ARCH_TRIPLE      default: auto from uname -m → {arch}-linux-gnu
#   GR_RELEASE_BASE / GV6_RELEASE_BASE override the full release URL prefix (for mirrors)
#   GR_PUBKEY_PATH / GV6_PUBKEY_PATH  default: $INSTALL_ROOT/ota_ed25519.pk
#   ACTIVATE         default: 1 — flip the active marker after staging
#   GR_CLI / GV6_CLI                  default: $INSTALL_ROOT/bin/gr-cli
set -euo pipefail

INSTALL_ROOT="${INSTALL_ROOT:-/opt/green-v7}"
RELEASE_REPO="${RELEASE_REPO:-greenpng/gr-server}"
MODULE="${MODULE:-${1:-}}"
VERSION="${VERSION:-}"
ACTIVATE="${ACTIVATE:-1}"
OTA_ROOT_PUBKEY_SHA256="${GR_OTA_ROOT_PUBKEY_SHA256:-${GV6_OTA_ROOT_PUBKEY_SHA256:-4a2296d33e66838d8a8cbd697a686bfb79c93a3d7da8a8f4cd60e949b297ea0d}}"
GR_CLI="${GR_CLI:-${GV6_CLI:-$INSTALL_ROOT/bin/gr-cli}}"

if [[ -z "$MODULE" ]]; then
  echo "USAGE: MODULE=<name> $0   (modules: identity brain analyze ingest edge probe_assets)" >&2
  exit 2
fi
# Default VERSION = the installed release tag: "refresh this module to what the
# current release carries". Incremental module publishes re-tag the same VERSION,
# so omitting VERSION tracks hotfixes without bumping anything else. Set
# VERSION explicitly to jump to another tag (up or down).
if [[ -z "$VERSION" && -f "$INSTALL_ROOT/VERSION" ]]; then
  VERSION="$(tr -d '[:space:]' < "$INSTALL_ROOT/VERSION")"
fi
host_arch="$(uname -m)"
case "$host_arch" in
  x86_64|amd64) host_arch=x86_64 ;;
  aarch64|arm64) host_arch=aarch64 ;;
esac
ARCH_TRIPLE="${ARCH_TRIPLE:-${host_arch}-linux-gnu}"

if [[ -z "$VERSION" ]]; then
  if [[ -n "${GR_RELEASE_BASE:-${GV6_RELEASE_BASE:-}}" ]]; then
    echo "[module-ota] MODULE=$MODULE target=highest-version-in-manifest (GR_RELEASE_BASE/GV6_RELEASE_BASE set, no VERSION)"
  else
    echo "[module-ota] MODULE=$MODULE target=release-tag-version" >&2
    echo "[module-ota] no $INSTALL_ROOT/VERSION and VERSION unset — set VERSION=x.y.z" >&2
    exit 2
  fi
else
  echo "[module-ota] MODULE=$MODULE target=VERSION=$VERSION"
fi

BASE="${GR_RELEASE_BASE:-${GV6_RELEASE_BASE:-https://github.com/${RELEASE_REPO}/releases/download/v${VERSION}}}"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# Prefer multi-arch index → per-arch manifest; fall back to per-arch/legacy names.
MANIFEST_NAME=""
if curl -fsSL -o "$TMP/manifest-index.json" "$BASE/manifest-index.json" 2>/dev/null; then
  MANIFEST_NAME="$(python3 - <<PY
import json
idx = json.load(open("$TMP/manifest-index.json"))
entry = (idx.get("architectures") or {}).get("$host_arch") or {}
print(entry.get("manifest") or "")
PY
)"
  if [[ -n "$MANIFEST_NAME" && ( "$MANIFEST_NAME" == */* || "$MANIFEST_NAME" == *..* ) ]]; then
    echo "[module-ota] manifest name rejected (path escape): $MANIFEST_NAME" >&2
    MANIFEST_NAME=""
  fi
fi
if [[ -n "$MANIFEST_NAME" ]]; then
  curl -fsSL -o "$TMP/manifest.json" "$BASE/$MANIFEST_NAME" 2>/dev/null \
    || { echo "[module-ota] cannot fetch $BASE/$MANIFEST_NAME" >&2; exit 1; }
  echo "[module-ota] manifest: $MANIFEST_NAME (arch=$host_arch)"
elif curl -fsSL -o "$TMP/manifest.json" "$BASE/manifest-${ARCH_TRIPLE}.json" 2>/dev/null; then
  echo "[module-ota] manifest: manifest-${ARCH_TRIPLE}.json"
elif curl -fsSL -o "$TMP/manifest.json" "$BASE/manifest.json" 2>/dev/null; then
  echo "[module-ota] manifest: manifest.json (legacy)"
else
  echo "[module-ota] no signed manifest at $BASE" >&2
  exit 1
fi

PK="${GR_PUBKEY_PATH:-${GV6_PUBKEY_PATH:-$INSTALL_ROOT/ota_ed25519.pk}}"
[[ -f "$PK" ]] || { echo "[module-ota] OTA root public key missing: $PK" >&2; exit 1; }
[[ "$(sha256sum "$PK" | awk '{print $1}')" == "$OTA_ROOT_PUBKEY_SHA256" ]] \
  || { echo "[module-ota] OTA root public key fingerprint mismatch ($PK)" >&2; exit 1; }

# Trust anchor check done above; gr-cli module update re-verifies the full chain.
# Show the candidate artifact so operators can eyeball before activation.
# (CLI's pick_module is authoritative for version ranking; this is display only.)
python3 - "$TMP/manifest.json" "$MODULE" "$VERSION" <<'PY'
import json, sys
try:
    import semver  # optional: semantic ranking for display
except Exception:
    semver = None
m = json.load(open(sys.argv[1]))
name, ver = sys.argv[2], sys.argv[3]
cands = [a for a in m.get("modules", []) if a.get("name") == name]
if not cands:
    print(f"[module-ota] module {name!r} not in manifest", file=sys.stderr)
    sys.exit(1)
if ver:
    cands = [a for a in cands if a.get("version") == ver]
    if not cands:
        print(f"[module-ota] manifest has no {name}@{ver}", file=sys.stderr)
        sys.exit(1)
elif semver is not None:
    cands.sort(key=lambda a: semver.Version.parse(a["version"]), reverse=True)
a = cands[0]
print(f"[module-ota] artifact {a['name']}@{a['version']} abi={a['abi']} "
      f"requires_major={a['requires_major']} min_runtime_minor={a['min_runtime_minor']} "
      f"max_runtime_minor={a.get('max_runtime_minor') or 'none'} sha256={a['sha256'][:16]}…")
PY

[[ -x "$GR_CLI" ]] || {
  echo "[module-ota] gr-cli not found/executable: $GR_CLI" >&2
  echo "[module-ota] install it via install.sh or update_runtime_from_github.sh first" >&2
  exit 1
}

ARGS=(module update --name "$MODULE" --base "$BASE" --pubkey "$PK"
      --modules-dir "$INSTALL_ROOT/modules")
# Compat gate must run against the REAL installed runtime, not the CLI's
# CLI default is only a fallback; a node on a later runtime whose module
# declares min_runtime_minor would otherwise be falsely rejected.
RUNTIME_VER="$(tr -d '[:space:]' < "$INSTALL_ROOT/VERSION" 2>/dev/null || true)"
if [[ -n "$RUNTIME_VER" ]]; then
  echo "[module-ota] runtime_version=$RUNTIME_VER (compat gate)"
  ARGS+=(--runtime-version "$RUNTIME_VER")
fi
if [[ -n "$VERSION" ]]; then
  ARGS+=(--version "$VERSION")
fi
if [[ "$ACTIVATE" == "1" ]]; then
  ARGS+=(--activate)
fi

if "$GR_CLI" module --help >/dev/null 2>&1; then
  echo "[module-ota] $GR_CLI ${ARGS[*]}"
  OUT="$("$GR_CLI" "${ARGS[@]}")"
  echo "$OUT"
else
  # 7.x rollback helpers may predate `gr-cli module update`; keep rollback
  # scriptable by doing the same signed stage/activate steps here.
  echo "[module-ota] legacy CLI without 'module update'; using direct stage/activate fallback"
  ART_JSON="$TMP/${MODULE}.artifact.json"
  SO_NAME="$(python3 - "$TMP/manifest.json" "$MODULE" "$VERSION" "$ART_JSON" "$ARCH_TRIPLE" <<'PY'
import json, sys
manifest, name, ver, out, triple = sys.argv[1:]
m = json.load(open(manifest))
cands = [a for a in m.get("modules", []) if a.get("name") == name]
if ver:
    cands = [a for a in cands if a.get("version") == ver]
if not cands:
    raise SystemExit(f"manifest has no {name}@{ver or '<highest>'}")
a = cands[0]
asset = a.get("asset") or ""
if "/" in asset or ".." in asset or not asset.endswith(f"-{triple}.so"):
    raise SystemExit(f"module asset invalid or wrong architecture: {asset}")
with open(out, "w") as f:
    json.dump(a, f)
print(asset)
PY
)"
  curl -fsSL -o "$TMP/$SO_NAME" "$BASE/$SO_NAME"
  "$GR_CLI" verify-manifest --manifest "$TMP/manifest.json" --pubkey "$PK" \
    --modules-dir "$INSTALL_ROOT/modules" >/dev/null
  STAGE_RUNTIME="${RUNTIME_VER:-${VERSION:-}}"
  [[ -n "$STAGE_RUNTIME" ]] || { echo "[module-ota] runtime version required for legacy stage fallback" >&2; exit 2; }
  echo "[module-ota] $GR_CLI stage --modules-dir $INSTALL_ROOT/modules --pubkey $PK --artifact-json $ART_JSON --so $TMP/$SO_NAME --runtime-version $STAGE_RUNTIME"
  "$GR_CLI" stage --modules-dir "$INSTALL_ROOT/modules" \
    --pubkey "$PK" \
    --artifact-json "$ART_JSON" \
    --so "$TMP/$SO_NAME" \
    --runtime-version "$STAGE_RUNTIME"
  if [[ "$ACTIVATE" == "1" ]]; then
    echo "[module-ota] $GR_CLI activate --modules-dir $INSTALL_ROOT/modules --name $MODULE --version ${VERSION:-$STAGE_RUNTIME}"
    "$GR_CLI" activate --modules-dir "$INSTALL_ROOT/modules" --name "$MODULE" --version "${VERSION:-$STAGE_RUNTIME}"
  fi
fi
echo "[module-ota] done — next runtime pick-up is hot (no restart). Roll back with:"
echo "  gr-cli activate --modules-dir $INSTALL_ROOT/modules --name $MODULE --version <OLD>"
