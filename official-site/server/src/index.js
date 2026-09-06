/**
 * Official site API — users, per-site $99 plans, OAuth2+PKCE, plan entitlements.
 * Product: visitor / request analysis (no intercept). Free vs paid = device lanes + RPA.
 */
import Fastify from "fastify";
import cors from "@fastify/cors";
import cookie from "@fastify/cookie";
import formbody from "@fastify/formbody";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import crypto from "node:crypto";
import { openDb, migrate, defaultDatabaseUrl } from "./db.js";
import { hashPassword, verifyPassword, randomToken, sha256b64url, b64url } from "./crypto_util.js";
import { STRATEGY_CATALOG, buildPlanPayload, planEntitlement } from "./strategies.js";
import {
  defaultBillingAdapter,
  MockBillingAdapter,
  selectBillingAdapter,
  stripeConfigFromEnv,
  StripeBillingAdapter,
  planActive,
  billingPublicView,
  trialMsFromEnv,
} from "./billing.js";
import {
  generateNodeKeypair,
  ecdhAesKey,
  encryptWithKey,
  buildAlgoBundleHeader,
} from "./algo_delivery.js";
import { makeVerifyToken, verifyDomain, dnsFqdn } from "./domain_verify.js";
import {
  issueEmailVerifyToken,
  hashToken,
  generateTotpSecret,
  verifyTotp,
  ticketCategories,
  sendVerificationEmail,
  gmailConfigFromEnv,
  assertBundleKeyMaterial,
} from "./account_security.js";
import { migrateContent, seedContentIfEmpty, patchPricingFaq, patchMarketingLayout, seedExtraContentPages, patchLocaleUrls } from "./content_cms.js";
import { registerContentRoutes } from "./content_routes.js";
import {
  isCmsStaff,
  orgOwnerId,
  loadUserRow,
  publicUserProfile,
  listOrgMembers,
  canManageMembers,
  hasPerm,
  PERMS,
  ROLES,
} from "./staff.js";
import { adminCmsRoutes, adminCmsSegment } from "./admin_path.js";
import { rateLimitAllow, clientIp, attachRateLimitDb } from "./rate_limit.js";
import {
  ensureSettingsKey,
  loadIntegrationSettings,
  registerIntegrationRoutes,
} from "./integration_settings.js";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(__dirname, "../..");
const DATA_DIR = (process.env.GR_OFFICIAL_DATA ?? process.env.GV6_OFFICIAL_DATA) || path.join(ROOT, "data");
fs.mkdirSync(DATA_DIR, { recursive: true });

const PORT = Number((process.env.GR_OFFICIAL_PORT ?? process.env.GV6_OFFICIAL_PORT) || 4101);
const PUBLIC_URL = (process.env.GR_OFFICIAL_PUBLIC_URL ?? process.env.GV6_OFFICIAL_PUBLIC_URL) || `http://127.0.0.1:${PORT}`;
// Local default: console + API same origin (static web on this port).
const WEB_ORIGIN = (process.env.GR_OFFICIAL_WEB_ORIGIN ?? process.env.GV6_OFFICIAL_WEB_ORIGIN) || PUBLIC_URL;
const ADMIN_REDIRECTS = (
  (process.env.GR_ADMIN_OAUTH_REDIRECTS ?? process.env.GV6_ADMIN_OAUTH_REDIRECTS) ||
  "http://127.0.0.1:28680/oauth/callback,http://127.0.0.1:5173/oauth/callback"
)
  .split(",")
  .map((s) => s.trim())
  .filter(Boolean);
// Production default: wrap-key endpoint OFF. Lab must set GR_OFFICIAL_EXPOSE_WRAP_KEY=1 (legacy GV6_) explicitly.
// Prefer POST /v1/runtime/algo-bundle (ECDH v2).
const EXPOSE_WRAP_KEY = (process.env.GR_OFFICIAL_EXPOSE_WRAP_KEY ?? process.env.GV6_OFFICIAL_EXPOSE_WRAP_KEY) === "1";
const REQUIRE_DOMAIN_VERIFY =
  (process.env.GR_REQUIRE_DOMAIN_VERIFY ?? process.env.GV6_REQUIRE_DOMAIN_VERIFY) === "1" ||
  (process.env.GR_REQUIRE_DOMAIN_VERIFY ?? process.env.GV6_REQUIRE_DOMAIN_VERIFY) === "true";
const DEPLOY_ENV = ((process.env.GR_DEPLOY_ENV ?? process.env.GV6_DEPLOY_ENV) || process.env.NODE_ENV || "lab").toLowerCase();
const IS_PROD = ["prod", "production", "live"].includes(DEPLOY_ENV);
/** Public register/login: off unless GR_PUBLIC_AUTH=1 (legacy GV6_) (website holds accounts until launch). */
const PUBLIC_AUTH = (process.env.GR_PUBLIC_AUTH ?? process.env.GV6_PUBLIC_AUTH) === "1";

// Stripe billing (production). Null in lab → MockBillingAdapter.
const STRIPE_CONFIG = stripeConfigFromEnv();
const STRIPE_ENABLED = !!STRIPE_CONFIG;
// Gmail SMTP for email verification. Null in lab → no-op fallback.
const GMAIL_CONFIG = gmailConfigFromEnv();
const SETTINGS_KEY = ensureSettingsKey(DATA_DIR);
let activeBillingAdapter = defaultBillingAdapter;
let activeGmailConfig = GMAIL_CONFIG;
let activeIntegrationConfig = { stripe: {}, gmail: {} };

const OAUTH_CLIENT_ID = (process.env.GR_OAUTH_CLIENT_ID ?? process.env.GV6_OAUTH_CLIENT_ID) || "gr-admin-panel";
const SESSION_COOKIE = "gr_official_session";
const SESSION_TTL_MS = 12 * 3600 * 1000;
// P2-05 (recent-auth): a full login stays "recent" for this window. Within
// it, sensitive operations on a session without TOTP skip re-prompting;
// outside it (or always when TOTP is enabled) a factor must be re-verified.
const RECENT_AUTH_WINDOW_MS = Number((process.env.GR_RECENT_AUTH_WINDOW_MS ?? process.env.GV6_RECENT_AUTH_WINDOW_MS) || 10 * 60 * 1000);
const CODE_TTL_MS = 5 * 60 * 1000;
const ACCESS_TTL_SEC = 3600;
const BUNDLE_TTL_SEC = 15 * 60;

let db;
let cachedKeys = null;
let cachedLicenseKey = null;

const COOKIE_SECURE = IS_PROD || (process.env.GR_COOKIE_SECURE ?? process.env.GV6_COOKIE_SECURE) === "1";

function sessionCookieOpts() {
  return {
    path: "/",
    httpOnly: true,
    sameSite: "lax",
    secure: COOKIE_SECURE,
    maxAge: SESSION_TTL_MS / 1000,
  };
}

function corsAllowedOrigin(origin) {
  if (!origin) return true;
  const extra = String((process.env.GR_CORS_ORIGINS ?? process.env.GV6_CORS_ORIGINS) || "")
    .split(",")
    .map((s) => s.trim().replace(/\/$/, ""))
    .filter(Boolean);
  const allow = new Set(
    [WEB_ORIGIN, PUBLIC_URL, "http://127.0.0.1:28680", "http://127.0.0.1:5173", "http://127.0.0.1:3000", ...extra]
      .map((s) => String(s || "").replace(/\/$/, ""))
      .filter(Boolean)
  );
  return allow.has(String(origin).replace(/\/$/, ""));
}

function csrfAllowedOrigin(origin) {
  const normalized = String(origin || "").replace(/\/$/, "");
  const extra = String((process.env.GR_CORS_ORIGINS ?? process.env.GV6_CORS_ORIGINS) || "")
    .split(",")
    .map((s) => s.trim().replace(/\/$/, ""))
    .filter(Boolean);
  return [WEB_ORIGIN, PUBLIC_URL, ...extra]
    .map((s) => String(s || "").replace(/\/$/, ""))
    .includes(normalized);
}

const app = Fastify({ logger: true });
await app.register(cors, {
  origin: (origin, cb) => cb(null, corsAllowedOrigin(origin)),
  credentials: true,
});
await app.register(cookie);
await app.register(formbody);

// Capture raw JSON body for Stripe webhook signature verification.
// Other JSON routes still receive parsed bodies; this stores the raw Buffer
// on req.rawBody so the webhook handler can verify the Stripe signature.
app.addContentTypeParser(
  "application/json",
  { parseAs: "buffer" },
  (req, body, done) => {
    req.rawBody = body;
    try {
      req.body = JSON.parse(body.toString("utf8"));
      done(null, req.body);
    } catch (e) {
      done(e, undefined);
    }
  }
);

async function authRateLimited(req, reply, bucket, limit = 20) {
  const cap = IS_PROD ? limit : Math.max(limit, 80);
  const key = `${bucket}:${clientIp(req)}`;
  if (!(await rateLimitAllow(key, { limit: cap, windowMs: 60_000 }))) {
    reply.code(429).send({ ok: false, error: "rate_limited" });
    return true;
  }
  return false;
}

function sendWeb(reply, name = "index.html") {
  const file = path.join(ROOT, "web", name);
  const body = fs.readFileSync(file);
  return reply.type("text/html; charset=utf-8").send(body);
}

