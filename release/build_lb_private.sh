#!/usr/bin/env bash
# Build the PAID lb module for PRIVATE distribution (docs/guides/08-LB-MODULE.md).
#
# The lb module is intentionally NOT part of public releases:
#   - it is not listed in 04-release-github-ci/release/build_multiarch.sh's module loop
#   - this script produces `dist/lb-private/` only — a private transfer to
#     paying customers, NEVER a GitHub release asset
#   - even on the customer node, module activation requires a signed
#     system license with `lb.enabled` (runtime + CLI + module gates)
#
# Usage:
#   bash 04-release-github-ci/release/build_lb_private.sh            # host arch, repo keys
#   TARGETS=aarch64 bash 04-release-github-ci/release/build_lb_private.sh   # cross (docker) arch
#   GV6_LB_PRIVATE_KEYS_DIR=/path bash 04-release-github-ci/release/build_lb_private.sh
#
# Env:
#   TARGETS                  x86_64 aarch64 (default: host)
#   GV6_LB_PRIVATE_KEYS_DIR  dir with ota_ed25519.sk/.pk (default repo keys/)
#   USE_CROSS                1 for foreign arch via `cross`
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
VERSION="$(tr -d '[:space:]' < VERSION)"
KEYS_DIR="${GV6_LB_PRIVATE_KEYS_DIR:-$ROOT/keys}"
OUT="$ROOT/dist/lb-private"
mkdir -p "$OUT"

HOST_ARCH="$(uname -m)"; case "$HOST_ARCH" in x86_64|amd64) HOST_ARCH=x86_64;; aarch64|arm64) HOST_ARCH=aarch64;; esac
TARGETS="${TARGETS:-$HOST_ARCH}"

triple_for() { case "$1" in x86_64) echo "x86_64-linux-gnu";; aarch64) echo "aarch64-linux-gnu";; *) return 1;; esac; }
rust_target_for() { case "$1" in x86_64) echo "x86_64-unknown-linux-gnu";; aarch64) echo "aarch64-unknown-linux-gnu";; *) return 1;; esac; }

[[ -f "$KEYS_DIR/ota_ed25519.sk" ]] || { echo "[lb-private] missing $KEYS_DIR/ota_ed25519.sk" >&2; exit 1; }

build_arch() {
  local arch="$1"; local triple; local rust_target
  triple="$(triple_for "$arch")"
  rust_target="$(rust_target_for "$arch")"
  echo "[lb-private] building lb module $arch ($rust_target) version=$VERSION"
  local so=""
  if [[ "$arch" == "$HOST_ARCH" ]]; then
    GV6_RELEASE_VERSION="$VERSION" cargo build -p gr-module-lb --release --features plugin
    so="$ROOT/target/release/libgr_lb.so"
  else
    [[ "${USE_CROSS:-0}" == "1" ]] || { echo "[lb-private] foreign arch needs USE_CROSS=1" >&2; exit 1; }
    GV6_RELEASE_VERSION="$VERSION" cross build -p gr-module-lb --release --features plugin --target "$rust_target"
    so="$ROOT/target/$rust_target/release/libgr_lb.so"
  fi
  [[ -f "$so" ]] || { echo "[lb-private] missing $so" >&2; exit 1; }
  local asset="libgr_lb-${VERSION}-${triple}.so"
  cp -f "$so" "$OUT/$asset"
  local art
  art=$(cargo run -q -p gr-cli -- sign-module \
    --name lb --version "$VERSION" --so "$OUT/$asset" \
    --secret-key "$KEYS_DIR/ota_ed25519.sk" --domain lb)
  printf '%s' "$art" > "$OUT/artifact-lb-${triple}.json"
  # Per-release key dual-sign (P0-2) when a release key pair is provided.
  if [[ -f "$KEYS_DIR/release_ed25519.sk" ]]; then
    local rk
    rk=$(cargo run -q -p gr-cli -- sign-module \
      --name lb --version "$VERSION" --so "$OUT/$asset" \
      --secret-key "$KEYS_DIR/ota_ed25519.sk" --domain lb \
      --release-key "$KEYS_DIR/release_ed25519.sk")
    printf '%s' "$rk" > "$OUT/artifact-lb-${triple}.json"
  fi
  # 公钥随私域分发 (customer 侧 CLI stage 需要)
  [[ -f "$KEYS_DIR/ota_ed25519.pk" ]] && cp -f "$KEYS_DIR/ota_ed25519.pk" "$OUT/ota_ed25519.pk"
  sha256sum "$OUT/$asset" | tee "$OUT/$asset.sha256"
  echo "[lb-private] done: $OUT/$asset"
}

for a in $TARGETS; do build_arch "$a"; done

echo
echo "[lb-private] private distribution box: $OUT"
echo "  distribute ONLY to paying customers (docs/guides/08-LB-MODULE.md); never upload to public repos."
ls -l "$OUT"
