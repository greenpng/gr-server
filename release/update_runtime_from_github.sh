#!/usr/bin/env bash
# Pull Green V7 **runtime binary** from GitHub releases and restart service.
# Module .so updates should use admin panel OTA (hot); this path is for rare
# runtime / in-tree plane changes that require process restart.
#
# Usage on the server (or via SSH):
#   VERSION=8.0.0 bash 04-release-github-ci/release/update_runtime_from_github.sh
# Env:
#   VERSION          default: from /opt/green-v7/VERSION or required
#   INSTALL_ROOT     default: /opt/green-v7
#   RELEASE_REPO     default: greenpng/install
#   ARCH_TRIPLE      default: auto from uname -m → {arch}-linux-gnu
#   REQUIRE_SHA      default: 1 — fail if manifest lacks runtime.sha256
#   HEALTH_URL       default: http://127.0.0.1:28680/v1/health
set -euo pipefail

INSTALL_ROOT="${INSTALL_ROOT:-/opt/green-v7}"
RELEASE_REPO="${RELEASE_REPO:-greenpng/install}"
REQUIRE_SHA="${REQUIRE_SHA:-1}"
OTA_ROOT_PUBKEY_SHA256="${GR_OTA_ROOT_PUBKEY_SHA256:-${GV6_OTA_ROOT_PUBKEY_SHA256:-6dffd7d3c40b75a21db766cbee97ddebef21bcf089e30ba6dea0db5d8a1754a2}}"
HEALTH_URL="${HEALTH_URL:-http://127.0.0.1:28680/v1/health}"
VERSION="${VERSION:-}"
host_arch="$(uname -m)"
case "$host_arch" in
  x86_64|amd64) host_arch=x86_64 ;;
  aarch64|arm64) host_arch=aarch64 ;;
esac
ARCH_TRIPLE="${ARCH_TRIPLE:-${host_arch}-linux-gnu}"
if [[ -z "$VERSION" && -f "$INSTALL_ROOT/VERSION" ]]; then
  # if caller wants "latest" they must set VERSION explicitly
  VERSION="$(tr -d '[:space:]' < "$INSTALL_ROOT/VERSION")"
fi
if [[ -z "$VERSION" ]]; then
  echo "USAGE: VERSION=x.y.z $0" >&2
  exit 2
fi
[[ "$VERSION" =~ ^(7|8)\.[0-9]+\.[0-9]+$ ]] || { echo "Green V7/GR updater requires a 7.x.y or 8.x.y version" >&2; exit 2; }

BASE="${GR_RELEASE_BASE:-${GV6_RELEASE_BASE:-https://github.com/${RELEASE_REPO}/releases/download/v${VERSION}}}"
ASSET_SVC="gr-service-${VERSION}-${ARCH_TRIPLE}"
ASSET_CLI="gr-cli-${VERSION}-${ARCH_TRIPLE}"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# Prefer multi-arch index → per-arch manifest; fall back to manifest.json
resolve_manifest() {
  local man_name=""
  if curl -fsSL -o "$TMP/manifest-index.json" "$BASE/manifest-index.json" 2>/dev/null; then
    man_name=$(python3 - <<PY
import json
idx=json.load(open("$TMP/manifest-index.json"))
arch="$host_arch"
entry=(idx.get("architectures") or {}).get(arch) or {}
print(entry.get("manifest") or "")
PY
)
    if [[ -n "$man_name" && "$man_name" != *"/"* && "$man_name" != *".."* ]]; then
      if curl -fsSL -o "$TMP/manifest.json" "$BASE/$man_name"; then
        echo "[runtime-ota] manifest from index: $man_name (arch=$host_arch)"
        return 0
      fi
    fi
  fi
  if curl -fsSL -o "$TMP/manifest.json" "$BASE/manifest-${ARCH_TRIPLE}.json" 2>/dev/null; then
    echo "[runtime-ota] using manifest-${ARCH_TRIPLE}.json"
    return 0
  fi
  if curl -fsSL -o "$TMP/manifest.json" "$BASE/manifest.json" 2>/dev/null; then
    echo "[runtime-ota] using legacy manifest.json"
    return 0
  fi
  return 1
}

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