function sendCms(reply) {
  const file = path.join(ROOT, "web", "cms.html");
  let body = fs.readFileSync(file, "utf8");
  return reply.type("text/html; charset=utf-8").send(body);
}

async function requireUserRow(req, reply) {
  const sess = await requireUser(req, reply);
  if (!sess) return null;
  return await loadUserRow(db, sess.user_id);
}

async function requirePerm(req, reply, perm) {
  const u = await requireUserRow(req, reply);
  if (!u) return null;
  if (!hasPerm(u, perm)) {
    reply.code(403).send({ ok: false, error: "forbidden", required: perm });
    return null;
  }
  return u;
}

async function requireSensitive(req, reply, perm) {
  const sess = await requireUser(req, reply);
  if (!sess) return null;
  const u = await loadUserRow(db, sess.user_id);
  if (!u) return null;
  if (!hasPerm(u, perm)) {
    reply.code(403).send({ ok: false, error: "forbidden", required: perm });
    return null;
  }
  if (IS_PROD && (process.env.GR_OFFICIAL_LAB_EMAIL ?? process.env.GV6_OFFICIAL_LAB_EMAIL) !== "1") {
    if (!u.email_verified) {
      reply.code(403).send({ ok: false, error: "email_unverified" });
      return null;
    }
  }
  if (!(await verifySensitiveFactor(req, u, sess))) {
    reply.code(401).send({ ok: false, error: "factor_required" });
    return null;
  }
  return u;
}

function accountLoginUrl(nextPath) {
  const base = (process.env.GR_ACCOUNT_PATH ?? process.env.GV6_ACCOUNT_PATH) || "/account";
  const origin = WEB_ORIGIN.replace(/\/$/, "");
  if (!nextPath) return `${origin}${base}`;
  return `${origin}${base}?next=${encodeURIComponent(nextPath)}`;
}

function isUnifiedWeb() {
  return WEB_ORIGIN.replace(/\/$/, "") !== PUBLIC_URL.replace(/\/$/, "");
}

app.get("/", async (req, reply) => {
  if (isUnifiedWeb()) {
    return reply.redirect(`${WEB_ORIGIN.replace(/\/$/, "")}/`);
  }
  return sendWeb(reply);
});

app.get("/login", async (req, reply) => {
  if (isUnifiedWeb()) {
    const next = req.query?.next ? String(req.query.next) : "";
    return reply.redirect(accountLoginUrl(next || undefined));
  }
  return sendWeb(reply);
});

app.get("/index.html", async (req, reply) => {
  if (isUnifiedWeb()) {
    return reply.redirect(`${WEB_ORIGIN.replace(/\/$/, "")}/account`);
  }
  return sendWeb(reply);
});

app.get("/docs/deploy", async (_req, reply) => sendWeb(reply, "docs-deploy.html"));

for (const route of adminCmsRoutes()) {
  app.get(route, async (_req, reply) => sendCms(reply));
}

function now() {
  return Date.now();
}

async function maybeDowngradeSite(site) {
  if (!site) return site;
  const rec = await activeBillingAdapter.reconcile(db, site, now());
  if (rec.downgraded) {
    return { ...site, plan: "free", billing_status: "free" };
  }
  return site;
}

async function removeLegacyCryptoTable(material) {
  const table = await db.get("SELECT to_regclass('public.crypto_keys') AS name");
  if (!table?.name) return;
  const legacy = await db.get("SELECT * FROM crypto_keys WHERE id=1");
  if (legacy) {
    const same =
      legacy.ed25519_private_pem === material.ed25519_private_pem &&
      legacy.wrap_key_b64 === material.wrap_key_b64;
    if (!same) throw new Error("legacy_bundle_keys_conflict");
  }
  await db.exec("DROP TABLE crypto_keys");
}

async function ensureKeys() {
  // Bundle keys must not remain in PostgreSQL, where routine application and
  // backup access would expose signing and encryption material together.
  const configuredPrivate = (process.env.GR_BUNDLE_SIGNING_KEY_PEM ?? process.env.GV6_BUNDLE_SIGNING_KEY_PEM);
  const configuredWrap = (process.env.GR_BUNDLE_WRAP_KEY_B64 ?? process.env.GV6_BUNDLE_WRAP_KEY_B64);
  if (configuredPrivate || configuredWrap) {
    if (!configuredPrivate || !configuredWrap) throw new Error("bundle_key_configuration_incomplete");
    const privateKey = crypto.createPrivateKey(configuredPrivate);
    if (privateKey.asymmetricKeyType !== "ed25519" || Buffer.from(configuredWrap, "base64").length !== 32) {
      throw new Error("bundle_key_configuration_invalid");
    }
    cachedKeys = {
      ed25519_private_pem: configuredPrivate,
      ed25519_public_pem: crypto.createPublicKey(privateKey).export({ type: "spki", format: "pem" }),
      wrap_key_b64: configuredWrap,
    };
    await removeLegacyCryptoTable(cachedKeys);
    return cachedKeys;
  }

  const file = (process.env.GR_BUNDLE_KEY_FILE ?? process.env.GV6_BUNDLE_KEY_FILE) || path.join(DATA_DIR, "bundle_keys.json");
  let fileMaterial = null;
  try {
    const parsed = JSON.parse(fs.readFileSync(file, "utf8"));
    // P1-3: file-loaded key must be Ed25519 with a 32-byte wrap key.
    assertBundleKeyMaterial(parsed);
    fileMaterial = parsed;
  } catch {}
  if (fileMaterial) {
    cachedKeys = fileMaterial;
    await removeLegacyCryptoTable(cachedKeys);
    return cachedKeys;
  }

  const legacyTable = await db.get("SELECT to_regclass('public.crypto_keys') AS name");
  const legacy = legacyTable?.name ? await db.get("SELECT * FROM crypto_keys WHERE id=1") : null;
  if (legacy) {
    // One-time, atomic local migration then erase database-held secret data.
    const material = {
      ed25519_public_pem: legacy.ed25519_public_pem,
      ed25519_private_pem: legacy.ed25519_private_pem,
      wrap_key_b64: legacy.wrap_key_b64,
    };
    const tmp = `${file}.${process.pid}.tmp`;
    fs.writeFileSync(tmp, JSON.stringify(material), { mode: 0o600 });
    fs.renameSync(tmp, file);
    try { fs.chmodSync(file, 0o600); } catch {}
    await db.exec("DROP TABLE crypto_keys");
    cachedKeys = material;
    return material;
  }

  if (IS_PROD) throw new Error("production_requires_bundle_key_secret");
  const { publicKey, privateKey } = crypto.generateKeyPairSync("ed25519");
  const material = {
    ed25519_public_pem: publicKey.export({ type: "spki", format: "pem" }),
    ed25519_private_pem: privateKey.export({ type: "pkcs8", format: "pem" }),
    wrap_key_b64: crypto.randomBytes(32).toString("base64"),
  };
  fs.writeFileSync(file, JSON.stringify(material), { mode: 0o600 });
  try { fs.chmodSync(file, 0o600); } catch {}
  cachedKeys = material;
  return material;
}

async function ensureLicenseKey() {
  const configured = (process.env.GR_LICENSE_SIGNING_KEY_PEM ?? process.env.GV6_LICENSE_SIGNING_KEY_PEM);
  if (configured) {
    const privateKey = crypto.createPrivateKey(configured);
    if (privateKey.asymmetricKeyType !== "ed25519") {
      throw new Error("license_signing_key_must_be_ed25519");
    }
    cachedLicenseKey = privateKey;
    return privateKey;
  }
  if (IS_PROD) {
    throw new Error("production_requires_license_signing_key");
  }
  // Lab-only fallback: keep a separate key on the service filesystem, never
  // in crypto_keys, so bundle compromise does not grant license issuance.
  const file = path.join(DATA_DIR, "license_ed25519.private.pem");
  try {
    const priv = crypto.createPrivateKey(fs.readFileSync(file, "utf8"));
    if (priv.asymmetricKeyType !== "ed25519") {
      throw new Error("license_signing_key_must_be_ed25519");
    }
    cachedLicenseKey = priv;
  } catch {
    const { privateKey } = crypto.generateKeyPairSync("ed25519");
    fs.writeFileSync(file, privateKey.export({ type: "pkcs8", format: "pem" }), { mode: 0o600 });
    try { fs.chmodSync(file, 0o600); } catch {}
    cachedLicenseKey = privateKey;
  }
  return cachedLicenseKey;
}

async function ensureOAuthClient() {
  const c = await db.get("SELECT client_id FROM oauth_clients WHERE client_id=?", OAUTH_CLIENT_ID);
  if (!c) {
    await db.run(
      `INSERT INTO oauth_clients (client_id, name, redirect_uris_json, created_at)
       VALUES (?, ?, ?, ?)`,
      OAUTH_CLIENT_ID,
      "GR Admin Panel",
      JSON.stringify(ADMIN_REDIRECTS),
      now()
    );
  }
}

async function seedStrategiesIfEmpty() {
  const row = await db.get("SELECT COUNT(*)::int AS n FROM strategy_versions");
  if ((row?.n ?? 0) > 0) return;
  for (const s of STRATEGY_CATALOG) {
    await db.run(
      `INSERT INTO strategy_versions (version_id, title, plan_min, body_json, created_at)
       VALUES (?, ?, ?, ?, ?)`,
      s.version_id,
      s.title,
      s.plan_min,
      JSON.stringify(s.body),
      now()
    );
  }
}

