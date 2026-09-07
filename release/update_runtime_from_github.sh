#!/usr/bin/env bash
# Compat shim — the real updater lives at install/release/ (synced flat into
# gr-server as install/release/update_runtime_from_github.sh). Whole-bundle
# flow: index → bundle sha256 → safe extract → in-bundle signed manifest.
set -euo pipefail
exec bash "$(cd "$(dirname "$0")" && pwd)/../install/release/update_runtime_from_github.sh" "$@"
