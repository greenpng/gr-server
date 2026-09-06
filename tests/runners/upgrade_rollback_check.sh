#!/usr/bin/env bash
# V7 upgrade + version-control contract (runs on GitHub Runners, both archs):
#  1) good install → versioned slot bin/releases/<ver>/, VERSION, dist manifest
#  2) tampered release (bad runtime sha) → rejected, previous binaries retained
#  3) good upgrade with FE tarball → fe/ installed, fe/VERSION synced, old slot
#     AND old FE backup retained (rollback safety)
#  4) tampered FE sha → rejected before install, nothing mutates
#
# Layout: green-v7 repo copy lives at 03-local-test-lab/tests/runners/upgrade_rollback_check.sh,
# root = ../.. (the public install repo copy at test/ differs by one level).
set -euo pipefail

# 仓库根解析: 从脚本位置向上找 Cargo.toml (编号布局 / 扁平发行布局两用)
ROOT="$(cd "$(dirname "$0")" && pwd)"
while [[ "$ROOT" != "/" && ! -f "$ROOT/Cargo.toml" ]]; do ROOT="$(dirname "$ROOT")"; done
[[ -f "$ROOT/Cargo.toml" ]] || { echo "[FATAL] cannot locate workspace root (Cargo.toml) from $0" >&2; exit 1; }
TMP="$(mktemp -d /tmp/v7-upgrade-check.XXXXXX)"
PORT="$((18000 + RANDOM % 1000))"
SERVER_PID=""
cleanup() {
  [[ -n "$SERVER_PID" ]] && kill "$SERVER_PID" 2>/dev/null || true
  rm -rf "$TMP"
}
trap cleanup EXIT

host_arch="$(uname -m)"
case "$host_arch" in
  x86_64|amd64) host_arch=x86_64 ;;
  aarch64|arm64) host_arch=aarch64 ;;
  *) echo "unsupported host architecture: $host_arch" >&2; exit 1 ;;
esac
triple="${host_arch}-linux-gnu"

mkdir -p "$TMP/v7.0.0" "$TMP/v7.0.1" "$TMP/v7.0.2" "$TMP/v7.0.3" "$TMP/install/bin"
cp /bin/sh "$TMP/v7.0.0/gr-service-7.0.0-${triple}"        # good 7.0.0
cp /bin/false "$TMP/v7.0.1/gr-service-7.0.1-${triple}"     # tampered runtime sha in manifest
cp /bin/true "$TMP/v7.0.2/gr-service-7.0.2-${triple}"      # good 7.0.2 upgrade
sha0="$(sha256sum "$TMP/v7.0.0/gr-service-7.0.0-${triple}" | awk '{print $1}')"
sha2="$(sha256sum "$TMP/v7.0.2/gr-service-7.0.2-${triple}" | awk '{print $1}')"
cat > "$TMP/v7.0.0/manifest-${triple}.json" <<EOF
{"runtime":{"asset":"gr-service-7.0.0-${triple}","sha256":"$sha0"},"fe":null}
EOF
cat > "$TMP/v7.0.1/manifest-${triple}.json" <<EOF
{"runtime":{"asset":"gr-service-7.0.1-${triple}","sha256":"deliberately-wrong"},"fe":null}
EOF
# 7.0.2 ships FE: tarball fe/ subtree + sha in manifest
mkdir -p "$TMP/v7.0.2/fe-asset/fe"
printf 'v7.0.2-seal\n' > "$TMP/v7.0.2/fe-asset/fe/seal.js"
tar -C "$TMP/v7.0.2/fe-asset" -czf "$TMP/v7.0.2/fe-7.0.2.tgz" fe
fe_sha="$(sha256sum "$TMP/v7.0.2/fe-7.0.2.tgz" | awk '{print $1}')"
cat > "$TMP/v7.0.2/manifest-${triple}.json" <<EOF
{"runtime":{"asset":"gr-service-7.0.2-${triple}","sha256":"$sha2"},"fe":{"asset":"fe-7.0.2.tgz","sha256":"$fe_sha"}}
EOF
# 7.0.3 ships FE with a deliberately wrong fe sha → must be rejected pre-install
mkdir -p "$TMP/v7.0.3/fe-asset/fe"
printf 'v7.0.3-seal\n' > "$TMP/v7.0.3/fe-asset/fe/seal.js"
tar -C "$TMP/v7.0.3/fe-asset" -czf "$TMP/v7.0.3/fe-7.0.3.tgz" fe
cat > "$TMP/v7.0.3/manifest-${triple}.json" <<EOF
{"runtime":{"asset":"gr-service-7.0.3-${triple}","sha256":"$sha2"},"fe":{"asset":"fe-7.0.3.tgz","sha256":"deadbeef"}}
EOF

python3 -m http.server "$PORT" --bind 127.0.0.1 --directory "$TMP" >/dev/null 2>&1 &
SERVER_PID=$!
for _ in $(seq 1 30); do
  curl -fsS "http://127.0.0.1:${PORT}/v7.0.0/manifest-${triple}.json" >/dev/null 2>&1 && break
  sleep 0.2