function getKeys() {
  if (!cachedKeys) throw new Error("crypto keys not loaded");
  return cachedKeys;
}

function signBytes(bytes) {
  const k = getKeys();
  const priv = crypto.createPrivateKey(k.ed25519_private_pem);
  return crypto.sign(null, bytes, priv);
}

function signLicenseBytes(bytes) {
  if (!cachedLicenseKey) throw new Error("license_signing_key_not_loaded");
  return crypto.sign(null, bytes, cachedLicenseKey);
}

// The bundle is the authenticated delivery channel for the short-lived
// license. The node verifies this token with the same public key exposed by
// /v1/jwks and caches only the raw token.
function issueLicenseToken({ site_id, domain, plan }) {
  const ent = planEntitlement(plan);
  const iat = now();
  const claims = {
    v: 1,
    license_id: `site-${site_id}-${randomToken(8)}`,
    site_id,
    domain,
    plan,
    rpa_enabled: ent.rpa_enabled,
    device_precisions: ent.device_precisions,
    quotas: {},
    lb: null,
    iat_ms: iat,
    exp_ms: iat + 24 * 3600 * 1000,
  };
  const payload = Buffer.from(JSON.stringify(claims), "utf8").toString("base64url");
  const msg = `gv6lic1.${payload}`;
  return `${msg}.${signLicenseBytes(Buffer.from(msg, "utf8")).toString("base64url")}`;
}

function wrapKey() {
  return Buffer.from(getKeys().wrap_key_b64, "base64");
}

function encryptBundle(plaintextObj) {
  const key = wrapKey();
  const iv = crypto.randomBytes(12);
  const cipher = crypto.createCipheriv("aes-256-gcm", key, iv);
  const pt = Buffer.from(JSON.stringify(plaintextObj), "utf8");
  const ct = Buffer.concat([cipher.update(pt), cipher.final()]);
  const tag = cipher.getAuthTag();
  const ciphertext = Buffer.concat([ct, tag]);
  return { iv, ciphertext };
}

async function sessionUser(req) {
  const sid = req.cookies?.[SESSION_COOKIE];
  if (!sid) return null;
  const row = await db.get(
    "SELECT s.*, u.email, u.id AS user_id FROM sessions s JOIN users u ON u.id=s.user_id WHERE s.token=? AND s.expires_at>?",
    sid,
    now()
  );
  return row || null;
}

async function requireUser(req, reply) {
  const u = await sessionUser(req);
  if (!u) {
    reply.code(401).send({ ok: false, error: "unauthorized" });
    return null;
  }
  // Cookie-authenticated state changes must be same-origin. Bearer-token
  // clients never reach this helper and remain suitable for service APIs.
  if (IS_PROD && !["GET", "HEAD", "OPTIONS"].includes(req.method)) {
    const origin = String(req.headers.origin || "").replace(/\/$/, "");
    if (!origin || !csrfAllowedOrigin(origin)) {
      reply.code(403).send({ ok: false, error: "csrf_origin_required" });
      return null;
    }
  }
  return u;
}

async function bearerUser(req) {
  const h = req.headers.authorization || "";
  const m = /^Bearer\s+(.+)$/i.exec(h);
  if (!m) return null;
  const tok = m[1];
  const row = await db.get(
    `SELECT t.*, u.email, u.id AS user_id FROM access_tokens t
     JOIN users u ON u.id=t.user_id
     WHERE t.token=? AND t.expires_at>?`,
    tok,
    now()
  );
  return row || null;
}

async function requireBearer(req, reply) {
  const u = await bearerUser(req);
  if (!u) {
    reply.code(401).send({ ok: false, error: "unauthorized" });
    return null;
  }
  return u;
}

function profileCap(plan) {
  return planEntitlement(plan).profile_cap;
}

function allowedStrategies(_plan) {
  return [];
}

// ── health ──────────────────────────────────────────────────────────
app.get("/v1/health", async () => ({
  ok: true,
  service: "gr-official-site",
  public_url: PUBLIC_URL,
  web_origin: WEB_ORIGIN,
  unified_web: isUnifiedWeb(),
}));

app.get("/v1/public-config", async () => ({
  ok: true,
  config: {
    require_domain_verify: REQUIRE_DOMAIN_VERIFY,
    panel_oauth_client_id: OAUTH_CLIENT_ID,
    panel_oauth_redirect_uri: ADMIN_REDIRECTS[0] || "",
    web_origin: WEB_ORIGIN,
    unified_web: isUnifiedWeb(),
    public_auth: PUBLIC_AUTH,
  },
}));

app.get("/v1/jwks", async () => {
  const k = getKeys();
  const pub = crypto.createPublicKey(k.ed25519_public_pem);
  const spki = pub.export({ type: "spki", format: "der" });
  // raw 32-byte for ed25519 from SPKI (last 32 bytes)
  const x = b64url(spki.subarray(spki.length - 32));
  return {
    keys: [{ kty: "OKP", crv: "Ed25519", x, kid: "official-1", use: "sig", alg: "EdDSA" }],
  };
});

app.get("/v1/license-jwks", async () => {
  if (!cachedLicenseKey) throw new Error("license_signing_key_not_loaded");
  const pub = crypto.createPublicKey(cachedLicenseKey);
  const spki = pub.export({ type: "spki", format: "der" });
  const x = b64url(spki.subarray(spki.length - 32));
  return {
    keys: [{ kty: "OKP", crv: "Ed25519", x, kid: "license-1", use: "sig", alg: "EdDSA" }],
  };
});

app.get("/v1/crypto/wrap-key", async (req, reply) => {
  // Lab only. Production / unset flag → 404; use ECDH algo-bundle instead.
  if (!EXPOSE_WRAP_KEY || IS_PROD) {
    return reply.code(404).send({
      ok: false,
      error: "wrap_key_disabled",
      hint: "Use POST /v1/runtime/algo-bundle (ECDH). Lab: GR_OFFICIAL_EXPOSE_WRAP_KEY=1 and non-prod GR_DEPLOY_ENV (legacy GV6_ accepted).",
    });
  }
  const u = await requireBearer(req, reply);
  if (!u) return;
  return { ok: true, wrap_key_b64: getKeys().wrap_key_b64, kid: "wrap-1" };
});

// ── auth (website session) ──────────────────────────────────────────
app.post("/v1/auth/register", async (req, reply) => {
  if (!PUBLIC_AUTH) {
    return reply.code(403).send({ ok: false, error: "auth_disabled" });
  }
  if (await authRateLimited(req, reply, "register", 8)) return;
  const email = String(req.body?.email || "")
    .trim()
    .toLowerCase();
  const password = String(req.body?.password || "");
  if (!email.includes("@") || password.length < 10) {
    return reply.code(400).send({ ok: false, error: "invalid_email_or_password" });
  }
  const exists = await db.get("SELECT id FROM users WHERE email=?", email);
  if (exists) return reply.code(409).send({ ok: false, error: "email_taken" });
  const id = randomToken(16);
  await db.run(
    "INSERT INTO users (id, email, password_hash, created_at, account_role) VALUES (?, ?, ?, ?, 'owner')",
    id,
    email,
    hashPassword(password),
    now()
  );
  const token = randomToken(32);
  await db.run(
    "INSERT INTO sessions (token, user_id, expires_at, created_at, recent_auth_at) VALUES (?, ?, ?, ?, ?)",
    token,
    id,
    now() + SESSION_TTL_MS,
    now(),
    now()
  );
  reply.setCookie(SESSION_COOKIE, token, sessionCookieOpts());
  return { ok: true, user: { id, email } };
});

app.post("/v1/auth/login", async (req, reply) => {
  if (!PUBLIC_AUTH) {
    return reply.code(403).send({ ok: false, error: "auth_disabled" });
  }
  if (await authRateLimited(req, reply, "login", 12)) return;
  const email = String(req.body?.email || "")
    .trim()
    .toLowerCase();
  const password = String(req.body?.password || "");
  const u = await db.get("SELECT * FROM users WHERE email=?", email);
  if (!u || !verifyPassword(password, u.password_hash)) {
    return reply.code(401).send({ ok: false, error: "invalid_credentials" });
  }
  if (u.totp_enabled) {
    const code = String(req.body?.totp_code || req.body?.code || "");
    if (!verifyTotp(u.totp_secret, code)) {
      // P1-2: TOTP loss must not lock the account. Recovery codes are
      // single-use and consumed only on a successful match (atomic
      // conditional UPDATE), so replays and brute force reuse fail.
      const claimed = await db.run(
        "UPDATE totp_recovery_codes SET used_at=? WHERE user_id=? AND code_hash=? AND used_at IS NULL",
        now(),
        u.id,
        hashToken(code.replace(/\s|-/g, "").toUpperCase())
      );
      if (claimed.changes !== 1) {
        return reply.code(401).send({ ok: false, error: "totp_required" });
      }
    }
  }
  const token = randomToken(32);
  await db.run(
    "INSERT INTO sessions (token, user_id, expires_at, created_at, recent_auth_at) VALUES (?, ?, ?, ?, ?)",
    token,
    u.id,
    now() + SESSION_TTL_MS,
    now(),
    now()
  );
  reply.setCookie(SESSION_COOKIE, token, sessionCookieOpts());
  return { ok: true, user: { id: u.id, email: u.email } };
});

