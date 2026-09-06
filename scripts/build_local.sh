#!/usr/bin/env bash
# Lean local build for green-v6 (independent workspace; no green-v5).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
JOBS="${CARGO_BUILD_JOBS:-2}"
export CARGO_BUILD_JOBS="$JOBS"
echo "[gr] cargo build -p gr-service --jobs $JOBS (cwd=$ROOT)"
exec cargo build -p gr-service --jobs "$JOBS" "$@"
