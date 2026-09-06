#!/usr/bin/env bash
# iss/opus5 05 low (wasm provenance): verify the committed seal_v2 wasm binary
# against its pinned content hash and the server-declared module id.
#
# The seal wasm is shipped prebuilt (no .wat/.c source is tracked in-tree, see
# docs/guides/05-RELEASE.md §seal-v2 wasm provenance). This check closes the gap by
# making the *binary* verifiable: any silent replacement of the artifact in a
# release build fails CI, and the module id advertised by the Rust server must
# match the canonical release.
#
# Usage: 04-release-github-ci/release/check_seal_wasm.sh [path/to/gr.seal_v2.wasm]
set -u

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WASM="${1:-$ROOT/02-probe-analysis/probe/fe/gr.seal_v2.wasm}"
# Pinned canonical hash of the committed seal_v2 module (provenance record).
PINNED_SHA256="c5b97b9ad21c3dc3a47b02d55f1409dc1d4442409b14b2a4dd81cd0d45ef4409"
# Must match 02-probe-analysis/crates/gr-probe-core/src/seal_v2.rs::SEAL_WASM_MODULE_ID.
MODULE_ID="wm-v2-s2-20260806"

fail() { echo "seal-wasm check FAILED: $1" >&2; exit 1; }

[ -f "$WASM" ] || fail "wasm not found at $WASM"

actual="$(sha256sum "$WASM" | cut -d' ' -f1)"
[ "$actual" = "$PINNED_SHA256" ] || fail "content hash mismatch for $WASM
  pinned:  $PINNED_SHA256
  actual:  $actual
(binary replaced / rebuilt? re-pin only after a controlled rebuild + seal audit)"

# Server-side module id must match the pinned binary's release.
if [ -f "$ROOT/02-probe-analysis/crates/gr-probe-core/src/seal_v2.rs" ]; then
  grep -q "SEAL_WASM_MODULE_ID: &str = \"$MODULE_ID\"" \
    "$ROOT/02-probe-analysis/crates/gr-probe-core/src/seal_v2.rs" \
    || fail "module id mismatch: Rust declares an id != $MODULE_ID"
fi

echo "seal-wasm check OK: $WASM matches pinned sha256 ($PINNED_SHA256) and module id $MODULE_ID"
