#!/usr/bin/env bash
# Rebuild FE race/entry/boot/pin min bundles + ASSET_GEN for current VERSION.
# Standard C: ship artifacts only; content-hash URLs come from bootstrap at runtime.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"   # 02-probe-analysis (FE assets + scripts/fe node_modules)
REPO="$(cd "$ROOT/.." && pwd)"                # workspace root (VERSION lives here since the area split)
V="$(tr -d '[:space:]' < "$REPO/VERSION")"
echo "[fe-build] VERSION=$V ROOT=$ROOT REPO=$REPO"
FE="$ROOT/probe/fe"

# P1-6: per-release JS obfuscation. Enabled for release builds (GR_BUILD_ID
# set and != dev); dev/local builds stay terser-only for speed and stable
# diffs. Seed derives from build_id + version so each release differs:
# a patch crafted against the previous release's JS cannot be reused.
OBF_MODE=0
if [[ "${GR_OBFUSCATE_FE:-${GR_OBFUSCATE_FE:-}}" == "1" ]]; then
  OBF_MODE=1
elif [[ "${GR_OBFUSCATE_FE:-${GR_OBFUSCATE_FE:-}}" == "0" ]]; then
  OBF_MODE=0
elif [[ -n "${GR_BUILD_ID:-${GR_BUILD_ID:-}}" && "${GR_BUILD_ID}" != "dev" ]]; then
  OBF_MODE=1
fi
OBF_SEED="$(printf '%s|%s' "${GR_BUILD_ID:-${GR_BUILD_ID:-dev}}" "$V" | sha256sum | cut -c1-12)"
OBF_BIN="$ROOT/scripts/fe/node_modules/javascript-obfuscator/bin/javascript-obfuscator"
# terser ships in scripts/fe (npm dep); CI has no global terser, dev boxes may.
TERSER_BIN="$ROOT/scripts/fe/node_modules/.bin/terser"
[[ -x "$TERSER_BIN" ]] || TERSER_BIN="$(command -v terser || true)"
if [[ -z "$TERSER_BIN" ]]; then
  echo "[fe-build] ERROR: terser missing — run: (cd scripts/fe && npm install)" >&2
  exit 1
fi
if [[ "$OBF_MODE" == "1" && ! -x "$(command -v node)" ]]; then
  echo "[fe-build] ERROR: obfuscation enabled but node is unavailable" >&2
  exit 1
fi

obfuscate_js() {
  local src="$1" dst="$2" obf_out
  if [[ "$OBF_MODE" != "1" ]]; then
    if [[ "$src" != "$dst" ]]; then
      cp -f "$src" "$dst"
    fi
    return 0
  fi
  if [[ ! -f "$OBF_BIN" ]]; then
    echo "[fe-build] ERROR: javascript-obfuscator missing — run: (cd scripts/fe && npm install)" >&2
    exit 1
  fi
  # The CLI creates a temp dir next to its output; a pre-existing temp file
  # confuses it — use a dedicated work dir instead.
  local obf_work="$ROOT/target/fe-obf-work.$$"
  mkdir -p "$obf_work"
  obf_out="$obf_work/$(basename "$src")"
  if ! node "$OBF_BIN" "$src" \
    --output "$obf_out" \
    --compact true \
    --string-array true \
    --string-array-threshold 0.9 \
    --string-array-encoding "base64" \
    --identifier-names-generator "hexadecimal" \
    --rename-globals false \
    --simplify true \
    --numbers-to-expressions true \
    --dead-code-injection false \
    --self-defending false \
    --seed "$OBF_SEED"; then
    echo "[fe-build] javascript-obfuscator failed: $src" >&2
    rm -rf "$obf_work"
    return 1
  fi
  local sz
  sz=$(wc -c < "$obf_out")
  if [[ "$sz" -lt 500 ]]; then
    echo "[fe-build] obfuscator output too small ($sz) for $src" >&2
    rm -rf "$obf_work"
    return 1
  fi
  mv "$obf_out" "$dst"
  rm -rf "$obf_work"
  echo "[fe-build] obfuscated $dst ($(wc -c < "$dst") bytes, seed=$OBF_SEED)"
}

