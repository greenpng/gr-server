#!/usr/bin/env bash
# Build signed release artifacts for x86_64 + aarch64 on ONE dev machine.
#
# Recommended local workflow (no GitHub Actions minutes):
#   1. Install: Docker + `cargo install cross --locked`
#   2. bash 04-release-github-ci/release/build_multiarch.sh SKIP_PUBLISH=1
#   3. gh release upload …   # hosting only; build stays on your box
#
# Env:
#   HOST_ONLY=1       — current arch only → build_and_publish.sh
#   TARGETS="x86_64 aarch64"
#   USE_CROSS=0       — skip foreign arch (default: auto ON if docker+cross exist)
#   SKIP_PUBLISH=1    — do not gh upload
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
VERSION="$(tr -d '[:space:]' < VERSION)"
# P0-1/P0-4: one build_id + obf salt for the whole release (all arches share it).
GR_BUILD_ID="${GR_BUILD_ID:-${GV6_BUILD_ID:-$(openssl rand -hex 12)}}"
export GR_BUILD_ID
export GV6_BUILD_ID="$GR_BUILD_ID"   # legacy compile-time symbol name
GR_OBF_SALT="${GR_OBF_SALT:-${GV6_OBF_SALT:-$(openssl rand -hex 16)}}"
export GR_OBF_SALT
export GV6_OBF_SALT="$GR_OBF_SALT"   # legacy compile-time symbol name
echo "[multiarch] build_id=$GR_BUILD_ID"
TARGETS="${TARGETS:-x86_64 aarch64}"
HOST_ARCH="$(uname -m)"
case "$HOST_ARCH" in
  x86_64|amd64) HOST_ARCH=x86_64 ;;
  aarch64|arm64) HOST_ARCH=aarch64 ;;
esac

if [[ "${HOST_ONLY:-0}" == "1" ]]; then
  exec bash "$(cd "$(dirname "$0")" && pwd)/build_and_publish.sh"
fi

need_cross=0
for a in $TARGETS; do
  [[ "$a" != "$HOST_ARCH" ]] && need_cross=1
done

if [[ "$need_cross" == 1 && "${USE_CROSS:-auto}" == "auto" ]]; then
  if command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1; then
    USE_CROSS=1
  else
    USE_CROSS=0
  fi
fi

if [[ "$need_cross" == 1 && "${USE_CROSS:-0}" != "1" ]]; then
  echo "[multiarch] foreign arch requested but USE_CROSS!=1 and docker/cross unavailable"
  echo "[multiarch] install: cargo install cross --locked  (needs Docker running)"
  exit 1
fi

if [[ "$need_cross" == 1 ]] && ! command -v cross >/dev/null 2>&1; then
  echo "[multiarch] installing cross CLI…"
  cargo install cross --locked
fi

# Asset / manifest naming — matches build_and_publish.sh + panel OTA (`{arch}-linux-gnu`).
triple_for() {
  case "$1" in
    x86_64) echo "x86_64-linux-gnu" ;;
    aarch64) echo "aarch64-linux-gnu" ;;
    *) echo "unknown"; return 1 ;;
  esac
}

# rustc / cross target triple
rust_target_for() {
  case "$1" in
    x86_64) echo "x86_64-unknown-linux-gnu" ;;
    aarch64) echo "aarch64-unknown-linux-gnu" ;;
    *) echo "unknown"; return 1 ;;
  esac
}

ensure_keys() {
  mkdir -p "$ROOT/keys"
  if [[ -f "$ROOT/keys/ota_ed25519.sk" ]]; then
    return 0
  fi
  if [[ -n "${GR_OTA_SIGNING_KEY:-${GV6_OTA_SIGNING_KEY:-}}" ]]; then
    printf '%s' "${GR_OTA_SIGNING_KEY:-${GV6_OTA_SIGNING_KEY:-}}" > "$ROOT/keys/ota_ed25519.sk"
    chmod 600 "$ROOT/keys/ota_ed25519.sk"
    return 0
  fi
  if [[ "${GR_ALLOW_KEYGEN:-${GV6_ALLOW_KEYGEN:-0}}" == "1" ]]; then
    cargo run -q -p gr-cli -- keygen --out-dir "$ROOT/keys"
    return 0
  fi
  echo "[multiarch] missing keys/ota_ed25519.sk — set GR_OTA_SIGNING_KEY/GV6_OTA_SIGNING_KEY or GR_ALLOW_KEYGEN=1" >&2
  exit 1
}