app.post("/v1/auth/logout", async (req, reply) => {
  if (!(await requireUser(req, reply))) return;
  const sid = req.cookies?.[SESSION_COOKIE];
  if (sid) await db.run("DELETE FROM sessions WHERE token=?", sid);
  reply.clearCookie(SESSION_COOKIE, { path: "/" });
  return { ok: true };
});

function recoveryCodes() {
  return Array.from({ length: 10 }, () => crypto.randomBytes(5).toString("hex").toUpperCase());
}

async function auditAccount(req, userId, action, detail = {}) {
  await db.run(
    "INSERT INTO account_security_events (user_id, action, ip, created_at, detail_json) VALUES (?, ?, ?, ?, ?)",
    userId || null, action, clientIp(req), now(), JSON.stringify(detail)
  );
}

function recentAuthFresh(sess) {
  const t = Number(sess?.recent_auth_at || 0);
  return t > 0 && now() - t < RECENT_AUTH_WINDOW_MS;
}

async function markRecentAuth(sessToken) {
  await db.run("UPDATE sessions SET recent_auth_at=? WHERE token=?", now(), sessToken);
}

/**
 * Step-up gate for sensitive operations (P2-05 recent-auth):
 * - TOTP users always present a second factor (hard step-up on every op).
 * - Users without TOTP re-verify their password only when the session's last
 *   full authentication is older than GR_RECENT_AUTH_WINDOW_MS (legacy GV6_ accepted); a fresh
 *   login stays "recent" so the session is not punished on every op.
 * A successful factor refreshes recent_auth_at on the session.
 */
async function verifySensitiveFactor(req, user, sess) {
  const code = String(req.body?.totp_code || req.body?.code || "");
  if (user.totp_enabled) {
    if (verifyTotp(user.totp_secret, code)) {
      await markRecentAuth(sess.token);
      return true;
    }
    if (!code) return false;
    const claimed = await db.run(
      "UPDATE totp_recovery_codes SET used_at=? WHERE user_id=? AND code_hash=? AND used_at IS NULL",
      now(), user.id, hashToken(code.replace(/\s|-/g, "").toUpperCase())
    );
    if (claimed.changes === 1) {
      await markRecentAuth(sess.token);
      return true;
    }
    return false;
  }
  if (recentAuthFresh(sess)) return true;
  const password = String(req.body?.password || "");
  if (!password || !verifyPassword(password, user.password_hash)) return false;
  await markRecentAuth(sess.token);
  return true;
}

app.post("/v1/account/password", async (req, reply) => {
  const sess = await requireUser(req, reply);
  if (!sess) return;
  if (await authRateLimited(req, reply, "password_change", 8)) return;
  const u = await loadUserRow(db, sess.user_id);
  const password = String(req.body?.new_password || "");
  if (!u || password.length < 10 || !(await verifySensitiveFactor(req, u, sess))) {
    return reply.code(400).send({ ok: false, error: "invalid_password_or_factor" });
  }
  await db.transaction(async (tx) => {
    await tx.run("UPDATE users SET password_hash=? WHERE id=?", hashPassword(password), u.id);
    await tx.run("DELETE FROM sessions WHERE user_id=? AND token<>?", u.id, sess.token);
  });
  await auditAccount(req, u.id, "password_changed");
  return { ok: true };
});

app.get("/v1/account/sessions", async (req, reply) => {
  const sess = await requireUser(req, reply);
  if (!sess) return;
  const rows = await db.all("SELECT token, created_at, expires_at FROM sessions WHERE user_id=? ORDER BY created_at DESC", sess.user_id);
  return { ok: true, sessions: rows.map((r) => ({ created_at: r.created_at, expires_at: r.expires_at, current: r.token === sess.token })) };
});

app.post("/v1/account/sessions/revoke-all", async (req, reply) => {
  const sess = await requireUser(req, reply);
  if (!sess) return;
  if (await authRateLimited(req, reply, "session_revoke", 8)) return;
  const u = await loadUserRow(db, sess.user_id);
  if (!u || !(await verifySensitiveFactor(req, u, sess))) return reply.code(401).send({ ok: false, error: "factor_required" });
  await db.run("DELETE FROM sessions WHERE user_id=? AND token<>?", u.id, sess.token);
  await auditAccount(req, u.id, "sessions_revoked");
  return { ok: true };
});

app.post("/v1/auth/password-reset/request", async (req, reply) => {
  if (await authRateLimited(req, reply, "password_reset_request", 5)) return;
  const email = String(req.body?.email || "").trim().toLowerCase();
  const u = await db.get("SELECT id, email FROM users WHERE email=?", email);
  // Always return the same result, preventing account enumeration.
  if (!u) return { ok: true };
  // P1-1: production must deliver the reset link by email (admin panel Gmail
  // settings included) — never fall back to echoing the raw token.
  if (IS_PROD && !activeGmailConfig) {
    return reply.code(503).send({ ok: false, error: "gmail_not_configured" });
  }
  const issued = issueEmailVerifyToken();
  await db.run("DELETE FROM password_reset_tokens WHERE user_id=?", u.id);
  await db.run("INSERT INTO password_reset_tokens (user_id, token_hash, expires_at) VALUES (?, ?, ?)", u.id, issued.hash, issued.expires_at);
  let delivery = null;
  if (activeGmailConfig || !IS_PROD) {
    // With DB-managed Gmail settings, send through them (P1-1 — the reset
    // link must reach the inbox, never the HTTP response). Lab without
    // creds falls through to the no-op path so GR_OFFICIAL_LAB_EMAIL=1
    // can still echo the token for local testing.
    const mailEnv = activeGmailConfig
      ? {
          ...process.env,
          GMAIL_USER: activeGmailConfig.user,
          GMAIL_APP_PASSWORD: activeGmailConfig.pass,
          GMAIL_FROM: activeGmailConfig.from,
          GMAIL_SMTP_HOST: activeGmailConfig.host,
          GMAIL_SMTP_PORT: String(activeGmailConfig.port),
        }
      : undefined;
    try {
      delivery = await sendVerificationEmail({
        to: u.email,
        rawToken: issued.raw,
        pathSuffix: "reset",
        subject: "Reset your GreenV6 password",
        publicUrl: PUBLIC_URL,
        ...(mailEnv ? { env: mailEnv } : {}),
      });
    } catch (e) {
      app.log.warn({ msg: "gmail_send_failed", err: String(e.message || e) });
    }
  }
  if (IS_PROD && !delivery?.sent) {
    return reply.code(503).send({ ok: false, error: "verification_email_unavailable" });
  }
  await auditAccount(req, u.id, "password_reset_requested", { delivered: !!delivery?.sent });
  // P1-1: the raw token is echoed only in explicit lab mode
  // (GR_OFFICIAL_LAB_EMAIL=1; legacy GV6_ accepted), never in production, and never after the
  // email was actually delivered.
  const labEcho =
    !IS_PROD &&
    (process.env.GR_OFFICIAL_LAB_EMAIL ?? process.env.GV6_OFFICIAL_LAB_EMAIL) === "1" &&
    delivery?.sent !== true;
  return {
    ok: true,
    ...(delivery?.sent ? { email_sent: true } : {}),
    ...(labEcho ? { lab_reset_token: issued.raw } : {}),
  };
});

app.post("/v1/auth/password-reset/confirm", async (req, reply) => {
  if (await authRateLimited(req, reply, "password_reset_confirm", 8)) return;
  const token = String(req.body?.token || "");
  const password = String(req.body?.new_password || "");
  if (token.length < 20 || password.length < 10) return reply.code(400).send({ ok: false, error: "invalid_or_expired" });
  let userId;
  try {
    userId = await db.transaction(async (tx) => {
    const row = await tx.get(
      "DELETE FROM password_reset_tokens WHERE token_hash=? AND expires_at>? RETURNING user_id",
      hashToken(token), now()
    );
    if (!row) throw new Error("invalid_or_expired");
    await tx.run("UPDATE users SET password_hash=? WHERE id=?", hashPassword(password), row.user_id);
    await tx.run("DELETE FROM password_reset_tokens WHERE user_id=?", row.user_id);
    await tx.run("DELETE FROM sessions WHERE user_id=?", row.user_id);
    return row.user_id;
    });
  } catch (e) {
    if (e.message === "invalid_or_expired") return reply.code(400).send({ ok: false, error: "invalid_or_expired" });
    throw e;
  }
  await auditAccount(req, userId, "password_reset_completed");
  return { ok: true };
});

app.get("/v1/me", async (req, reply) => {
  const u = (await sessionUser(req)) || (await bearerUser(req));
  if (!u) return reply.code(401).send({ ok: false, error: "unauthorized" });
  const profile = await publicUserProfile(db, u.user_id);
  return { ok: true, user: profile };
});

// ── org members (sub-accounts) ─────────────────────────────────────
app.get("/v1/account/members", async (req, reply) => {
  const u = await requireUserRow(req, reply);
  if (!u) return;
  if (!canManageMembers(u)) {
    return reply.code(403).send({ ok: false, error: "forbidden" });
  }
  const ownerId = orgOwnerId(u);
  return { ok: true, members: await listOrgMembers(db, ownerId), roles: ROLES };
});

