/**
 * Account security helpers: email verify tokens, TOTP 2FA, basic ticket shaping.
 * Production sends verification email via Gmail SMTP (app-password env vars);
 * lab falls back to a no-op / log-safe path (no token echo in production).
 */
import crypto from "node:crypto";

export function hashToken(raw) {
  return crypto.createHash("sha256").update(String(raw)).digest("hex");
}

/**
 * Validate bundle-key material (env-configured, file-loaded, or legacy
 * migration) before it is cached. P1-3: `createPrivateKey` parses PEMs of
 * many types, so an RSA/EC key would otherwise be accepted and silently
 * weaken bundle signatures — require the material to be genuinely Ed25519
 * with a 32-byte wrap key.
 */
export function assertBundleKeyMaterial(material) {
  if (!material || typeof material !== "object") throw new Error("bundle_key_material_required");
  if (typeof material.ed25519_private_pem !== "string" || !material.ed25519_private_pem.includes("PRIVATE KEY")) {
    throw new Error("bundle_key_private_pem_required");
  }
  const priv = crypto.createPrivateKey(material.ed25519_private_pem);
  if (priv.asymmetricKeyType !== "ed25519") throw new Error("bundle_key_must_be_ed25519");
  if (Buffer.from(material.wrap_key_b64 || "", "base64").length !== 32) {
    throw new Error("invalid_bundle_wrap_key");
  }
  return material;
}

export function issueEmailVerifyToken() {
  const raw = crypto.randomBytes(24).toString("base64url");
  return { raw, hash: hashToken(raw), expires_at: Date.now() + 24 * 3600 * 1000 };
}

/**
 * Gmail SMTP configuration from environment.
 * Required for production email sending: GMAIL_USER, GMAIL_APP_PASSWORD.
 * Optional: GMAIL_FROM (defaults to GMAIL_USER), GMAIL_SMTP_HOST (smtp.gmail.com),
 *           GMAIL_SMTP_PORT (465).
 * Returns null when GMAIL_USER or GMAIL_APP_PASSWORD is absent (lab fallback).
 */
export function gmailConfigFromEnv(env = process.env) {
  const user = env.GMAIL_USER || "";
  const appPassword = env.GMAIL_APP_PASSWORD || "";
  if (!user || !appPassword) return null;
  return {
    user,
    pass: appPassword,
    from: env.GMAIL_FROM || user,
    host: env.GMAIL_SMTP_HOST || "smtp.gmail.com",
    port: Number(env.GMAIL_SMTP_PORT || 465),
    secure: Number(env.GMAIL_SMTP_PORT || 465) === 465,
    publicUrl: env.GV6_OFFICIAL_PUBLIC_URL || "http://127.0.0.1:4101",
    verifyPath: env.GV6_ACCOUNT_PATH || "/account",
    labEcho: env.GV6_OFFICIAL_LAB_EMAIL === "1",
    isProd: ["prod", "production", "live"].includes(
      String(env.GV6_DEPLOY_ENV || env.NODE_ENV || "lab").toLowerCase()
    ),
  };
}

/**
 * Send the email verification message via Gmail SMTP.
 * - Production (GMAIL_USER + GMAIL_APP_PASSWORD set): sends a real email with a
 *   verification link. The raw token is NEVER echoed in production responses.
 *   (handled by the caller); it only appears inside the email body link.
 * - Lab fallback (no Gmail creds): no-op / log-safe. Returns { ok: true, sent: false,
 *   fallback: "lab_noop" } so the caller can decide to echo a lab_token.
 *
 * A `transporter` may be injected for unit testing (nodemailer-style).
 */
export async function sendVerificationEmail({ to, rawToken, env = process.env, transporter = null, publicUrl = null, pathSuffix = "", subject = "Verify your GreenV6 account email" }) {
  const cfg = gmailConfigFromEnv(env);
  // Lab fallback: no Gmail creds → no-op, never throw.
  if (!cfg) {
    return { ok: true, sent: false, fallback: "lab_noop" };
  }
  const base = publicUrl || cfg.publicUrl;
  const verifyUrl = `${base.replace(/\/$/, "")}${cfg.verifyPath}?${pathSuffix || "verify"}=${encodeURIComponent(rawToken)}`;
  const text = `Open this GreenV6 account link:\n\n${verifyUrl}\n\nThis link expires in 24 hours. If you did not request this, ignore this email.`;
  const html = `<p>Open this GreenV6 account link:</p><p><a href="${verifyUrl}">${subject}</a></p><p style="color:#888;font-size:12px">This link expires in 24 hours. If you did not request this, ignore this email.</p>`;
  const mailOptions = {
    from: cfg.from,
    to,
    subject,
    text,
    html,
  };
  if (transporter) {
    await transporter.sendMail(mailOptions);
    return { ok: true, sent: true, fallback: null };
  }
  // Lazy dynamic import keeps lab (no nodemailer) working.
  const nodemailer = await import("nodemailer");
  const tr = nodemailer.createTransport({
    host: cfg.host,
    port: cfg.port,
    secure: cfg.secure,
    auth: { user: cfg.user, pass: cfg.pass },
  });
  try {
    await tr.sendMail(mailOptions);
    return { ok: true, sent: true, fallback: null };
  } finally {
    tr.close?.();
  }
}

/** Generate TOTP secret (base32) — verify with standard authenticator apps. */
export function generateTotpSecret() {
  // 20 bytes → base32
  const buf = crypto.randomBytes(20);
  const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
  let bits = 0;
  let value = 0;
  let out = "";
  for (const b of buf) {
    value = (value << 8) | b;
    bits += 8;
    while (bits >= 5) {
      out += alphabet[(value >>> (bits - 5)) & 31];
      bits -= 5;
    }
  }
  if (bits > 0) out += alphabet[(value << (5 - bits)) & 31];
  return out;
}

export function totpCode(secretBase32, step = 30, digits = 6, atMs = Date.now()) {
  const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
  let bits = "";
  for (const c of secretBase32.replace(/=+$/, "").toUpperCase()) {
    const v = alphabet.indexOf(c);
    if (v < 0) continue;
    bits += v.toString(2).padStart(5, "0");
  }
  const bytes = [];
  for (let i = 0; i + 8 <= bits.length; i += 8) {
    bytes.push(parseInt(bits.slice(i, i + 8), 2));
  }
  const key = Buffer.from(bytes);
  const counter = Math.floor(atMs / 1000 / step);
  const msg = Buffer.alloc(8);
  msg.writeBigUInt64BE(BigInt(counter));
  const hmac = crypto.createHmac("sha1", key).update(msg).digest();
  const offset = hmac[hmac.length - 1] & 0xf;
  const code =
    ((hmac[offset] & 0x7f) << 24) |
    ((hmac[offset + 1] & 0xff) << 16) |
    ((hmac[offset + 2] & 0xff) << 8) |
    (hmac[offset + 3] & 0xff);
  return String(code % 10 ** digits).padStart(digits, "0");
}

export function verifyTotp(secretBase32, code, window = 1) {
  const want = String(code || "").trim();
  if (!/^\d{6}$/.test(want)) return false;
  const now = Date.now();
  for (let w = -window; w <= window; w++) {
    if (totpCode(secretBase32, 30, 6, now + w * 30_000) === want) return true;
  }
  return false;
}

export function ticketCategories() {
  return ["billing", "domain", "algo", "security", "other"];
}
