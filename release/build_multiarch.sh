#!/usr/bin/env bash
# Build signed WHOLE-BUNDLE releases for x86_64 + aarch64 on one dev machine.
#
# Output (dist/release-<VER>/, ≈5 release assets):
#   greenpng-<VER>-x86_64.tar.gz / greenpng-<VER>-aarch64.tar.gz
#   fe-<VER>.tgz                 (shared FE tarball, single source)
#   manifest-index.json          (arch → bundle + bundle_sha256, fe entry)
#   ota_ed25519.pk
# Each bundle contains bin/ modules/ fe/ admin/ spec/ manifest.json
# ota_ed25519.pk; the in-bundle manifest is root-signed and lists per-file
# sha256 (runtime/cli/fe_tree/admin_tree/spec_tree/modules with sig chain).
#
# Env:
#   HOST_ONLY=1       — current arch only → build_and_publish.sh
#   TARGETS="x86_64 aarch64"
#   USE_CROSS=0       — skip foreign arch (default: auto ON if docker+cross exist)
#   SKIP_PUBLISH=1    — do not gh upload
set -euo pipefail
# 仓库根解析: 从脚本位置向上找 Cargo.toml (编号布局 / 扁平发行布局两用)
ROOT="$(cd "$(dirname "$0")" && pwd)"
while [[ "$ROOT" != "/" && ! -f "$ROOT/Cargo.toml" ]]; do ROOT="$(dirname "$ROOT")"; done
[[ -f "$ROOT/Cargo.toml" ]] || { echo "[FATAL] cannot locate workspace root (Cargo.toml) from $0" >&2; exit 1; }
cd "$ROOT"
VERSION="$(tr -d '[:space:]' < VERSION)"
# P0-1/P0-4: one build_id + obf salt for the whole release (all arches share it).
GR_BUILD_ID="${GR_BUILD_ID:-$(openssl rand -hex 12)}"
export GR_BUILD_ID
GR_OBF_SALT="${GR_OBF_SALT:-$(openssl rand -hex 16)}"
export GR_OBF_SALT
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

# 布局可移植: greenpng 工作区(02-probe-analysis/...) 或扁平发行仓 gr-server
if [[ -d "$ROOT/02-probe-analysis/probe/fe" ]]; then
  AREA="$ROOT/02-probe-analysis"
elif [[ -d "$ROOT/probe/fe" ]]; then
  AREA="$ROOT"
else
  echo "[multiarch] cannot locate probe/fe" >&2; exit 1
fi
# P0 运行时数据 (data_tree) 布局可移植: 开发仓在仓根 data/ (02-probe-analysis
# 无 data/), 扁平发行仓同样在仓根 data/ — 两布局都是 $ROOT/data; 留
# $AREA/data 优先分支以兼容未来归位。缺件即死 (与 sync 在位断言同防线)。
if [[ -f "$AREA/data/r100_templates.json" ]]; then
  DATA_SRC="$AREA/data"
elif [[ -f "$ROOT/data/r100_templates.json" ]]; then
  DATA_SRC="$ROOT/data"
else
  echo "[multiarch] FAIL: cannot locate r100_templates.json (tried $AREA/data and $ROOT/data)" >&2
  exit 1
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

triple_for() {
  case "$1" in
    x86_64) echo "x86_64-linux-gnu" ;;
    aarch64) echo "aarch64-linux-gnu" ;;
    *) echo "unknown"; return 1 ;;
  esac
}

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
  if [[ -n "${GR_OTA_SIGNING_KEY:-}" ]]; then
    printf '%s' "${GR_OTA_SIGNING_KEY:-}" > "$ROOT/keys/ota_ed25519.sk"
    chmod 600 "$ROOT/keys/ota_ed25519.sk"
    return 0
  fi
  if [[ "${GR_ALLOW_KEYGEN:-0}" == "1" ]]; then
    cargo run -q -p gr-cli -- keygen --out-dir "$ROOT/keys"
    return 0
  fi
  echo "[multiarch] missing keys/ota_ed25519.sk — set GR_OTA_SIGNING_KEY or GR_ALLOW_KEYGEN=1" >&2
  exit 1
}

ensure_fe() {
  echo -n "$VERSION" > "$AREA/probe/fe/VERSION"
  echo -n "$VERSION" > "$ROOT/VERSION.probe"
  if [[ -x "$AREA/scripts/fe/rebuild_bundles.sh" ]]; then
    bash "$AREA/scripts/fe/rebuild_bundles.sh"
  fi
  # 共享 FE tarball 单源: 所有 arch manifest 哈希同一份 (确定性 tar)。
  mkdir -p "$ROOT/dist/fe-bundle"
  if [[ -n "${GR_FE_MASTER_TGZ:-}" && -f "${GR_FE_MASTER_TGZ:-}" ]]; then
    cp -f "${GR_FE_MASTER_TGZ:-}" "$ROOT/dist/fe-bundle/fe-${VERSION}.tgz"
  elif [[ ! -f "$ROOT/dist/fe-bundle/fe-${VERSION}.tgz" ]]; then
    tar -C "$AREA" --sort=name --mtime='@0' --owner=0 --group=0 --numeric-owner \
        -cf - probe/fe | gzip -n -9 > "$ROOT/dist/fe-bundle/fe-${VERSION}.tgz"
  fi
}