app.post("/v1/account/members", async (req, reply) => {
  const u = await requireUserRow(req, reply);
  if (!u) return;
  if (!canManageMembers(u)) {
    return reply.code(403).send({ ok: false, error: "forbidden" });
  }
  if (IS_PROD && (process.env.GR_OFFICIAL_LAB_EMAIL ?? process.env.GV6_OFFICIAL_LAB_EMAIL) !== "1" && !u.email_verified) {
    return reply.code(403).send({ ok: false, error: "email_unverified" });
  }
  const email = String(req.body?.email || "")
    .trim()
    .toLowerCase();
  const password = String(req.body?.password || "");
  const role = String(req.body?.role || "viewer");
  if (!email.includes("@") || password.length < 10) {
    return reply.code(400).send({ ok: false, error: "invalid_email_or_password" });
  }
  if (!ROLES.includes(role) || role === "owner") {
    return reply.code(400).send({ ok: false, error: "bad_role", roles: ROLES.filter((r) => r !== "owner") });
  }
  const exists = await db.get("SELECT id FROM users WHERE email=?", email);
  if (exists) return reply.code(409).send({ ok: false, error: "email_taken" });
  const ownerId = orgOwnerId(u);
  const id = randomToken(16);
  await db.run(`INSERT INTO users (id, email, password_hash, created_at, parent_user_id, account_role)
     VALUES (?, ?, ?, ?, ?, ?)`, id, email, hashPassword(password), now(), ownerId, role);
  return { ok: true, member: { id, email, role } };
});

app.patch("/v1/account/members/:memberId", async (req, reply) => {
  const u = await requireUserRow(req, reply);
  if (!u) return;
  if (!canManageMembers(u)) {
    return reply.code(403).send({ ok: false, error: "forbidden" });
  }
  const ownerId = orgOwnerId(u);
  const member = await db.get("SELECT * FROM users WHERE id=? AND parent_user_id=?", req.params.memberId, ownerId);
  if (!member) return reply.code(404).send({ ok: false, error: "not_found" });
  const role = String(req.body?.role || "").trim();
  if (!ROLES.includes(role) || role === "owner") {
    return reply.code(400).send({ ok: false, error: "bad_role" });
  }
  await db.run("UPDATE users SET account_role=? WHERE id=?", role, member.id);
  return { ok: true, member: { id: member.id, email: member.email, role } };
});

app.delete("/v1/account/members/:memberId", async (req, reply) => {
  const u = await requireUserRow(req, reply);
  if (!u) return;
  if (!canManageMembers(u)) {
    return reply.code(403).send({ ok: false, error: "forbidden" });
  }
  const ownerId = orgOwnerId(u);
  const member = await db.get("SELECT * FROM users WHERE id=? AND parent_user_id=?", req.params.memberId, ownerId);
  if (!member) return reply.code(404).send({ ok: false, error: "not_found" });
  await db.run("DELETE FROM sessions WHERE user_id=?", member.id);
  await db.run("DELETE FROM users WHERE id=?", member.id);
  return { ok: true };
});

// ── sites ───────────────────────────────────────────────────────────
app.get("/v1/sites", async (req, reply) => {
  const sess = (await sessionUser(req)) || (await bearerUser(req));
  if (!sess) return reply.code(401).send({ ok: false, error: "unauthorized" });
  const u = await loadUserRow(db, sess.user_id);
  if (!u) return reply.code(401).send({ ok: false, error: "unauthorized" });
  if (!hasPerm(u, PERMS.SITES_READ) && !hasPerm(u, PERMS.SITES_MANAGE)) {
    return reply.code(403).send({ ok: false, error: "forbidden" });
  }
  const ownerId = orgOwnerId(u);
  const raw = await db.all("SELECT * FROM sites WHERE user_id=? ORDER BY created_at DESC", ownerId);
  const rows = [];
  for (const s0 of raw) {
      const s = await maybeDowngradeSite(s0);
      const view = billingPublicView(s);
      rows.push({
        site_id: s.site_id,
        domain: s.domain,
        name: s.name,
        plan: view.plan,
        billing_status: view.status,
        paid_until: view.paid_until,
        grace_until: view.grace_until,
        trial_end: view.trial_end,
        price_usd_per_year: view.price_usd_per_year,
        profile_cap: view.profile_cap,
        rpa_enabled: view.rpa_enabled,
        device_precisions: view.device_precisions,
        primary_device_lane: view.primary_device_lane,
        data_retained: true,
        verified: !!s.verified_at,
        verified_at: s.verified_at || null,
        verify: s.verify_token
          ? {
              dns_name: dnsFqdn(s.domain),
              dns_txt: s.verify_token,
              http_path: "/.well-known/gv6-verify.txt",
              http_body: s.verify_token,
            }
          : null,
      });
    }
  return { ok: true, sites: rows };
});

