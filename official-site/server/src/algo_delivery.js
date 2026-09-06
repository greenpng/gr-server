/**
 * Algorithm delivery crypto helpers (protocol v2).
 *
 * Industry pattern: per-pull ECDH (X25519) → HKDF-SHA256 → AES-256-GCM.
 * Lab may still use shared wrap key when GV6_OFFICIAL_EXPOSE_WRAP_KEY=1.
 */
import crypto from "node:crypto";

export function generateNodeKeypair() {
  const { publicKey, privateKey } = crypto.generateKeyPairSync("x25519");
  return {
    public_b64: publicKey.export({ type: "spki", format: "der" }).toString("base64"),
    private_pem: privateKey.export({ type: "pkcs8", format: "pem" }),
  };
}

/** Derive 32-byte AES key from our X25519 priv + peer pub (HKDF-SHA256). */
export function ecdhAesKey(ourPrivatePem, peerPublicSpkiB64, info = "gv6-algo-bundle-v2") {
  const priv = crypto.createPrivateKey(ourPrivatePem);
  const peer = crypto.createPublicKey({
    key: Buffer.from(peerPublicSpkiB64, "base64"),
    type: "spki",
    format: "der",
  });
  const shared = crypto.diffieHellman({ privateKey: priv, publicKey: peer });
  return Buffer.from(
    crypto.hkdfSync("sha256", shared, Buffer.alloc(0), Buffer.from(info, "utf8"), 32)
  );
}

export function encryptWithKey(aesKey32, plaintextObj) {
  const iv = crypto.randomBytes(12);
  const cipher = crypto.createCipheriv("aes-256-gcm", aesKey32, iv);
  const pt = Buffer.from(JSON.stringify(plaintextObj), "utf8");
  const ct = Buffer.concat([cipher.update(pt), cipher.final()]);
  const tag = cipher.getAuthTag();
  return {
    iv_b64: iv.toString("base64"),
    ciphertext_b64: Buffer.concat([ct, tag]).toString("base64"),
  };
}

/** Decrypt AES-GCM blob (ct||tag) produced by encryptWithKey. */
export function decryptWithKey(aesKey32, ivB64, ciphertextB64) {
  const iv = Buffer.from(ivB64, "base64");
  const raw = Buffer.from(ciphertextB64, "base64");
  const tag = raw.subarray(raw.length - 16);
  const ct = raw.subarray(0, raw.length - 16);
  const decipher = crypto.createDecipheriv("aes-256-gcm", aesKey32, iv);
  decipher.setAuthTag(tag);
  const pt = Buffer.concat([decipher.update(ct), decipher.final()]);
  return JSON.parse(pt.toString("utf8"));
}

export function buildAlgoBundleHeader({
  site_id,
  domain,
  plan,
  instance_id,
  node_ephemeral_pub_b64,
  server_ephemeral_pub_b64,
  exp_unix,
  bundle_id,
}) {
  return {
    v: 2,
    alg: "EdDSA",
    enc: "A256GCM",
    kex: "X25519-HKDF-SHA256",
    kid: "official-1",
    site_id,
    domain,
    plan,
    instance_id: instance_id || null,
    node_ephemeral_pub_b64,
    server_ephemeral_pub_b64,
    exp: exp_unix,
    bundle_id,
  };
}