stage_bundle() {
  # stage_bundle <arch> <triple> <out> <target_dir>
  local arch="$1" triple="$2" out="$3" target_dir="$4"
  local bundle_name="greenpng-${VERSION}-${arch}"
  local bd="$out/bundle/$bundle_name"

  rm -rf "$out/bundle"
  mkdir -p "$bd/bin" "$bd/modules"

  cp -f "$target_dir/gr-service" "$bd/bin/gr-service"
  cp -f "$target_dir/gr-cli" "$bd/bin/gr-cli" 2>/dev/null || true
  chmod +x "$bd/bin/gr-service"
  [[ -f "$bd/bin/gr-cli" ]] && chmod +x "$bd/bin/gr-cli"

  local mods_json="[]"
  local pair name crate so asset art
  for pair in identity:gr_module_identity brain:gr_module_brain analyze:gr_module_analyze ingest:gr_module_ingest edge:gr_module_edge probe_assets:gr_module_probe_assets; do
    name="${pair%%:*}"; crate="${pair##*:}"
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
    asset="modules/libgr_${name}-${VERSION}-${triple}.so"
    cp -f "$so" "$bd/$asset"
    art=$(cargo run -q -p gr-cli -- sign-module \
      --name "$name" --version "$VERSION" --so "$bd/$asset" \
      --secret-key "$ROOT/keys/ota_ed25519.sk" --domain "$name" \
      --release-key "$RELEASE_SK")
    mods_json=$(python3 - <<PY
import json
mods = json.loads('''$mods_json''')
mods.append(json.loads('''$art'''))
print(json.dumps(mods))
PY
)
  done

  # FE 展开 + 共享 tgz (跨 arch 字节一致)
  cp -f "$ROOT/dist/fe-bundle/fe-${VERSION}.tgz" "$out/fe-${VERSION}.tgz"
  rm -rf "$bd/fe"
  cp -a "$AREA/probe/fe" "$bd/fe"
  rm -rf "$bd/admin"
  cp -a "$AREA/panel/admin-spa" "$bd/admin"
  # spec (analyze 运行时数据, 1.0.2+ 进签名清单; 与 build_and_publish.sh 同型)
  rm -rf "$bd/spec"
  cp -a "$AREA/spec" "$bd/spec"
  # data (P0 运行时数据: r100 反脚本模板 + geoip mmdb, 1.0.8+ data_tree;
  # 与 build_and_publish.sh 同型 — 只装产品文件, 安装侧 overlay 不删运行态;
  # 缺件即死, 与 sync 在位断言同防线)
  rm -rf "$bd/data"
  mkdir -p "$bd/data/geo"
  [[ -f "$DATA_SRC/r100_templates.json" ]] \
    || { echo "[release] FAIL: data/r100_templates.json missing (arch $arch)" >&2; exit 1; }
  cp -f "$DATA_SRC/r100_templates.json" "$bd/data/r100_templates.json"
  for m in dbip-asn-lite.mmdb dbip-country-lite.mmdb; do
    [[ -f "$DATA_SRC/geo/$m" ]] \
      || { echo "[release] FAIL: data/geo/$m missing (arch $arch)" >&2; exit 1; }
    cp -f "$DATA_SRC/geo/$m" "$bd/data/geo/$m"
  done
  if [[ -f "$ROOT/keys/ota_ed25519.pk" ]]; then
    cp -f "$ROOT/keys/ota_ed25519.pk" "$bd/ota_ed25519.pk"
  elif [[ -f "$ROOT/keys/ota_ed25519.pub" ]]; then
    cp -f "$ROOT/keys/ota_ed25519.pub" "$bd/ota_ed25519.pk"
  fi

  python3 - <<PY > "$bd/manifest.json"
import hashlib, json, pathlib
version = """$VERSION"""
triple = """$triple"""
arch = """$arch"""
bd = pathlib.Path(r"""$bd""")
out = pathlib.Path(r"""$out""")
mods = json.loads(r'''$mods_json''')
for m in mods:
    flat = m.get("asset", "")
    if "/" not in flat:
        m["asset"] = f"modules/{flat}"

def sha(p):
    return hashlib.sha256(p.read_bytes()).hexdigest()

def tree_files(root):
    root = pathlib.Path(root)
    files = {}
    for p in sorted(root.rglob("*")):
        if p.is_file() and not p.is_symlink():
            files[str(p.relative_to(root))] = sha(p)
    return files

svc = bd / "bin" / "gr-service"
cli = bd / "bin" / "gr-cli"
fe_tgz = out / f"fe-{version}.tgz"
man = {
    "product": "greenpng",
    "channel": "stable",
    "arch": arch,
    "triple": triple,
    "runtime": {"version": version, "abi": 1, "asset": "bin/gr-service", "sha256": sha(svc)},
    "cli": {"version": version, "abi": 1, "asset": "bin/gr-cli", "sha256": sha(cli)} if cli.is_file() else None,
    "fe": {"asset": fe_tgz.name, "sha256": sha(fe_tgz)},
    "fe_tree": {"epoch": version, "files": tree_files(bd / "fe")},
    "admin_tree": {"files": tree_files(bd / "admin")},
    "spec_tree": {"files": tree_files(bd / "spec")},
    "data_tree": {"files": tree_files(bd / "data")},
    "modules": mods,
}
man = {k: v for k, v in man.items() if v is not None}
print(json.dumps(man, indent=2))
PY

  cargo run -q -p gr-cli -- sign-manifest --manifest "$bd/manifest.json" \
    --secret-key "$ROOT/keys/ota_ed25519.sk" \
    --build-id "$GR_BUILD_ID" \
    --release-pubkey "$RELEASE_PK_HEX" \
    --release-cert "$RELEASE_CERT"

  tar -C "$out/bundle" --sort=name --mtime='@0' --owner=0 --group=0 --numeric-owner \
      -cf - "$bundle_name" | gzip -n -9 > "$out/$bundle_name.tar.gz"
  # 守卫/比对用副本 (合并时索引引用包内 manifest.json)
  cp -f "$bd/manifest.json" "$out/manifest.json"
  echo -n "$GR_BUILD_ID" > "$out/build_id"
  echo "[multiarch] bundle $out/$bundle_name.tar.gz"
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
  stage_bundle "$arch" "$triple" "$out" "$target_dir"
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

  stage_bundle "$arch" "$triple" "$out" "$target_dir"
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

# ---- merge: ≈5 release assets ----
MERGE="$ROOT/dist/release-$VERSION"
rm -rf "$MERGE"
mkdir -p "$MERGE"
for a in $TARGETS; do
  src="$ROOT/dist/release-$VERSION-$a"
  [[ -d "$src" ]] || continue
  cp -f "$src/greenpng-${VERSION}-${a}.tar.gz" "$MERGE/"
done
cp -f "$ROOT/dist/fe-bundle/fe-${VERSION}.tgz" "$MERGE/"
cp -f "$ROOT/keys/ota_ed25519.pk" "$MERGE/ota_ed25519.pk"

python3 - <<PY
import hashlib, json, pathlib
version = """$VERSION"""
root = pathlib.Path(r"""$ROOT""")
merge = root / f"dist/release-{version}"
index = {
    "product": "greenpng",
    "version": version,
    "architectures": {},
    "fe": {},
    "build_host": """$(uname -m)""",
}
for arch in """$TARGETS""".split():
    bundle = merge / f"greenpng-{version}-{arch}.tar.gz"
    assert bundle.is_file(), f"missing bundle for {arch}"
    triples = {"x86_64": "x86_64-linux-gnu", "aarch64": "aarch64-linux-gnu"}
    index["architectures"][arch] = {
        "triple": triples[arch],
        "bundle": bundle.name,
        "bundle_sha256": hashlib.sha256(bundle.read_bytes()).hexdigest(),
        "manifest": "manifest.json",
    }
fe = merge / f"fe-{version}.tgz"
index["fe"] = {"asset": fe.name, "sha256": hashlib.sha256(fe.read_bytes()).hexdigest()}
(merge / "manifest-index.json").write_text(json.dumps(index, indent=2) + "\n")
print(json.dumps(index, indent=2))
PY

echo "[multiarch] merged → $MERGE"
ls -la "$MERGE"

if [[ "${SKIP_PUBLISH:-0}" == "1" ]]; then
  echo "[multiarch] SKIP_PUBLISH=1 done"
  exit 0
fi

REPO="${GR_RELEASE_REPO:-greenpng/gr-server}"
if command -v gh >/dev/null 2>&1; then
  mapfile -t ASSETS < <(find "$MERGE" -maxdepth 1 -type f ! -name '*.sk' | sort)
  if gh release view "v${VERSION}" --repo "$REPO" >/dev/null 2>&1; then
    echo "[multiarch] release v${VERSION} exists — uploading new assets only (no clobber)"
    gh release upload "v${VERSION}" "${ASSETS[@]}" --repo "$REPO"
  else
    gh release create "v${VERSION}" "${ASSETS[@]}" --repo "$REPO" \
      --title "greenpng ${VERSION}" \
      --notes "Whole-bundle release (x86_64 + aarch64). See manifest-index.json."
  fi
else
  echo "[multiarch] gh not installed — artifacts ready at $MERGE"
fi