minify_stamp() {
  local src="$1" dst="$2"
  local tmp_min tmp_out
  tmp_min="$(mktemp)"
  tmp_out="$(mktemp)"
  # Always write terser to a temp file first (avoid empty dst race / stamp clobber).
  if ! "$TERSER_BIN" "$src" -c -m -o "$tmp_min"; then
    echo "[fe-build] terser failed: $src" >&2
    rm -f "$tmp_min" "$tmp_out"
    return 1
  fi
  local sz
  sz=$(wc -c < "$tmp_min")
  if [[ "$sz" -lt 500 ]]; then
    echo "[fe-build] terser output too small ($sz) for $src" >&2
    rm -f "$tmp_min" "$tmp_out"
    return 1
  fi
  {
    printf 'window.__GR_BUILD_IMPL__="%s";\n' "$V"
    cat "$tmp_min"
  } > "$tmp_out"
  mv "$tmp_out" "$dst"
  rm -f "$tmp_min"
  echo "[fe-build] min $dst ($(wc -c < "$dst") bytes)"
  # P1-6: obfuscate in place (per-release seed) — after the version stamp so
  # the __GR_BUILD_IMPL__ marker stays intact for runtime boot logic.
  obfuscate_js "$dst" "$dst"
}

{
  echo "/* green-v6 race pack | $V */"
  echo
  for part in gr.fe_impl.js gr.privacy_guard.js probe_lifecycle.js session_scheduler.js \
              origin_coordinator.js probe_method_matrix.js probe_self_heal.js gr.seal.js upload_queue.js \
              pack_loader.js collectors/l1.js; do
    echo "/* ---- $part ---- */"
    cat "$FE/$part"
    echo
  done
} > "$FE/gr.race.concat.js"
minify_stamp "$FE/gr.race.concat.js" "$FE/gr.race.min.js"

{
  echo "/* green-v6 entry | $V */"
  echo
  # probe_self_heal + origin_coordinator + method matrix MUST ship in entry (pin loads entry not race).
  for part in gr.fe_impl.js storage.js probe_lifecycle.js session_scheduler.js \
              origin_coordinator.js probe_method_matrix.js probe_self_heal.js ops_report.js upload_queue.js \
              pack_loader.js collectors/l1.js gr.boot.js; do
    echo "/* ---- $part ---- */"
    cat "$FE/$part"
    echo
  done
} > "$FE/gr.entry.concat.js"
minify_stamp "$FE/gr.entry.concat.js" "$FE/gr.entry.min.js"

minify_stamp "$FE/gr.boot.js" "$FE/gr.boot.min.js"
minify_stamp "$FE/pack_loader.js" "$FE/pack_loader.min.js"
minify_stamp "$FE/gr.js" "$FE/gr.min.js"
if [[ -f "$FE/gr.micro.js" ]]; then minify_stamp "$FE/gr.micro.js" "$FE/gr.micro.min.js"; fi
cp -f "$FE/gr.min.js" "$FE/gr.pin.js"

# Standalone privacy guard min: handlers.rs serves it as a secondary asset and
# gr.boot.js may load it outside the race bundle. Regenerate from the same
# source so the standalone copy cannot drift from the inlined race copy.
minify_stamp "$FE/gr.privacy_guard.js" "$FE/gr.privacy_guard.min.js"

# optional standalones
if [[ -f "$FE/gr.seal.js" ]]; then minify_stamp "$FE/gr.seal.js" "$FE/gr.seal.min.js" || true; fi
if [[ -f "$FE/gr.loader.js" ]]; then minify_stamp "$FE/gr.loader.js" "$FE/gr.loader.min.js" || true; fi
if [[ -f "$FE/sandbox_tree.js" ]]; then minify_stamp "$FE/sandbox_tree.js" "$FE/sandbox_tree.min.js" || true; fi
if [[ -f "$FE/rpa_monitor.js" ]]; then minify_stamp "$FE/rpa_monitor.js" "$FE/rpa_monitor.min.js" || true; fi

# Keep every generated bundle on the same release marker. Collector and
# auxiliary minified bundles are often generated by separate pack jobs and
# can otherwise overwrite __GR_BUILD_IMPL__ with an older release value.
python3 - "$FE" "$V" <<'PY'
from pathlib import Path
import re
import sys

root, version = Path(sys.argv[1]), sys.argv[2]
marker = re.compile(r'window\.__GR_BUILD_IMPL__\s*=\s*["\'][^"\']+["\']\s*;')
for path in root.rglob("*.min.js"):
    text = path.read_text(errors="ignore")
    updated, count = marker.subn(f'window.__GR_BUILD_IMPL__="{version}";', text, count=1)
    if count and updated != text:
        path.write_text(updated)
PY

GEN=$(sha256sum "$FE/gr.race.min.js" | awk '{print substr($1,1,12)}')
echo -n "$GEN" > "$FE/ASSET_GEN"
echo -n "$V" > "$FE/VERSION"
echo "[fe-build] ASSET_GEN=$GEN VERSION=$V done"
