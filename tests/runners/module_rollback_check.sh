#!/usr/bin/env bash
# Module hot-rollback drill (fixture level, runs on any arch):
#  1) stage+activate module v1 → active link points at versions/<name>/1.0.0
#  2) stage+activate v2 → link flips to 2.0.0, v1 version dir retained
#  3) rollback activate v1 → instant, no re-download; link back to 1.0.0
#  4) activating a never-staged version fails and leaves the link untouched
#  5) staging a tampered artifact (sha mismatch vs signed body) is rejected
#
# This is the module leg of the four-state rollback doctrine (runtime / FE /
# module / entitlement); runtime+FE are covered by upgrade_rollback_check.sh.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
TMP="$(mktemp -d /tmp/v7-module-rollback.XXXXXX)"
trap 'rm -rf "$TMP"' EXIT

GR="$ROOT/target/debug/gr"
if [[ ! -x "$GR" ]]; then
  echo "[module-rollback] building gr debug CLI…"
  (cd "$ROOT" && cargo build -p gr-cli)
fi

cd "$ROOT"
KEYS="$TMP/keys"; MODS="$TMP/modules"; DATA="$TMP/data"
mkdir -p "$KEYS" "$MODS" "$DATA"
"$GR" keygen --out-dir "$KEYS" >/dev/null
PK="$KEYS/ota_ed25519.pk"
SK="$KEYS/ota_ed25519.sk"
[[ $(wc -c < "$PK") -eq 32 ]] || { echo "keygen failed" >&2; exit 1; }

# two byte-different module payloads (content A/B)
cp /bin/true  "$TMP/mod_a.so"
cp /bin/false "$TMP/mod_b.so"

art1="$("$GR" sign-module --name brain --version 1.0.0 --so "$TMP/mod_a.so" --secret-key "$SK" --domain brain)"
art2="$("$GR" sign-module --name brain --version 2.0.0 --so "$TMP/mod_b.so" --secret-key "$SK" --domain brain)"
printf '%s' "$art1" > "$TMP/art1.json"
printf '%s' "$art2" > "$TMP/art2.json"

stage() { # stage <artifact-json> <so>
  "$GR" stage --modules-dir "$MODS" --pubkey "$PK" --artifact-json "$1" --so "$2" \
    --runtime-version 7.0.2 --data-dir "$DATA"
}
link_of() { cat "$MODS/active/brain" 2>/dev/null || true; }
expect_link() { # expect_link <substr>
  local l
  l="$(link_of)"
  [[ "$l" == *"$1"* ]] || { echo "FAIL: active link expected *$1* got: $l" >&2; exit 1; }
}

# 1) v1 staged + activated
stage "$TMP/art1.json" "$TMP/mod_a.so" >/dev/null
"$GR" activate --modules-dir "$MODS" --name brain --version 1.0.0 --data-dir "$DATA" >/dev/null
expect_link "versions/brain/1.0.0"
[[ -f "$MODS/versions/brain/1.0.0/mod_a.so" ]]
echo "V7_MODULE_STAGE_ACTIVATE_V1_OK"

# 2) v2 staged + activated; v1 retained for rollback
stage "$TMP/art2.json" "$TMP/mod_b.so" >/dev/null
"$GR" activate --modules-dir "$MODS" --name brain --version 2.0.0 --data-dir "$DATA" >/dev/null
expect_link "versions/brain/2.0.0"
[[ -d "$MODS/versions/brain/1.0.0" ]] || { echo "FAIL: v1 version dir pruned too early" >&2; exit 1; }
echo "V7_MODULE_UPGRADE_V2_KEEPS_V1_OK"

# 3) instant rollback to v1
"$GR" activate --modules-dir "$MODS" --name brain --version 1.0.0 --data-dir "$DATA" >/dev/null
expect_link "versions/brain/1.0.0"
echo "V7_MODULE_ROLLBACK_V1_OK"

# 4) never-staged version rejected; link untouched
if "$GR" activate --modules-dir "$MODS" --name brain --version 9.9.9 --data-dir "$DATA" >/dev/null 2>&1; then
  echo "FAIL: activating unstaged 9.9.9 unexpectedly succeeded" >&2
  exit 1
fi
expect_link "versions/brain/1.0.0"
echo "V7_MODULE_UNSTAGED_REJECT_OK"

# 5) tampered artifact rejected at stage (sha in signed body != file)
python3 - "$TMP/art2.json" "$TMP/art2-tampered.json" <<'PY'
import json, sys
a = json.load(open(sys.argv[1]))
a["sha256"] = "0" * 64  # mismatch with the actual file
open(sys.argv[2], "w").write(json.dumps(a))
PY
if stage "$TMP/art2-tampered.json" "$TMP/mod_b.so" >/dev/null 2>&1; then
  echo "FAIL: tampered artifact staged successfully" >&2
  exit 1
fi
expect_link "versions/brain/1.0.0"
echo "V7_MODULE_TAMPER_REJECT_OK"

echo "V7_MODULE_ROLLBACK_PASS"
