import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { StripeBillingAdapter, MockBillingAdapter, stripeConfigFromEnv } from "./billing.js";
import { gmailConfigFromEnv } from "./account_security.js";
import { PERMS, hasPerm } from "./staff.js";

const SETTINGS_KEY = "integrations";
const SECRET_PREFIX = "enc:v1:";

function keyBytes(material) {
  if (!material) return null;
  const value = String(material).trim();
  if (/^[0-9a-f]{64}$/i.test(value)) return Buffer.from(value, "hex");
  return crypto.createHash("sha256").update(value).digest();
}

export function ensureSettingsKey(dataDir, env = process.env) {
  const fromEnv = keyBytes(env.GV6_OFFICIAL_SETTINGS_KEY);
  if (fromEnv) return fromEnv;
  const file = path.join(dataDir, "integration_settings.key");
  try {
    const existing = fs.readFileSync(file);
    if (existing.length === 32) return existing;
  } catch {
    // Create below.
  }
  const created = crypto.randomBytes(32);
  fs.mkdirSync(dataDir, { recursive: true });
  fs.writeFileSync(file, created, { mode: 0o600 });
  try { fs.chmodSync(file, 0o600); } catch {}
  return created;
}

export function encryptSecret(value, key) {
  if (!value) return "";
  const iv = crypto.randomBytes(12);
  const cipher = crypto.createCipheriv("aes-256-gcm", key, iv);
  const ciphertext = Buffer.concat([cipher.update(String(value), "utf8"), cipher.final()]);
  return `${SECRET_PREFIX}${iv.toString("base64url")}.${ciphertext.toString("base64url")}.${cipher.getAuthTag().toString("base64url")}`;
}

export function decryptSecret(value, key) {
  if (!value) return "";
  if (!String(value).startsWith(SECRET_PREFIX)) return String(value);
  const parts = String(value).slice(SECRET_PREFIX.length).split(".");
  if (parts.length !== 3) throw new Error("invalid_encrypted_setting");
  const decipher = crypto.createDecipheriv("aes-256-gcm", key, Buffer.from(parts[0], "base64url"));
  decipher.setAuthTag(Buffer.from(parts[2], "base64url"));
  return Buffer.concat([
    decipher.update(Buffer.from(parts[1], "base64url")),
    decipher.final(),
  ]).toString("utf8");
}

function masked(value) {
  const s = String(value || "");
  return s ? `${s.slice(0, 4)}…${s.slice(-3)}` : "";
}

function envDefaults(env) {
  const stripe = stripeConfigFromEnv(env);
  const gmail = gmailConfigFromEnv(env);
  return {
    stripe: stripe ? {
      secret_key: stripe.secretKey,
      webhook_secret: stripe.webhookSecret,
      price_id: stripe.priceId,
      success_url: stripe.successUrl,
      cancel_url: stripe.cancelUrl,
      annual_usd: stripe.annualUsd,
      period_ms: stripe.periodMs,
      grace_ms: stripe.graceMs,
    } : {},
    gmail: gmail ? {
      user: gmail.user,
      app_password: gmail.pass,
      from: gmail.from,
      host: gmail.host,
      port: gmail.port,
    } : {},
  };
}

function decodeStored(value, key) {
  if (!value) return null;
  const parsed = typeof value === "string" ? JSON.parse(value) : value;
  if (parsed?.stripe) {
    parsed.stripe.secret_key = decryptSecret(parsed.stripe.secret_key, key);
    parsed.stripe.webhook_secret = decryptSecret(parsed.stripe.webhook_secret, key);
  }
  if (parsed?.gmail) parsed.gmail.app_password = decryptSecret(parsed.gmail.app_password, key);
  return parsed;
}

function encodeStored(config, key) {
  const copy = JSON.parse(JSON.stringify(config));
  if (copy.stripe) {
    copy.stripe.secret_key = encryptSecret(copy.stripe.secret_key, key);
    copy.stripe.webhook_secret = encryptSecret(copy.stripe.webhook_secret, key);
  }
  if (copy.gmail) copy.gmail.app_password = encryptSecret(copy.gmail.app_password, key);
  return copy;
}

export async function loadIntegrationSettings(db, key, env = process.env) {
  const row = await db.get("SELECT value_json FROM admin_settings WHERE setting_key=?", SETTINGS_KEY);
  const stored = decodeStored(row?.value_json, key);
  const defaults = envDefaults(env);
  const merged = {
    stripe: { ...defaults.stripe, ...(stored?.stripe || {}) },
    gmail: { ...defaults.gmail, ...(stored?.gmail || {}) },
  };
  return {
    config: merged,
    adapter: merged.stripe.secret_key
      ? new StripeBillingAdapter({
        secretKey: merged.stripe.secret_key,
        webhookSecret: merged.stripe.webhook_secret,
        priceId: merged.stripe.price_id,
        successUrl: merged.stripe.success_url,
        cancelUrl: merged.stripe.cancel_url,
        annualUsd: merged.stripe.annual_usd,
        periodMs: merged.stripe.period_ms,
        graceMs: merged.stripe.grace_ms,
      })
      : new MockBillingAdapter(),
    gmail: merged.gmail.user && merged.gmail.app_password
      ? {
        user: merged.gmail.user,
        pass: merged.gmail.app_password,
        from: merged.gmail.from || merged.gmail.user,
        host: merged.gmail.host || "smtp.gmail.com",
        port: Number(merged.gmail.port || 465),
        secure: Number(merged.gmail.port || 465) === 465,
        publicUrl: env.GV6_OFFICIAL_PUBLIC_URL || "http://127.0.0.1:4101",
        verifyPath: env.GV6_ACCOUNT_PATH || "/account",
        labEcho: env.GV6_OFFICIAL_LAB_EMAIL === "1",
        isProd: ["prod", "production", "live"].includes(String(env.GV6_DEPLOY_ENV || env.NODE_ENV || "lab").toLowerCase()),
      }
      : null,
  };
}

