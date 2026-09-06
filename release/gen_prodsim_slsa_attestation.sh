#!/usr/bin/env bash
# Local SLSA provenance simulation (P2-9 local equivalence).
#
# Produces an in-toto v0.1 Statement with a generated SLSA v1 predicate listing
# the release artifacts of the workspace --release build (binaries, FE bundles,
# catalog/spec) with sha256 digests, builder metadata and the exact build steps.
# This mechanically mirrors what a GitHub `attest-build-provenance` action would
# emit for the same source tree, without requiring a cloud CI workflow.
#
# Honest boundary: the GitHub-native attestation (signed with Sigstore by GH)
# still requires the repository to move to a cloud CI with the official action;
# this artifact covers the predicate/material part and is stored per build.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
VERSION="${VERSION:-$(grep -m1 '^version' "$ROOT/Cargo.toml" | sed -E 's/.*"([^"]+)"/\1/')}"
TS="$(date -u +%Y%m%dT%H%M%SZ)"
OUT_DIR="${SLSA_OUT:-$ROOT/reports/slsa}"
mkdir -p "$OUT_DIR"
OUT="$OUT_DIR/provenance-$VERSION-$TS.json"
SUMS="$OUT_DIR/sha256sums-$VERSION-$TS.txt"

# Material set: everything user/ops need provenance over.
MATERIALS=(
  "$ROOT/target/release/gr-service"
  "$ROOT/target/release/gr-cli"
  "$ROOT/02-probe-analysis/probe/fe/pack_loader.min.js"
  "$ROOT/02-probe-analysis/probe/fe/gv5.seal.js"
  "$ROOT/02-probe-analysis/probe/fe/gr.boot.min.js"
  "$ROOT/02-probe-analysis/probe/fe/gv5.entry.min.js"
  "$ROOT/02-probe-analysis/probe/fe/gv5.race.min.js"
  "$ROOT/02-probe-analysis/probe/fe/ASSET_GEN"
  "$ROOT/02-probe-analysis/spec/component_catalog.json"
  "$ROOT/02-probe-analysis/spec/layers_v11.json"
)
: > "$SUMS"
SUBJECTS=()
for f in "${MATERIALS[@]}"; do
  if [[ -f "$f" ]]; then
    H=$(sha256sum "$f" | awk '{print $1}')
    B=$(basename "$f")
    echo "$H  $B" >> "$SUMS"
    SUBJECTS+=("{\"name\":\"$B\",\"digest\":{\"sha256\":\"$H\"}}")
  fi
done
[[ ${#SUBJECTS[@]} -gt 0 ]] || { echo "no release artifacts found" >&2; exit 1; }
SUBJ_JSON=$(python3 - <<PY
import json
s=[$(
  printf '%s,' "${SUBJECTS[@]}"
) ]
print(json.dumps(s))
PY
)

GIT_HEAD=$(git -C "$ROOT" rev-parse HEAD 2>/dev/null || echo "uncommitted-tree")
GIT_DIRTY=$(git -C "$ROOT" status --porcelain 2>/dev/null | wc -l)
RUSTC_VER="$(rustc --version 2>/dev/null || echo unknown)"

python3 - <<PY
import json
out = {
  "_type": "https://in-toto.io/Statement/v0.1",
  "subject": json.loads("""$SUBJ_JSON"""),
  "predicateType": "https://slsa.dev/provenance/v1",
  "predicate": {
    "buildDefinition": {
      "buildType": "https://github.com/slsa-framework/slsa-github-generator/generic@v2",
      "externalParameters": {
        "repository": "$ROOT",
        "workflow": "local-prodsim-release-build",
        "rustcVersion": "$RUSTC_VER",
        "buildCommand": "CARGO_BUILD_JOBS=2 cargo build --release --workspace --offline",
        "feBundleCommand": "bash fe/rebuild_bundles.sh"
      },
      "internalParameters": {
        "source": {
          "gitHead": "$GIT_HEAD",
          "dirtyEntries": $GIT_DIRTY
        },
        "environment": {
          "host": "$(hostname 2>/dev/null || echo local)",
          "image": "ubuntu:24.04 (anti-detect lab)",
          "dockerCluster": "03-local-test-lab/tests/multi-node/docker (node1+node2+PG+LBalancer)"
        }
      },
      "resolvedDependencies": [
        {"uri": "git+$ROOT", "digest": {"sha1": "$GIT_HEAD"}}
      ]
    },
    "runDetails": {
      "builder": {"id": "https://github.com/green-v7/local-prodsim"},
      "metadata": {
        "invocationId": "prodsim-$TS",
        "startedOn": "$TS",
        "finishedOn": "$TS"
      },
      "byproducts": [{"name": "sha256sums", "uri": "$(basename "$SUMS")"}]
    }
  }
}
open("$OUT", "w").write(json.dumps(out, indent=2) + "\n")
print(json.dumps({"out": "$OUT", "subjects": len(out["subject"]), "git": "$GIT_HEAD", "dirty": $GIT_DIRTY}, ensure_ascii=False))
PY

echo "[slsa] $OUT"
echo "[slsa] $SUMS"
