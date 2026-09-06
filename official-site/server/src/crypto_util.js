import crypto from "node:crypto";

export function randomToken(nbytes = 24) {
  return crypto.randomBytes(nbytes).toString("hex");
}

export function b64url(buf) {
  return Buffer.from(buf)
    .toString("base64")
    .replace(/\+/g, "-")
    .replace(/\//g, "_")
    .replace(/=+$/g, "");
}

export function sha256b64url(s) {
  return b64url(crypto.createHash("sha256").update(String(s), "utf8").digest());
}

/** scrypt password hash: scrypt$N$r$p$salt$dk */
export function hashPassword(password) {
  const salt = crypto.randomBytes(16);
  const N = 16384;
  const r = 8;
  const p = 1;
  const dk = crypto.scryptSync(password, salt, 32, { N, r, p });
  return `scrypt$${N}$${r}$${p}$${salt.toString("base64")}$${dk.toString("base64")}`;
}

export function verifyPassword(password, stored) {
  try {
    const [, N, r, p, saltB64, dkB64] = stored.split("$");
    const salt = Buffer.from(saltB64, "base64");
    const want = Buffer.from(dkB64, "base64");
    const got = crypto.scryptSync(password, salt, want.length, {
      N: Number(N),
      r: Number(r),
      p: Number(p),
    });
    return crypto.timingSafeEqual(want, got);
  } catch {
    return false;
  }
}