ensure_fe() {
  # 布局可移植(同 build_and_publish.sh): greenpng 02-probe-analysis/... 或扁平 gr-server
  local area="$ROOT"
  if [[ -d "$ROOT/02-probe-analysis/probe/fe" ]]; then
    area="$ROOT/02-probe-analysis"
  fi
  echo -n "$VERSION" > "$area/probe/fe/VERSION"
  echo -n "$VERSION" > "$ROOT/VERSION.probe"
  if [[ -x "$area/scripts/fe/rebuild_bundles.sh" ]]; then
    bash "$area/scripts/fe/rebuild_bundles.sh"
  fi
}

stage_artifacts() {
  local arch="$1"
  local triple="$2"
  local out="$3"
  local target_dir="$4"

  mkdir -p "$out"
  # 公钥随产物分发 (install.sh 从 release 资产读取公钥验签)
  if [[ -f "$ROOT/keys/ota_ed25519.pk" ]]; then
    cp -f "$ROOT/keys/ota_ed25519.pk" "$out/ota_ed25519.pk"
  elif [[ -f "$ROOT/keys/ota_ed25519.pub" ]]; then
    cp -f "$ROOT/keys/ota_ed25519.pub" "$out/ota_ed25519.pk"
  fi
  cp -f "$target_dir/gr-service" "$out/gr-service-${VERSION}-${triple}"
  cp -f "$target_dir/gr-cli" "$out/gr-cli-${VERSION}-${triple}" 2>/dev/null || true
  chmod +x "$out/gr-service-${VERSION}-${triple}"
  [[ -f "$out/gr-cli-${VERSION}-${triple}" ]] && chmod +x "$out/gr-cli-${VERSION}-${triple}"

  local mods_json="[]"
  for pair in identity:gr_module_identity brain:gr_module_brain analyze:gr_module_analyze ingest:gr_module_ingest edge:gr_module_edge probe_assets:gr_module_probe_assets; do
    local name crate so asset art
    name="${pair%%:*}"
    crate="${pair##*:}"
    so="$(find "$target_dir" -maxdepth 1 -name "lib${crate//-/_}.so" | head -1 || true)"
    if [[ -z "$so" || ! -f "$so" ]]; then
      so="$(find "$target_dir" -maxdepth 1 -name "libgr_module_${name}*.so" | head -1 || true)"
    fi
    if [[ -z "$so" || ! -f "$so" ]]; then
      so="$(find "$target_dir" -maxdepth 1 -name "libgr_${name}*.so" | head -1 || true)"
    fi
    if [[ -z "$so" || ! -f "$so" ]]; then
      echo "[multiarch] WARN missing module so: $name ($arch)"
      continue
    fi
    asset="libgr_${name}-${VERSION}-${triple}.so"
    cp -f "$so" "$out/$asset"
    art=$(cargo run -q -p gr-cli -- sign-module \
      --name "$name" --version "$VERSION" --so "$out/$asset" \
      --secret-key "$ROOT/keys/ota_ed25519.sk" --domain "$name" \
      --release-key "$RELEASE_SK")
    echo "$art" > "$out/${name}.artifact.json"
    mods_json=$(python3 - <<PY
import json
mods = json.loads('''$mods_json''')
mods.append(json.loads('''$art'''))
print(json.dumps(mods))
PY
)
  done

  # FE/admin-spa 与架构无关: 若其他 arch 目录已打包同版本, 复制字节 (sha 一致),
  # 否则本 arch 首次打包。gzip 含时间戳, 重复打包会产生不同 sha → 各 arch manifest 不一致。
  fe_master="$(find "$ROOT/dist" -maxdepth 2 -name "fe-${VERSION}.tgz" ! -path "$out/*" | head -1 || true)"
  if [[ -n "$fe_master" && "$fe_master" != "$out/fe-${VERSION}.tgz" ]]; then
    cp -f "$fe_master" "$out/fe-${VERSION}.tgz"
  elif [[ -n "${GR_FE_MASTER_TGZ:-${GV6_FE_MASTER_TGZ:-}}" && -f "${GR_FE_MASTER_TGZ:-${GV6_FE_MASTER_TGZ:-}}" ]]; then
    cp -f "${GR_FE_MASTER_TGZ:-${GV6_FE_MASTER_TGZ:-}}" "$out/fe-${VERSION}.tgz"
  elif [[ ! -f "$out/fe-${VERSION}.tgz" ]]; then
    tar -C "$ROOT/02-probe-analysis" -czhf "$out/fe-${VERSION}.tgz" probe/fe
  fi
  admin_master="$(find "$ROOT/dist" -maxdepth 2 -name "admin-spa.tgz" ! -path "$out/*" | head -1 || true)"
  if [[ -n "$admin_master" && "$admin_master" != "$out/admin-spa.tgz" ]]; then
    cp -f "$admin_master" "$out/admin-spa.tgz"
  elif [[ -n "${GR_ADMIN_MASTER_TGZ:-${GV6_ADMIN_MASTER_TGZ:-}}" && -f "${GR_ADMIN_MASTER_TGZ:-${GV6_ADMIN_MASTER_TGZ:-}}" ]]; then
    cp -f "${GR_ADMIN_MASTER_TGZ:-${GV6_ADMIN_MASTER_TGZ:-}}" "$out/admin-spa.tgz"
  elif [[ -d "$ROOT/02-probe-analysis/panel/admin-spa" && ! -f "$out/admin-spa.tgz" ]]; then
    tar -C "$ROOT/02-probe-analysis" -czf "$out/admin-spa.tgz" panel/admin-spa
  fi

  python3 - <<PY
import hashlib, json, os, pathlib
out = pathlib.Path(r"""$out""")
version = """$VERSION"""
triple = """$triple"""
arch = """$arch"""
mods = json.loads(r'''$mods_json''')
svc = out / f"gr-service-{version}-{triple}"
fe = out / f"fe-{version}.tgz"
cli = out / f"gr-cli-{version}-{triple}"
man = {
    "product": "green-v7",
    "channel": "stable",
    "build_id": os.environ.get("GR_BUILD_ID") or os.environ.get("GV6_BUILD_ID", ""),
    "release_pubkey": os.environ.get("RELEASE_PK_HEX", ""),
    "release_cert": os.environ.get("RELEASE_CERT", ""),
    "arch": arch,
    "triple": triple,
    "runtime": {
        "version": version,
        "abi": 1,
        "asset": svc.name,
        "sha256": hashlib.sha256(svc.read_bytes()).hexdigest(),
    },
    # P1-4: signed CLI coverage — the installer verifies the gr-cli helper
    # against this entry before invoking verify/stage/activate.
    "cli": {
        "version": version,
        "abi": 1,
        "asset": cli.name,
        "sha256": hashlib.sha256(cli.read_bytes()).hexdigest(),
    } if cli.is_file() else None,
    "fe": {
        "asset": fe.name,
        "sha256": hashlib.sha256(fe.read_bytes()).hexdigest(),
    } if fe.is_file() else None,
    "modules": mods,
}
(out / f"manifest-{triple}.json").write_text(json.dumps(man, indent=2) + "\n")
print("[multiarch] manifest", out / f"manifest-{triple}.json", "modules", len(mods))
PY
}

build_native() {
  local arch="$1"
  local triple rust_target out target_dir
  triple="$(triple_for "$arch")"
  rust_target="$(rust_target_for "$arch")"
  out="$ROOT/dist/release-$VERSION-$arch"
  target_dir="$ROOT/target/release"

  echo "[multiarch] native build $arch ($rust_target)"
  cargo build --release -p gr-service -p gr-cli -p gr-harden
  for p in gr-module-identity gr-module-brain gr-module-analyze gr-module-ingest gr-module-edge gr-module-probe-assets; do
    cargo build --release -p "$p" --features plugin
  done
  stage_artifacts "$arch" "$triple" "$out" "$target_dir"
  file "$out/gr-service-${VERSION}-${triple}"
}

build_cross() {
  local arch="$1"
  local triple rust_target out cross_dir target_dir
  triple="$(triple_for "$arch")"
  rust_target="$(rust_target_for "$arch")"
  out="$ROOT/dist/release-$VERSION-$arch"
  cross_dir="/tmp/gr-cross-${VERSION}-${arch}"
  target_dir="$cross_dir/${rust_target}/release"

  echo "[multiarch] cross build $arch ($rust_target) target-dir=$cross_dir"
  rustup target add "$rust_target" >/dev/null 2>&1 || true
  rm -rf "$cross_dir"

  cross build --release --target "$rust_target" \
    --target-dir "$cross_dir" \
    -p gr-service -p gr-cli -p gr-harden
  for p in gr-module-identity gr-module-brain gr-module-analyze gr-module-ingest gr-module-edge gr-module-probe-assets; do
    cross build --release --target "$rust_target" \
      --target-dir "$cross_dir" \
      -p "$p" --features plugin
  done

  stage_artifacts "$arch" "$triple" "$out" "$target_dir"
  file "$out/gr-service-${VERSION}-${triple}"
}

ensure_keys

# P0-2: one ephemeral per-release signing key for the whole release; the root
# key certifies it. Secret stays local (never merged into $MERGE).
RELKEY_DIR="$ROOT/dist/keys-release-$VERSION"
mkdir -p "$RELKEY_DIR"
if [[ ! -f "$RELKEY_DIR/ota_ed25519.sk" ]]; then
  cargo run -q -p gr-cli -- keygen --out-dir "$RELKEY_DIR"
fi
RELEASE_PK_HEX="$(python3 -c "import pathlib,sys; sys.stdout.write(pathlib.Path(r'$RELKEY_DIR/ota_ed25519.pk').read_bytes().hex())")"
RELEASE_SK="$RELKEY_DIR/ota_ed25519.sk"
if [[ -z "$RELEASE_PK_HEX" || ! -f "$RELEASE_SK" ]]; then
  echo "[multiarch] FAILED per-release keypair" >&2
  exit 1
fi
RELEASE_CERT="$(cargo run -q -p gr-cli -- sign-cert \
  --secret-key "$ROOT/keys/ota_ed25519.sk" \
  --version "$VERSION" \
  --build-id "$GR_BUILD_ID" \
  --release-pubkey "$RELEASE_PK_HEX")"
if [[ -z "$RELEASE_CERT" || "$RELEASE_CERT" == *error* ]]; then
  echo "[multiarch] FAILED release cert: $RELEASE_CERT" >&2
  exit 1
fi
export RELEASE_PK_HEX RELEASE_CERT RELEASE_SK
echo "[multiarch] per-release key rotated"

ensure_fe

for a in $TARGETS; do
  if [[ "$a" == "$HOST_ARCH" ]]; then
    build_native "$a"
  else
    build_cross "$a"
  fi
done

MERGE="$ROOT/dist/release-$VERSION"
# Wipe any stale merge dir first: leftovers from a previous partial build
# (old binaries / build_id / manifest.json) silently survive cp and then
# mismatch the freshly signed manifest (v8.0.0 release incident, 2026-09-01).
rm -rf "$MERGE"
mkdir -p "$MERGE"
for a in $TARGETS; do
  src="$ROOT/dist/release-$VERSION-$a"
  [[ -d "$src" ]] || continue
  # -f: a stale merge dir (e.g. from a previous partial build) must never
  # keep old binaries — the manifest signs the FRESH build (iss 8.0.0 lesson).
  cp -af "$src"/* "$MERGE/" 2>/dev/null || cp -af "$src"/. "$MERGE/"
done
# Generic module sidecars are ambiguous after a multi-arch merge. The signed
# per-architecture manifest is the only consumer source of module assets.
find "$MERGE" -maxdepth 1 -type f -name '*.artifact.json' -delete

python3 - <<PY
import json, pathlib
merge = pathlib.Path(r"""$MERGE""")
version = """$VERSION"""
index = {
  "product": "green-v7",
  "version": version,
  "architectures": {},
  "build_host": "$(uname -m)",
  "note": "Built locally; GitHub Releases is asset hosting only.",
}
for p in sorted(merge.glob("manifest-*-linux-gnu.json")):
    if p.name == "manifest-index.json":
        continue
    man = json.loads(p.read_text())
    arch = man.get("arch") or "unknown"
    index["architectures"][arch] = {"manifest": p.name, "triple": man.get("triple")}
(merge / "manifest-index.json").write_text(json.dumps(index, indent=2) + "\n")
print(json.dumps(index, indent=2))
PY

for mf in "$MERGE"/manifest-*-linux-gnu.json; do
  [[ -f "$mf" ]] || continue
  cargo run -q -p gr-cli -- sign-manifest --manifest "$mf" --secret-key "$ROOT/keys/ota_ed25519.sk" \
    --build-id "$GR_BUILD_ID" \
    --release-pubkey "$RELEASE_PK_HEX" \
    --release-cert "$RELEASE_CERT"
done

# Back-compat: host arch also as manifest.json for single-arch OTA scripts
host_triple="$(triple_for "$HOST_ARCH")"
if [[ -f "$MERGE/manifest-${host_triple}.json" ]]; then
  cp -f "$MERGE/manifest-${host_triple}.json" "$MERGE/manifest.json"
fi

# Release key material as convenience assets, derived from the SIGNED
# manifest (single source of truth — never from build-time shell state,
# which may diverge from what was actually signed; v8.0.0 incident).
# The multiarch path (build_signed_modules.sh) does not write these, unlike
# the HOST_ONLY path (build_and_publish.sh) — derive them here so both
# paths ship the same asset set.
python3 - <<PY
import json, pathlib
merge = pathlib.Path(r"""$MERGE""")
man = json.loads((merge / "manifest.json").read_text())
(merge / "build_id").write_text((man.get("build_id") or "") + "\\n")
(merge / "release_pubkey.hex").write_text(man.get("release_pubkey") or "")
(merge / "release_cert.sig").write_text(man.get("release_cert") or "")
print("[multiarch] key material assets derived from signed manifest")
PY

echo "[multiarch] merged → $MERGE"
ls -la "$MERGE" | head -30

if [[ "${SKIP_PUBLISH:-0}" == "1" ]]; then
  echo "[multiarch] SKIP_PUBLISH=1 done"
  exit 0
fi

REPO="${GR_RELEASE_REPO:-${GV6_RELEASE_REPO:-greenpng/install}}"
if command -v gh >/dev/null 2>&1; then
  mapfile -t ASSETS < <(find "$MERGE" -maxdepth 1 -type f ! -name '*.sk' | sort)
  if gh release view "v${VERSION}" --repo "$REPO" >/dev/null 2>&1; then
    echo "[multiarch] release v${VERSION} exists — uploading new assets only (no clobber)"
    gh release upload "v${VERSION}" "${ASSETS[@]}" --repo "$REPO"
  else
    gh release create "v${VERSION}" "${ASSETS[@]}" --repo "$REPO" \
      --title "Green V7 ${VERSION}" \
      --notes "Local multi-arch build (x86_64 + aarch64). See manifest-index.json."
  fi
else
  echo "[multiarch] gh not installed — artifacts ready at $MERGE"
fi