echo "[runtime-ota] arch=$ARCH_TRIPLE base=$BASE"
resolve_manifest || true
if [[ ! -f "$TMP/manifest.json" ]]; then
  echo "signed manifest missing" >&2
  exit 1
fi
# Independent root verification. Local fixture tests may explicitly opt out;
# production/update paths must never do so.
if [[ "${GR_ALLOW_UNSIGNED_FIXTURE:-${GV6_ALLOW_UNSIGNED_FIXTURE:-0}}" == "1" ]]; then
  case "${GR_DEPLOY_ENV:-${GV6_DEPLOY_ENV:-lab}}" in
    prod|production|live) echo "unsigned fixture mode is forbidden in production" >&2; exit 1 ;;
  esac
  # An ambiguous deploy env must not bypass signed mode: if the operator
  # explicitly demands signed manifests, unsigned fixtures are forbidden too.
  if [[ "${GR_REQUIRE_MANIFEST_SIG:-${GV6_REQUIRE_MANIFEST_SIG:-0}}" == "1" ]]; then
    echo "unsigned fixture mode is forbidden when GR_REQUIRE_MANIFEST_SIG=1" >&2
    exit 1
  fi
  echo "[runtime-ota] WARNING: unsigned fixture mode enabled" >&2
else
curl -fsSL -o "$TMP/ota_ed25519.pk" "$BASE/ota_ed25519.pk"
[[ "$(sha256sum "$TMP/ota_ed25519.pk" | awk '{print $1}')" == "$OTA_ROOT_PUBKEY_SHA256" ]] \
  || { echo "OTA root public key fingerprint mismatch" >&2; exit 1; }
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
  || { echo "invalid OTA root public key" >&2; exit 1; }
openssl pkeyutl -verify -pubin -inkey "$TMP/ota-root.pem" -rawin \
  -in "$TMP/manifest-body" -sigfile "$TMP/manifest.sig" >/dev/null 2>&1 \
  || { echo "manifest root signature verification failed" >&2; exit 1; }
fi
if [[ -f "$TMP/manifest.json" ]]; then
  MAN_ASSET=$(python3 -c 'import json,sys; m=json.load(open(sys.argv[1])); print((m.get("runtime") or {}).get("asset") or "")' "$TMP/manifest.json" 2>/dev/null || true)
  if [[ -n "$MAN_ASSET" ]]; then
    ASSET_SVC="$MAN_ASSET"
  fi
fi
[[ "$ASSET_SVC" == *"-${ARCH_TRIPLE}" ]] || {
  echo "runtime asset has wrong architecture: $ASSET_SVC" >&2
  exit 1
}

echo "[runtime-ota] fetch $BASE/$ASSET_SVC"
curl -fsSL -o "$TMP/gr-service" "$BASE/$ASSET_SVC"
chmod +x "$TMP/gr-service"
# optional CLI (P1-4): in signed mode the `gr-cli` helper must match the
# manifest's `cli` entry — it runs verify/stage/activate, so a tampered copy
# would bypass every downstream integrity check.
CLI_SHA=""
CLI_ASSET_MAN=""
if [[ "${GR_ALLOW_UNSIGNED_FIXTURE:-${GV6_ALLOW_UNSIGNED_FIXTURE:-0}}" != "1" && -f "$TMP/manifest.json" ]]; then
  CLI_SHA="$(python3 -c 'import json,sys
try:
    m=json.load(open(sys.argv[1])); c=m.get("cli") or {}
    sys.stdout.write(c.get("sha256") or "")
except Exception:
    pass' "$TMP/manifest.json" 2>/dev/null || true)"
  CLI_ASSET_MAN="$(python3 -c 'import json,sys
try:
    m=json.load(open(sys.argv[1])); c=m.get("cli") or {}
    sys.stdout.write(c.get("asset") or "")