export function publicIntegrationSettings(config) {
  return {
    stripe: {
      configured: !!config?.stripe?.secret_key,
      secret_key: masked(config?.stripe?.secret_key),
      webhook_configured: !!config?.stripe?.webhook_secret,
      price_id: config?.stripe?.price_id || "",
      success_url: config?.stripe?.success_url || "",
      cancel_url: config?.stripe?.cancel_url || "",
      annual_usd: Number(config?.stripe?.annual_usd || 99),
    },
    gmail: {
      configured: !!config?.gmail?.user && !!config?.gmail?.app_password,
      user: config?.gmail?.user || "",
      app_password: masked(config?.gmail?.app_password),
      from: config?.gmail?.from || "",
      host: config?.gmail?.host || "smtp.gmail.com",
      port: Number(config?.gmail?.port || 465),
    },
  };
}

export function mergeIntegrationPatch(current, patch) {
  const next = {
    stripe: { ...(current?.stripe || {}) },
    gmail: { ...(current?.gmail || {}) },
  };
  for (const section of ["stripe", "gmail"]) {
    if (!patch?.[section]) continue;
    for (const [field, value] of Object.entries(patch[section])) {
      if (typeof value === "string" && value === "" && ["secret_key", "webhook_secret", "app_password"].includes(field)) continue;
      next[section][field] = value;
    }
  }
  if (patch?.stripe?.secret_key === null) next.stripe.secret_key = "";
  if (patch?.stripe?.webhook_secret === null) next.stripe.webhook_secret = "";
  if (patch?.gmail?.app_password === null) next.gmail.app_password = "";
  return next;
}

export async function saveIntegrationSettings(db, key, config, updatedBy, now) {
  const encoded = JSON.stringify(encodeStored(config, key));
  await db.run(
    `INSERT INTO admin_settings (setting_key, value_json, updated_at, updated_by, revision)
     VALUES (?, ?, ?, ?, 1)
     ON CONFLICT(setting_key) DO UPDATE SET value_json=EXCLUDED.value_json,
       updated_at=EXCLUDED.updated_at, updated_by=EXCLUDED.updated_by,
       revision=admin_settings.revision+1`,
    SETTINGS_KEY,
    encoded,
    now,
    updatedBy
  );
  return { ...config };
}

export function registerIntegrationRoutes(app, {
  db,
  key,
  requireUserRow,
  isCmsStaff,
  now,
  getConfig,
  applyConfig,
}) {
  async function requireIntegrationAdmin(req, reply) {
    const user = await requireUserRow(req, reply);
    if (!user) return null;
    if (!isCmsStaff(user) || !hasPerm(user, PERMS.CMS_EDIT)) {
      reply.code(403).send({ ok: false, error: "integration_admin_forbidden" });
      return null;
    }
    return user;
  }

  app.get("/v1/admin/integrations", async (req, reply) => {
    const user = await requireIntegrationAdmin(req, reply);
    if (!user) return;
    const row = await db.get(
      "SELECT revision, updated_at, updated_by FROM admin_settings WHERE setting_key=?",
      SETTINGS_KEY
    );
    return {
      ok: true,
      revision: Number(row?.revision || 0),
      updated_at: row?.updated_at || null,
      updated_by: row?.updated_by || null,
      settings: publicIntegrationSettings(getConfig()),
    };
  });

  app.put("/v1/admin/integrations", async (req, reply) => {
    const user = await requireIntegrationAdmin(req, reply);
    if (!user) return;
    const patch = req.body || {};
    const current = getConfig();
    const next = mergeIntegrationPatch(current, patch);
    const stripe = next.stripe || {};
    const gmail = next.gmail || {};
    if (stripe.price_id && !/^price_[A-Za-z0-9_]+$/.test(String(stripe.price_id))) {
      return reply.code(400).send({ ok: false, error: "invalid_stripe_price_id" });
    }
    if (stripe.success_url && !/^https:\/\//i.test(String(stripe.success_url))) {
      return reply.code(400).send({ ok: false, error: "stripe_success_url_must_be_https" });
    }
    if (stripe.cancel_url && !/^https:\/\//i.test(String(stripe.cancel_url))) {
      return reply.code(400).send({ ok: false, error: "stripe_cancel_url_must_be_https" });
    }
    if (gmail.port != null && (!Number.isInteger(Number(gmail.port)) || Number(gmail.port) < 1 || Number(gmail.port) > 65535)) {
      return reply.code(400).send({ ok: false, error: "invalid_gmail_port" });
    }
    if (IS_PROD_FOR_CONFIG() && (!stripe.secret_key || !stripe.webhook_secret || !stripe.price_id)) {
      return reply.code(400).send({ ok: false, error: "stripe_configuration_incomplete" });
    }
    const saved = await saveIntegrationSettings(db, key, next, user.email || user.id, now());
    await applyConfig(saved);
    return { ok: true, settings: publicIntegrationSettings(saved) };
  });
}

function IS_PROD_FOR_CONFIG() {
  return ["prod", "production", "live"].includes(
    String((process.env.GR_DEPLOY_ENV ?? process.env.GV6_DEPLOY_ENV) || process.env.NODE_ENV || "lab").toLowerCase()
  );
}