done

cur_bin_sha() { sha256sum "$TMP/install/bin/gr-service" | awk '{print $1}'; }
INST="$TMP/install"

# --- 1) good install 7.0.0: slot + VERSION + dist manifest + fe-not-updated ---
INSTALL_ROOT="$INST" \
VERSION=7.0.0 \
GR_RELEASE_BASE="http://127.0.0.1:${PORT}/v7.0.0" \
REQUIRE_SHA=1 \
GR_ALLOW_UNSIGNED_FIXTURE=1 \
bash "$ROOT/04-release-github-ci/release/update_runtime_from_github.sh"
test -x "$INST/bin/gr-service"
test "$(cur_bin_sha)" = "$sha0"
test -x "$INST/bin/releases/7.0.0/gr-service"
test "$(sha256sum "$INST/bin/releases/7.0.0/gr-service" | awk '{print $1}')" = "$sha0"
test -f "$INST/dist/release-7.0.0/manifest.json"
test "$(cat "$INST/VERSION")" = "7.0.0"
echo "V7_UPGRADE_SLOT_7_0_0_OK"

# --- 2) tampered 7.0.1 (bad runtime sha) → rejected, nothing mutated ---
if INSTALL_ROOT="$INST" \
  VERSION=7.0.1 \
  GR_RELEASE_BASE="http://127.0.0.1:${PORT}/v7.0.1" \
  REQUIRE_SHA=1 \
  GR_ALLOW_UNSIGNED_FIXTURE=1 \
  bash "$ROOT/04-release-github-ci/release/update_runtime_from_github.sh" >/tmp/v7-upgrade-bad.log 2>&1; then
  echo "tampered manifest unexpectedly accepted" >&2
  exit 1
fi
test "$(cur_bin_sha)" = "$sha0"
test "$(cat "$INST/VERSION")" = "7.0.0"
test ! -e "$INST/bin/releases/7.0.1"
test ! -d "$INST/dist/release-7.0.1"
echo "V7_UPGRADE_TAMPER_REJECT_OK"

# --- 3) good upgrade 7.0.2 with FE: slots stack, FE synced, old slot retained ---
# pre-existing FE tree (as on a real server after a previous FE deployment)
mkdir -p "$INST/fe"
printf 'legacy-fe\n' > "$INST/fe/legacy.js"
INSTALL_ROOT="$INST" \
VERSION=7.0.2 \
GR_RELEASE_BASE="http://127.0.0.1:${PORT}/v7.0.2" \
REQUIRE_SHA=1 \
GR_ALLOW_UNSIGNED_FIXTURE=1 \
bash "$ROOT/04-release-github-ci/release/update_runtime_from_github.sh"
test "$(cur_bin_sha)" = "$sha2"
test "$(cat "$INST/VERSION")" = "7.0.2"
# new versioned slot + previous slot retained for rollback
test -x "$INST/bin/releases/7.0.2/gr-service"
test "$(sha256sum "$INST/bin/releases/7.0.2/gr-service" | awk '{print $1}')" = "$sha2"
test -x "$INST/bin/releases/7.0.0/gr-service"
test "$(sha256sum "$INST/bin/releases/7.0.0/gr-service" | awk '{print $1}')" = "$sha0"
test -f "$INST/dist/release-7.0.2/manifest.json"
# FE installed from tarball + fe/VERSION synced to product version
test -f "$INST/fe/seal.js"
test "$(cat "$INST/fe/VERSION")" = "7.0.2"
# FE backup of the pre-upgrade tree is kept (rollback path) with old content
tmp_febaks=("$INST"/fe.bak.*)
test "${#tmp_febaks[@]}" -ge 1
test "$(grep -c 'legacy-fe' "$INST"/fe.bak.*/legacy.js)" -ge 1
echo "V7_UPGRADE_SLOT_FE_7_0_2_OK"

# --- 4) tampered 7.0.3 FE sha → rejected before install, nothing mutated ---
if INSTALL_ROOT="$INST" \
  VERSION=7.0.3 \
  GR_RELEASE_BASE="http://127.0.0.1:${PORT}/v7.0.3" \
  REQUIRE_SHA=1 \
  GR_ALLOW_UNSIGNED_FIXTURE=1 \
  bash "$ROOT/04-release-github-ci/release/update_runtime_from_github.sh" >/tmp/v7-upgrade-fe-bad.log 2>&1; then
  echo "tampered FE manifest unexpectedly accepted" >&2
  exit 1
fi
test "$(cur_bin_sha)" = "$sha2"
test "$(cat "$INST/VERSION")" = "7.0.2"
test ! -e "$INST/bin/releases/7.0.3"
test "$(cat "$INST/fe/VERSION")" = "7.0.2"
echo "V7_UPGRADE_FE_TAMPER_REJECT_OK"
echo "V7_UPGRADE_ROLLBACK_SAFETY_PASS"