except Exception:
    pass' "$TMP/manifest.json" 2>/dev/null || true)"
fi
if [[ -n "$CLI_SHA" || -n "$CLI_ASSET_MAN" ]]; then
  [[ -n "$CLI_ASSET_MAN" ]] && ASSET_CLI="$CLI_ASSET_MAN"
  [[ "$ASSET_CLI" != */* && "$ASSET_CLI" != *..* && "$ASSET_CLI" == *"-${ARCH_TRIPLE}" ]] \
    || { echo "cli asset invalid or wrong architecture: $ASSET_CLI" >&2; exit 1; }
  curl -fsSL -o "$TMP/gr-cli" "$BASE/$ASSET_CLI" 2>/dev/null \
    || { echo "[runtime-ota] manifest requires CLI asset but download failed ($ASSET_CLI)" >&2; exit 1; }
  chmod +x "$TMP/gr-cli"
  [[ -n "$CLI_SHA" ]] || { echo "cli sha256 missing" >&2; exit 1; }
  GOT_CLI="$(sha256sum "$TMP/gr-cli" | awk '{print $1}')"
  [[ "$GOT_CLI" == "$CLI_SHA" ]] || { echo "cli sha256 mismatch" >&2; exit 1; }
  echo "[runtime-ota] cli sha256 verified ($ASSET_CLI)"
elif [[ "${GR_ALLOW_UNSIGNED_FIXTURE:-${GV6_ALLOW_UNSIGNED_FIXTURE:-0}}" != "1" && "${GR_REQUIRE_MANIFEST_SIG:-${GV6_REQUIRE_MANIFEST_SIG:-0}}" == "1" ]]; then
  echo "[runtime-ota] signed release manifest lacks cli.sha256 (CLI integrity not covered)" >&2
  exit 1
else
  # fixture / legacy release: CLI remains an optional helper. 8.x uses gr-cli-*;
  # 7.x rollback assets may still be named gv6-*.
  for cand in "$ASSET_CLI" "gv6-${VERSION}-${ARCH_TRIPLE}"; do
    if curl -fsSL -o "$TMP/gr-cli" "$BASE/$cand" 2>/dev/null; then
      chmod +x "$TMP/gr-cli"
      ASSET_CLI="$cand"
      break
    fi
  done
fi
# optional FE tarball (product_version SSOT)
FE_TGZ="fe-${VERSION}.tgz"
if curl -fsSL -o "$TMP/fe.tgz" "$BASE/$FE_TGZ" 2>/dev/null; then
  echo "[runtime-ota] fetched $FE_TGZ"
else
  echo "[runtime-ota] no $FE_TGZ on release (FE not updated this run)"
fi

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

# R-01: verify runtime/FE sha256 from manifest when present; REQUIRE_SHA=1 fails if missing.
if [[ -f "$TMP/manifest.json" ]]; then
  EXPECT_SHA=$(python3 -c 'import json,sys; m=json.load(open(sys.argv[1])); print((m.get("runtime") or {}).get("sha256") or "")' "$TMP/manifest.json" 2>/dev/null || true)
  if [[ -n "${EXPECT_SHA}" ]]; then
    GOT_SHA=$(sha256sum "$TMP/gr-service" | awk '{print $1}')
    if [[ "$GOT_SHA" != "$EXPECT_SHA" ]]; then
      echo "runtime sha256 mismatch: got $GOT_SHA expect $EXPECT_SHA" >&2
      exit 1
    fi
    echo "[runtime-ota] runtime sha256 verified"
  elif [[ "$REQUIRE_SHA" == "1" ]]; then
    echo "runtime sha256 missing from manifest (set REQUIRE_SHA=0 only for lab)" >&2
    exit 1
  fi
  if [[ -f "$TMP/fe.tgz" ]]; then
    FE_SHA=$(python3 -c 'import json,sys; m=json.load(open(sys.argv[1])); print((m.get("fe") or {}).get("sha256") or "")' "$TMP/manifest.json" 2>/dev/null || true)
    if [[ -n "${FE_SHA}" ]]; then
      GOT_FE=$(sha256sum "$TMP/fe.tgz" | awk '{print $1}')
      if [[ "$GOT_FE" != "$FE_SHA" ]]; then
        echo "FE sha256 mismatch: got $GOT_FE expect $FE_SHA" >&2
        exit 1
      fi
      echo "[runtime-ota] FE sha256 verified"
    elif [[ "$REQUIRE_SHA" == "1" ]]; then
      echo "FE sha256 missing from manifest (set REQUIRE_SHA=0 only for lab)" >&2
      exit 1
    fi
  fi
elif [[ "$REQUIRE_SHA" == "1" ]]; then
  echo "manifest.json missing; cannot verify sha256 (set REQUIRE_SHA=0 only for lab)" >&2
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
if [[ -f "$TMP/manifest.json" ]]; then
  cp -f "$TMP/manifest.json" "$INSTALL_ROOT/dist/release-${VERSION}/manifest.json"
fi
echo "$VERSION" > "$INSTALL_ROOT/VERSION"
# Sync FE product_version so cool tickets + asset ?v= use V7 SSOT
if [[ -f "$TMP/fe.tgz" ]]; then
  echo "[runtime-ota] install FE assets (safe extract)"
  STAGE="$TMP/fe-extract"
  safe_extract_tar_gz "$TMP/fe.tgz" "$STAGE"
  if [[ -d "$INSTALL_ROOT/fe" ]]; then
    FE_BAK="$INSTALL_ROOT/fe.bak.$(date -u +%Y%m%dT%H%M%SZ)"
    mv "$INSTALL_ROOT/fe" "$FE_BAK" || true
  fi
  if [[ -d "$STAGE/fe" ]]; then
    mkdir -p "$INSTALL_ROOT"
    mv "$STAGE/fe" "$INSTALL_ROOT/fe"
  else
    mkdir -p "$INSTALL_ROOT/fe"
    # copy contents if tarball is flat under STAGE
    cp -a "$STAGE"/. "$INSTALL_ROOT/fe/"
  fi
  echo "$VERSION" > "$INSTALL_ROOT/fe/VERSION"
fi
if [[ -d "$INSTALL_ROOT/fe" ]]; then
  echo "$VERSION" > "$INSTALL_ROOT/fe/VERSION"
fi

# Point OTA module channel at this tag (panel still installs so separately)
ENVF="$INSTALL_ROOT/.env"
if [[ -f "$ENVF" ]]; then
  # 8.0: write GR_RELEASE_URL (primary) + GV6_RELEASE_URL (legacy alias)
  python3 - "$ENVF" "$BASE" <<'PYENV'
import sys
p, base = sys.argv[1], sys.argv[2]
lines = [l for l in open(p).read().splitlines(keepends=True)
         if not l.startswith(("GR_RELEASE_URL=", "GV6_RELEASE_URL="))]
lines.append(f"GR_RELEASE_URL={base}\n")
lines.append(f"GV6_RELEASE_URL={base}\n")
open(p, "w").writelines(lines)
PYENV
fi

echo "[runtime-ota] restart green-v7 + health gate"
UNIT="green-v7"
if ! systemctl is-enabled "$UNIT" >/dev/null 2>&1 && ! systemctl cat "$UNIT" >/dev/null 2>&1; then
  UNIT="green-v6"
fi
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
      systemctl restart "$UNIT" || true
      sleep 3
      if curl -fsS "$HEALTH_URL" >/tmp/gr-runtime-ota-health-rollback.json 2>/dev/null; then
        echo "[runtime-ota] ROLLBACK_HEALTH_OK" >&2
      else
        echo "[runtime-ota] ROLLBACK_HEALTH_FAIL" >&2
      fi
    fi
    exit 1
  fi
  systemctl is-active "$UNIT"
  echo "[runtime-ota] HEALTH_OK"
else
  echo "WARN: green-v7/green-v6 unit not found; binary installed, restart manually" >&2
fi

echo "[runtime-ota] done VERSION=$VERSION"