app.post("/v1/sites", async (req, reply) => {
  const u = await requirePerm(req, reply, PERMS.SITES_MANAGE);
  if (!u) return;
  const ownerId = orgOwnerId(u);
  const domain = String(req.body?.domain || "")
    .trim()
    .toLowerCase()
    .replace(/^https?:\/\//, "")
    .replace(/\/.*$/, "");
  const name = String(req.body?.name || domain).trim();
  if (!domain || !domain.includes(".")) {
    return reply.code(400).send({ ok: false, error: "invalid_domain" });
  }
  const taken = await db.get("SELECT site_id FROM sites WHERE domain=?", domain);
  if (taken) return reply.code(409).send({ ok: false, error: "domain_taken" });
  const site_id = "site_" + randomToken(8);
  const freeDefault = "builtin_plan@1";
  const vtok = makeVerifyToken(site_id);
  const tms = trialMsFromEnv();
  const trialEnd = tms > 0 ? now() + tms : null;
  const billingStatus = trialEnd ? "trial" : "free";
  try {
    await db.run(`INSERT INTO sites (site_id, user_id, domain, name, plan, paid_until, strategy_version_id, created_at, verify_token, trial_end, billing_status)
       VALUES (?, ?, ?, ?, 'free', NULL, ?, ?, ?, ?, ?)`, site_id, ownerId, domain, name, freeDefault, now(), vtok.token, trialEnd, billingStatus);
  } catch (e) {
    // older schema without verify_token column already migrated; rethrow others
    throw e;
  }
  const created = {
    site_id,
    domain,
    name,
    plan: "free",
    paid_until: null,
    trial_end: trialEnd,
    billing_status: billingStatus,
  };
  const view = billingPublicView(created);
  return {
    ok: true,
    site: {
      site_id,
      domain,
      name,
      plan: view.plan,
      billing_status: view.status,
      trial_end: view.trial_end,
      price_usd_per_year: view.price_usd_per_year,
      rpa_enabled: view.rpa_enabled,
      device_precisions: view.device_precisions,
      data_retained: true,
    },
  };
});

app.post("/v1/sites/:siteId/pay", async (req, reply) => {
  const u = await requireSensitive(req, reply, PERMS.BILLING);
  if (!u) return;
  const ownerId = orgOwnerId(u);
  const site = await db.get("SELECT * FROM sites WHERE site_id=? AND user_id=?", req.params.siteId, ownerId);
  if (!site) return reply.code(404).send({ ok: false, error: "not_found" });
  // Production: Stripe Checkout. Lab: mock annual pay.
  if (activeBillingAdapter instanceof StripeBillingAdapter) {
    try {
      const session = await activeBillingAdapter.createCheckoutSession(db, {
        site,
        userId: ownerId,
        now: now(),
      });
      return {
        ok: true,
        adapter: "stripe",
        checkout_url: session.url,
        session_id: session.session_id,
        note: "Complete payment on Stripe Checkout; entitlement applied on webhook.",
      };
    } catch (e) {
      return reply.code(502).send({ ok: false, error: "stripe_checkout_failed", detail: String(e.message || e) });
    }
  }
  const pay = await activeBillingAdapter.payAnnual(db, {
    site,
    userId: ownerId,
    now: now(),
    note: "mock_annual",
  });
  const ent = planEntitlement("paid");
  return {
    ok: true,
    adapter: pay.adapter,
    plan: "paid",
    billing_status: pay.status,
    paid_until: pay.paid_until,
    amount_usd: pay.amount_usd,
    data_retained: true,
    ...ent,
  };
});

// Explicit Stripe Checkout session creation (production).
app.post("/v1/sites/:siteId/checkout", async (req, reply) => {
  const u = await requireSensitive(req, reply, PERMS.BILLING);
  if (!u) return;
  if (!(activeBillingAdapter instanceof StripeBillingAdapter)) {
    return reply.code(404).send({ ok: false, error: "stripe_not_configured", hint: "Set STRIPE_SECRET_KEY + STRIPE_PRICE_ID for production." });
  }
  const ownerId = orgOwnerId(u);
  const site = await db.get("SELECT * FROM sites WHERE site_id=? AND user_id=?", req.params.siteId, ownerId);
  if (!site) return reply.code(404).send({ ok: false, error: "not_found" });
  try {
    const session = await activeBillingAdapter.createCheckoutSession(db, {
      site,
      userId: ownerId,
      now: now(),
    });
    return {
      ok: true,
      adapter: "stripe",
      checkout_url: session.url,
      session_id: session.session_id,
    };
  } catch (e) {
    return reply.code(502).send({ ok: false, error: "stripe_checkout_failed", detail: String(e.message || e) });
  }
});

app.post("/v1/sites/:siteId/strategy", async (_req, reply) => {
  return reply.code(410).send({
    ok: false,
    error: "strategy_config_removed",
    note: "Users no longer configure analysis algorithms; free vs paid controls device precision + RPA only.",
  });
});

// ── Stripe webhook (production) ────────────────────────────────────
// Raw body is captured by the content type parser into req.rawBody for
// signature verification. Idempotent event storage prevents replays.
app.post("/v1/billing/stripe-webhook", async (req, reply) => {
  if (!(activeBillingAdapter instanceof StripeBillingAdapter)) {
    return reply.code(404).send({ ok: false, error: "stripe_not_configured" });
  }
  const sig = req.headers["stripe-signature"];
  if (!sig) return reply.code(400).send({ ok: false, error: "missing_signature" });
  const raw = req.rawBody ? req.rawBody.toString("utf8") : "";
  if (!raw) return reply.code(400).send({ ok: false, error: "empty_body" });
  const stripe = await activeBillingAdapter._client();
  let event;
  try {
    event = stripe.webhooks.constructEvent(raw, sig, activeBillingAdapter.webhookSecret);
  } catch (e) {
    app.log.warn({ msg: "stripe_webhook_sig_fail", err: String(e.message || e) });
    return reply.code(400).send({ ok: false, error: "signature_verification_failed" });
  }
  try {
    const result = await activeBillingAdapter.handleWebhookEvent(db, event, now());
    return reply.code(200).send({ received: true, ...result });
  } catch (e) {
    app.log.error({ msg: "stripe_webhook_handler_error", err: String(e.message || e) });
    return reply.code(500).send({ ok: false, error: "webhook_handler_error" });
  }
});

app.get("/v1/strategies", async (req, reply) => {
  const u = (await sessionUser(req)) || (await bearerUser(req));
  if (!u) return reply.code(401).send({ ok: false, error: "unauthorized" });
  return {
    ok: true,
    strategies: [],
    note: "User strategy catalog removed. Entitlements: free=dv4/5/6 no RPA; paid=all lanes + RPA.",
    entitlements: {
      free: planEntitlement("free"),
      paid: planEntitlement("paid"),
    },
  };
});

// ── OAuth2 Authorization Code + PKCE ────────────────────────────────
app.get("/oauth/authorize", async (req, reply) => {
  if (await authRateLimited(req, reply, "oauth_authorize", 30)) return;
  const q = req.query || {};
  const client_id = String(q.client_id || "");
  const redirect_uri = String(q.redirect_uri || "");
  const state = String(q.state || "");
  const code_challenge = String(q.code_challenge || "");
  const code_challenge_method = String(q.code_challenge_method || "S256");
  const response_type = String(q.response_type || "code");
  if (response_type !== "code") return reply.code(400).send("unsupported_response_type");
  const client = await db.get("SELECT * FROM oauth_clients WHERE client_id=?", client_id);
  if (!client) return reply.code(400).send("invalid_client");
  const uris = JSON.parse(client.redirect_uris_json);
  if (!uris.includes(redirect_uri)) return reply.code(400).send("invalid_redirect_uri");
  if (!code_challenge || code_challenge_method !== "S256") {
    return reply.code(400).send("pkce_required");
  }
  const u = await sessionUser(req);
  if (!u) {
    const ret = `${PUBLIC_URL}/oauth/authorize?${new URLSearchParams(q).toString()}`;
    return reply.redirect(accountLoginUrl(ret));
  }
  if (IS_PROD && (process.env.GR_OFFICIAL_LAB_EMAIL ?? process.env.GV6_OFFICIAL_LAB_EMAIL) !== "1") {
    const row = await db.get("SELECT email_verified FROM users WHERE id=?", u.user_id);
    if (!row?.email_verified) {
      return reply.code(403).send({ ok: false, error: "email_unverified" });
    }
  }
  const code = randomToken(24);
  await db.run(`INSERT INTO oauth_codes (code, client_id, user_id, redirect_uri, code_challenge, expires_at, created_at)
     VALUES (?, ?, ?, ?, ?, ?, ?)`, code, client_id, u.user_id, redirect_uri, code_challenge, now() + CODE_TTL_MS, now());
  const url = new URL(redirect_uri);
  url.searchParams.set("code", code);
  if (state) url.searchParams.set("state", state);
  return reply.redirect(url.toString());
});

app.post("/oauth/token", async (req, reply) => {
  if (await authRateLimited(req, reply, "oauth_token", 30)) return;
  const body = req.body || {};
  const grant = String(body.grant_type || "");
  if (!["authorization_code", "refresh_token"].includes(grant)) {
    return reply.code(400).send({ error: "unsupported_grant_type" });
  }
  if (grant === "refresh_token") {
    const refresh = String(body.refresh_token || "");
    if (!refresh) return reply.code(400).send({ error: "invalid_grant" });
    // Atomic consume: rotation makes refresh-token replay fail, including
    // concurrent requests racing with the same token.
    const old = await db.get(
      "DELETE FROM access_tokens WHERE refresh_token=? RETURNING user_id, client_id",
      refresh
    );
    if (!old) return reply.code(400).send({ error: "invalid_grant" });
    const access = randomToken(32);
    const nextRefresh = randomToken(32);
    const exp = now() + ACCESS_TTL_SEC * 1000;
    await db.run(
      `INSERT INTO access_tokens (token, refresh_token, user_id, client_id, expires_at, created_at)
       VALUES (?, ?, ?, ?, ?, ?)`,
      access, nextRefresh, old.user_id, old.client_id, exp, now()
    );
    return {
      token_type: "Bearer",
      access_token: access,
      expires_in: ACCESS_TTL_SEC,
      refresh_token: nextRefresh,
      scope: "sites strategies offline",
    };
  }
  const code = String(body.code || "");
  const redirect_uri = String(body.redirect_uri || "");
  const client_id = String(body.client_id || "");
  const verifier = String(body.code_verifier || "");
  const row = await db.get("SELECT * FROM oauth_codes WHERE code=?", code);
  if (!row || Number(row.expires_at) < now()) {
    return reply.code(400).send({ error: "invalid_grant" });
  }
  if (row.client_id !== client_id || row.redirect_uri !== redirect_uri) {
    return reply.code(400).send({ error: "invalid_grant" });
  }
  const challenge = sha256b64url(verifier);
  if (challenge !== row.code_challenge) {
    return reply.code(400).send({ error: "invalid_grant", error_description: "pkce_mismatch" });
  }
  await db.run("DELETE FROM oauth_codes WHERE code=?", code);
  const access = randomToken(32);
  const refresh = randomToken(32);
  const exp = now() + ACCESS_TTL_SEC * 1000;
  await db.run(`INSERT INTO access_tokens (token, refresh_token, user_id, client_id, expires_at, created_at)
     VALUES (?, ?, ?, ?, ?, ?)`, access, refresh, row.user_id, client_id, exp, now());
  return {
    token_type: "Bearer",
    access_token: access,
    expires_in: ACCESS_TTL_SEC,
    refresh_token: refresh,
    scope: "sites strategies offline",
  };
});

app.post("/oauth/revoke", async (req, reply) => {
  if (await authRateLimited(req, reply, "oauth_revoke", 30)) return;
  const token = String(req.body?.token || "");
  if (!token) return reply.code(400).send({ error: "invalid_request" });
  await db.run("DELETE FROM access_tokens WHERE token=? OR refresh_token=?", token, token);
  return reply.code(200).send({});
});

// ── runtime plan entitlement bundle (memory-only on node) ───────────
// Lab compatibility only. Production should use /v1/runtime/algo-bundle (ECDH).
app.post("/v1/runtime/strategy-bundle", async (req, reply) => {
  if (IS_PROD) {
    return reply.code(410).send({
      ok: false,
      error: "gone",
      hint: "Use POST /v1/runtime/algo-bundle (ECDH v2)",
    });
  }
  const u = await requireBearer(req, reply);
  if (!u) return;
  const domain = String(req.body?.domain || "")
    .trim()
    .toLowerCase();
  const site_id = String(req.body?.site_id || "");
  let site = null;
  if (site_id) {
    site = await db.get("SELECT * FROM sites WHERE site_id=? AND user_id=?", site_id, u.user_id);
  } else if (domain) {
    site = await db.get("SELECT * FROM sites WHERE domain=? AND user_id=?", domain, u.user_id);
  }
  if (!site) return reply.code(404).send({ ok: false, error: "site_not_found" });
  site = await maybeDowngradeSite(site);
  if (domain && site.domain !== domain) {
    return reply.code(403).send({ ok: false, error: "domain_mismatch" });
  }
  const plan = planActive(site);
  if (plan === "paid" && !site.verified_at && REQUIRE_DOMAIN_VERIFY) {
    return reply.code(403).send({ ok: false, error: "domain_not_verified" });
  }
  const ent = planEntitlement(plan);
  const payload = {
    ...buildPlanPayload({ site_id: site.site_id, domain: site.domain, plan }),
    license_token: issueLicenseToken({ site_id: site.site_id, domain: site.domain, plan }),
  };
  const { iv, ciphertext } = encryptBundle(payload);
  const header = {
    alg: "EdDSA",
    enc: "A256GCM",
    kid: "official-1",
    site_id: site.site_id,
    domain: site.domain,
    plan,
    profile_cap: ent.profile_cap,
    rpa_enabled: ent.rpa_enabled,
    strategy_version_id: "builtin_plan@1",
    exp: Math.floor(now() / 1000) + BUNDLE_TTL_SEC,
    bundle_id: randomToken(12),
  };
  const headerBytes = Buffer.from(JSON.stringify(header), "utf8");
  const mac = crypto.createHmac("sha256", wrapKey()).update(headerBytes).update(iv).update(ciphertext).digest();
  return {
    ok: true,
    header,
    header_b64: headerBytes.toString("base64"),
    iv_b64: iv.toString("base64"),
    ciphertext_b64: ciphertext.toString("base64"),
    mac_b64: mac.toString("base64"),
    sig_b64: mac.toString("base64"),
    mac_alg: "HMAC-SHA256-wrap",
    expires_in: BUNDLE_TTL_SEC,
    note: "Plan entitlement decrypt in-process only; no user strategy body.",
  };
});


// ── domain verify (paid ownership) ─────────────────────────────────
app.post("/v1/sites/:siteId/verify", async (req, reply) => {
  const u = await requireUserRow(req, reply);
  if (!u) return;
  const ownerId = orgOwnerId(u);
  const site = await db.get("SELECT * FROM sites WHERE site_id=? AND user_id=?", req.params.siteId, ownerId);
  if (!site) return reply.code(404).send({ ok: false, error: "not_found" });
  if (!site.verify_token) {
    const vtok = makeVerifyToken(site.site_id);
    await db.run("UPDATE sites SET verify_token=? WHERE site_id=?", vtok.token, site.site_id);
    site.verify_token = vtok.token;
  }
  const result = await verifyDomain(site.domain, site.verify_token);
  if (!result.ok) {
    return {
      ok: false,
      error: "not_verified_yet",
      instruction: {
        dns_name: dnsFqdn(site.domain),
        dns_txt: site.verify_token,
        http_path: `https://${site.domain}/.well-known/gv6-verify.txt`,
        http_body: site.verify_token,
      },
      detail: result,
    };
  }
  await db.run("UPDATE sites SET verified_at=?, verify_method=? WHERE site_id=?", 
    now(),
    result.method,
    site.site_id
  );
  return { ok: true, verified_at: now(), method: result.method, domain: site.domain };
});

// ── node enroll (panel instance X25519 pubkey) ──────────────────────
app.post("/v1/nodes/enroll", async (req, reply) => {
  const u = (await bearerUser(req)) || (await requireUser(req, reply));
  if (!u) return;
  const instance_id = String(req.body?.instance_id || "").trim();
  const x25519_pub_b64 = String(req.body?.x25519_pub_b64 || "").trim();
  const label = String(req.body?.label || "panel").slice(0, 64);
  if (!instance_id || !x25519_pub_b64) {
    return reply.code(400).send({ ok: false, error: "instance_id_and_pubkey_required" });
  }
  await db.run(`INSERT INTO nodes (instance_id, user_id, label, x25519_pub_b64, created_at, last_seen_at)
     VALUES (?, ?, ?, ?, ?, ?)
     ON CONFLICT(instance_id) DO UPDATE SET
       x25519_pub_b64=excluded.x25519_pub_b64,
       label=excluded.label,
       last_seen_at=excluded.last_seen_at,
       user_id=excluded.user_id`, instance_id, u.user_id, label, x25519_pub_b64, now(), now());
  return { ok: true, instance_id, kex: "X25519-HKDF-SHA256" };
});

// ── algo-bundle v2 (ECDH) — preferred over shared wrap-key ─────────
app.post("/v1/runtime/algo-bundle", async (req, reply) => {
  const u = await requireBearer(req, reply);
  if (!u) return;
  const domain = String(req.body?.domain || "").trim().toLowerCase();
  const site_id = String(req.body?.site_id || "");
  const instance_id = String(req.body?.instance_id || "");
  const node_ephemeral_pub_b64 = String(req.body?.node_ephemeral_pub_b64 || "").trim();
  if (!node_ephemeral_pub_b64) {
    return reply.code(400).send({ ok: false, error: "node_ephemeral_pub_b64_required" });
  }
  let site = null;
  if (site_id) {
    site = await db.get("SELECT * FROM sites WHERE site_id=? AND user_id=?", site_id, u.user_id);
  } else if (domain) {
    site = await db.get("SELECT * FROM sites WHERE domain=? AND user_id=?", domain, u.user_id);
  }
  if (!site) return reply.code(404).send({ ok: false, error: "site_not_found" });
  site = await maybeDowngradeSite(site);
  const plan = planActive(site);
  if (plan === "paid" && !site.verified_at && REQUIRE_DOMAIN_VERIFY) {
    return reply.code(403).send({ ok: false, error: "domain_not_verified" });
  }
  const payload = {
    ...buildPlanPayload({ site_id: site.site_id, domain: site.domain, plan }),
    license_token: issueLicenseToken({ site_id: site.site_id, domain: site.domain, plan }),
  };
  const serverKp = generateNodeKeypair();
  let aesKey;
  try {
    aesKey = ecdhAesKey(serverKp.private_pem, node_ephemeral_pub_b64);
  } catch (e) {
    return reply.code(400).send({ ok: false, error: "bad_ephemeral_pub", detail: String(e.message || e) });
  }
  const { iv_b64, ciphertext_b64 } = encryptWithKey(aesKey, payload);
  const header = buildAlgoBundleHeader({
    site_id: site.site_id,
    domain: site.domain,
    plan,
    instance_id,
    node_ephemeral_pub_b64,
    server_ephemeral_pub_b64: serverKp.public_b64,
    exp_unix: Math.floor(now() / 1000) + BUNDLE_TTL_SEC,
    bundle_id: randomToken(12),
  });
  const headerBytes = Buffer.from(JSON.stringify(header), "utf8");
  const iv = Buffer.from(iv_b64, "base64");
  const ct = Buffer.from(ciphertext_b64, "base64");
  // Authenticate with server Ed25519 (preferred) — also HMAC with ECDH key for transit binding
  const mac = crypto.createHmac("sha256", aesKey).update(headerBytes).update(iv).update(ct).digest();
  const sig = signBytes(Buffer.concat([headerBytes, iv, ct]));
  if (instance_id) {
    await db.run("UPDATE nodes SET last_seen_at=? WHERE instance_id=? AND user_id=?", 
      now(),
      instance_id,
      u.user_id
    );
  }
  return {
    ok: true,
    protocol: "algo-bundle-v2",
    header,
    header_b64: headerBytes.toString("base64"),
    iv_b64,
    ciphertext_b64,
    mac_b64: mac.toString("base64"),
    sig_b64: sig.toString("base64"),
    expires_in: BUNDLE_TTL_SEC,
    note: "Decrypt with ECDH(server_ephemeral, node_ephemeral); plaintext memory-only.",
  };
});

// ── account: email verify + TOTP ────────────────────────────────────
app.post("/v1/account/email/request-verify", async (req, reply) => {
  const u = await requireUser(req, reply);
  if (!u) return;
  const tok = issueEmailVerifyToken();
  await db.run("DELETE FROM email_tokens WHERE user_id=?", u.user_id);
  await db.run("INSERT INTO email_tokens (user_id, token_hash, expires_at) VALUES (?, ?, ?)", u.user_id, tok.hash, tok.expires_at);
  // Production: send verification email via Gmail SMTP (no token echo).
  // Lab fallback: no Gmail creds → no-op; echo lab_token only when GR_OFFICIAL_LAB_EMAIL=1.
  const emailRow = await db.get("SELECT email FROM users WHERE id=?", u.user_id);
  const email = emailRow?.email || "";
  let emailSent = false;
  let mailError = null;
  if (IS_PROD && !activeGmailConfig) {
    return reply.code(503).send({ ok: false, error: "gmail_not_configured" });
  }
  if (activeGmailConfig) {
    try {
      const res = await sendVerificationEmail({
        to: email,
        rawToken: tok.raw,
        publicUrl: PUBLIC_URL,
        env: {
          ...process.env,
          GMAIL_USER: activeGmailConfig.user,
          GMAIL_APP_PASSWORD: activeGmailConfig.pass,
          GMAIL_FROM: activeGmailConfig.from,
          GMAIL_SMTP_HOST: activeGmailConfig.host,
          GMAIL_SMTP_PORT: String(activeGmailConfig.port),
        },
      });
      emailSent = !!res.sent;
    } catch (e) {
      mailError = String(e.message || e);
      app.log.warn({ msg: "gmail_send_failed", err: mailError });
    }
  }
  if (IS_PROD && !emailSent) {
    return reply.code(503).send({ ok: false, error: "verification_email_unavailable" });
  }
  // Never echo the raw token in production. Lab may echo when GR_OFFICIAL_LAB_EMAIL=1.
  const labEcho = (process.env.GR_OFFICIAL_LAB_EMAIL ?? process.env.GV6_OFFICIAL_LAB_EMAIL) === "1";
  return {
    ok: true,
    email_sent: emailSent,
    lab_token: labEcho ? tok.raw : undefined,
    note: emailSent
      ? "Verification email sent. Check your inbox."
      : mailError
        ? "Email send failed; contact support."
        : "Production: email link only. Set GR_OFFICIAL_LAB_EMAIL=1 to echo token (lab; legacy GV6_ accepted).",
    expires_at: tok.expires_at,
  };
});

app.post("/v1/account/email/confirm", async (req, reply) => {
  const u = await requireUser(req, reply);
  if (!u) return;
  const raw = String(req.body?.token || "");
  const row = await db.get("SELECT * FROM email_tokens WHERE user_id=? AND token_hash=?", u.user_id, hashToken(raw));
  if (!row || Number(row.expires_at) < now()) {
    return reply.code(400).send({ ok: false, error: "invalid_or_expired" });
  }
  await db.run("UPDATE users SET email_verified=TRUE WHERE id=?", u.user_id);
  await db.run("DELETE FROM email_tokens WHERE user_id=?", u.user_id);
  return { ok: true, email_verified: true };
});

app.post("/v1/account/totp/setup", async (req, reply) => {
  const u = await requireUser(req, reply);
  if (!u) return;
  const secret = generateTotpSecret();
  await db.run("UPDATE users SET totp_secret=?, totp_enabled=FALSE WHERE id=?", secret, u.user_id);
  const emailRow = await db.get("SELECT email FROM users WHERE id=?", u.user_id);
  const email = emailRow?.email || "user";
  return {
    ok: true,
    secret,
    otpauth_url: `otpauth://totp/GreenV6:${encodeURIComponent(email)}?secret=${secret}&issuer=GreenV6`,
    note: "Confirm with POST /v1/account/totp/enable { code }",
  };
});

app.post("/v1/account/totp/enable", async (req, reply) => {
  const u = await requireUser(req, reply);
  if (!u) return;
  const row = await db.get("SELECT totp_secret FROM users WHERE id=?", u.user_id);
  if (!row?.totp_secret) return reply.code(400).send({ ok: false, error: "setup_first" });
  if (!verifyTotp(row.totp_secret, req.body?.code)) {
    return reply.code(400).send({ ok: false, error: "bad_code" });
  }
  const codes = recoveryCodes();
  await db.transaction(async (tx) => {
    await tx.run("UPDATE users SET totp_enabled=TRUE WHERE id=?", u.user_id);
    await tx.run("DELETE FROM totp_recovery_codes WHERE user_id=?", u.user_id);
    for (const code of codes) {
      await tx.run(
        "INSERT INTO totp_recovery_codes (user_id, code_hash, created_at) VALUES (?, ?, ?)",
        u.user_id, hashToken(code), now()
      );
    }
  });
  const sid = req.cookies?.[SESSION_COOKIE];
  if (sid) {
    await db.run("DELETE FROM sessions WHERE user_id=? AND token<>?", u.user_id, sid);
  } else {
    await db.run("DELETE FROM sessions WHERE user_id=?", u.user_id);
  }
  await auditAccount(req, u.user_id, "totp_enabled");
  return { ok: true, totp_enabled: true, recovery_codes: codes };
});

app.post("/v1/account/totp/recovery-codes", async (req, reply) => {
  const sess = await requireUser(req, reply);
  if (!sess) return;
  if (await authRateLimited(req, reply, "totp_recovery_regenerate", 5)) return;
  const u = await loadUserRow(db, sess.user_id);
  if (!u?.totp_enabled || !(await verifySensitiveFactor(req, u, sess))) {
    return reply.code(401).send({ ok: false, error: "factor_required" });
  }
  const codes = recoveryCodes();
  await db.transaction(async (tx) => {
    await tx.run("DELETE FROM totp_recovery_codes WHERE user_id=?", u.id);
    for (const code of codes) {
      await tx.run("INSERT INTO totp_recovery_codes (user_id, code_hash, created_at) VALUES (?, ?, ?)", u.id, hashToken(code), now());
    }
  });
  await auditAccount(req, u.id, "totp_recovery_codes_rotated");
  return { ok: true, recovery_codes: codes };
});

app.post("/v1/account/totp/disable", async (req, reply) => {
  const sess = await requireUser(req, reply);
  if (!sess) return;
  if (await authRateLimited(req, reply, "totp_disable", 5)) return;
  const u = await loadUserRow(db, sess.user_id);
  if (!u?.totp_enabled || !(await verifySensitiveFactor(req, u, sess))) return reply.code(401).send({ ok: false, error: "factor_required" });
  await db.transaction(async (tx) => {
    await tx.run("UPDATE users SET totp_secret=NULL, totp_enabled=FALSE WHERE id=?", u.id);
    await tx.run("DELETE FROM totp_recovery_codes WHERE user_id=?", u.id);
    await tx.run("DELETE FROM sessions WHERE user_id=? AND token<>?", u.id, sess.token);
  });
  await auditAccount(req, u.id, "totp_disabled");
  return { ok: true, totp_enabled: false };
});

// ── support tickets ────────────────────────────────────────────────
app.get("/v1/tickets", async (req, reply) => {
  const u = await requireUser(req, reply);
  if (!u) return;
  const rows = await db.all(
    "SELECT id, site_id, category, subject, status, created_at, updated_at FROM tickets WHERE user_id=? ORDER BY updated_at DESC",
    u.user_id
  );
  return { ok: true, tickets: rows, categories: ticketCategories() };
});

app.post("/v1/tickets", async (req, reply) => {
  const u = await requireUser(req, reply);
  if (!u) return;
  const category = String(req.body?.category || "other");
  if (!ticketCategories().includes(category)) {
    return reply.code(400).send({ ok: false, error: "bad_category", categories: ticketCategories() });
  }
  const subject = String(req.body?.subject || "").trim().slice(0, 200);
  const body = String(req.body?.body || "").trim().slice(0, 8000);
  if (subject.length < 3 || body.length < 3) {
    return reply.code(400).send({ ok: false, error: "subject_body_required" });
  }
  const id = "tkt_" + randomToken(10);
  const t = now();
  await db.run(`INSERT INTO tickets (id, user_id, site_id, category, subject, body, status, created_at, updated_at)
     VALUES (?, ?, ?, ?, ?, ?, 'open', ?, ?)`, id, u.user_id, req.body?.site_id || null, category, subject, body, t, t);
  return { ok: true, ticket: { id, category, subject, status: "open" } };
});

app.get("/v1/tickets/:id", async (req, reply) => {
  const u = await requireUser(req, reply);
  if (!u) return;
  const t = await db.get("SELECT * FROM tickets WHERE id=? AND user_id=?", req.params.id, u.user_id);
  if (!t) return reply.code(404).send({ ok: false, error: "not_found" });
  const replies = await db.all(
    "SELECT id, author, body, created_at FROM ticket_replies WHERE ticket_id=? ORDER BY created_at",
    t.id
  );
  return { ok: true, ticket: t, replies };
});

async function bootstrap() {
  db = await openDb(defaultDatabaseUrl());
  await migrate(db);
  const loadedIntegrations = await loadIntegrationSettings(db, SETTINGS_KEY);
  activeIntegrationConfig = loadedIntegrations.config;
  activeBillingAdapter = loadedIntegrations.adapter;
  activeGmailConfig = loadedIntegrations.gmail;
  if (IS_PROD && !(activeBillingAdapter instanceof StripeBillingAdapter)) {
    throw new Error("production_requires_stripe_configuration");
  }
  attachRateLimitDb(db);
  await migrateContent(db);
  await seedContentIfEmpty(db);
  await patchPricingFaq(db);
  await patchMarketingLayout(db);
  await patchLocaleUrls(db);
  await seedExtraContentPages(db);
  await ensureKeys();
  await ensureLicenseKey();
  await ensureOAuthClient();
  await seedStrategiesIfEmpty();
  registerContentRoutes(app, { db, requireUser, requireUserRow, isCmsStaff, now });
  registerIntegrationRoutes(app, {
    db,
    key: SETTINGS_KEY,
    requireUserRow,
    isCmsStaff,
    now,
    getConfig: () => activeIntegrationConfig,
    applyConfig: async (config) => {
      activeIntegrationConfig = config;
      activeBillingAdapter = config.stripe?.secret_key
        ? new StripeBillingAdapter({
          secretKey: config.stripe.secret_key,
          webhookSecret: config.stripe.webhook_secret,
          priceId: config.stripe.price_id,
          successUrl: config.stripe.success_url,
          cancelUrl: config.stripe.cancel_url,
          annualUsd: config.stripe.annual_usd,
          periodMs: config.stripe.period_ms,
          graceMs: config.stripe.grace_ms,
        })
        : new MockBillingAdapter();
      activeGmailConfig = config.gmail?.user && config.gmail?.app_password
        ? {
          ...config.gmail,
          pass: config.gmail.app_password,
          secure: Number(config.gmail.port || 465) === 465,
          publicUrl: PUBLIC_URL,
          verifyPath: (process.env.GR_ACCOUNT_PATH ?? process.env.GV6_ACCOUNT_PATH) || "/account",
          labEcho: (process.env.GR_OFFICIAL_LAB_EMAIL ?? process.env.GV6_OFFICIAL_LAB_EMAIL) === "1",
        }
        : null;
    },
  });
  const HOST = (process.env.GR_OFFICIAL_HOST ?? process.env.GV6_OFFICIAL_HOST) || "0.0.0.0";
  await app.listen({ port: PORT, host: HOST });
  app.log.info(
    `official-site API on ${PUBLIC_URL} deploy=${DEPLOY_ENV} db=postgresql cms_admin=/${adminCmsSegment()} expose_wrap_key=${EXPOSE_WRAP_KEY} require_domain_verify=${REQUIRE_DOMAIN_VERIFY} billing=${activeBillingAdapter instanceof StripeBillingAdapter ? "stripe" : "mock"} gmail=${activeGmailConfig ? "on" : "lab_noop"}`
  );
}

bootstrap().catch((err) => {
  console.error(err);
  process.exit(1);
});
