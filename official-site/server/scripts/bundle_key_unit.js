// P1-3 regression: bundle-key material validation (env/file/legacy) must
// reject non-Ed25519 keys even though createPrivateKey can parse them.
import assert from "node:assert/strict";
import crypto from "node:crypto";
import { assertBundleKeyMaterial } from "../src/account_security.js";

function edKeyPem() {
  const { privateKey } = crypto.generateKeyPairSync("ed25519");
  return privateKey.export({ type: "pkcs8", format: "pem" });
}
function rsaKeyPem() {
  const { privateKey } = crypto.generateKeyPairSync("rsa", { modulusLength: 2048 });
  return privateKey.export({ type: "pkcs8", format: "pem" });
}

// Valid Ed25519 material passes.
assertBundleKeyMaterial({
  ed25519_private_pem: edKeyPem(),
  wrap_key_b64: crypto.randomBytes(32).toString("base64"),
});
console.log("PASS  bundle key material: valid ed25519 + 32B wrap key accepted");

// RSA PEM parses with createPrivateKey but must be rejected here.
assert.throws(
  () =>
    assertBundleKeyMaterial({
      ed25519_private_pem: rsaKeyPem(),
      wrap_key_b64: crypto.randomBytes(32).toString("base64"),
    }),
  /ed25519/
);
console.log("PASS  bundle key material: non-ed25519 PEM rejected");

// Wrong wrap-key length rejected.
assert.throws(
  () =>
    assertBundleKeyMaterial({
      ed25519_private_pem: edKeyPem(),
      wrap_key_b64: crypto.randomBytes(16).toString("base64"),
    }),
  /wrap_key/
);
console.log("PASS  bundle key material: wrong wrap-key length rejected");

// Missing / garbage fields rejected.
assert.throws(() => assertBundleKeyMaterial({}), /required/);
assert.throws(
  () => assertBundleKeyMaterial({ ed25519_private_pem: "not-a-pem", wrap_key_b64: crypto.randomBytes(32).toString("base64") }),
  /key|require/i
);
console.log("PASS  bundle key material: missing/blank fields rejected");
