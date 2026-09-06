/**
 * Unit: StripeBillingAdapter webhook idempotency + state reconciliation.
 * Uses an in-memory DB and a mock Stripe client — no network, no real API.
 */
import {
  GRACE_MS,
  YEAR_MS,
  resolveBilling,
  StripeBillingAdapter,
  stripeConfigFromEnv,
  selectBillingAdapter,
  MockBillingAdapter,
} from "../src/billing.js";
import { sendVerificationEmail, gmailConfigFromEnv } from "../src/account_security.js";
import crypto from "node:crypto";
import Stripe from "stripe";

function assert(cond, msg) {
  if (!cond) throw new Error(msg);
}

// ── In-memory DB that simulates the subset of SQL the adapter uses ──
class MemoryDb {
  constructor() {
    this.sites = new Map();
    this.payments = [];
    this.stripeEvents = new Map();
    this.stripeCustomers = new Map(); // site_id -> customer_id
    this.stripeSubscriptions = new Map(); // site_id -> { subscription_id, status, current_period_end }
    this.failNextPaidUpdate = false;
  }
  async transaction(fn) {
    // Adapter unit fixture. Production rollback semantics are covered by the
    // PostgreSQL smoke suite; this keeps the test double API-compatible.
    return fn(this);
  }
  async get(sql, ...params) {
    if (/FROM sites s JOIN stripe_customers/.test(sql)) {
      const customerId = params[0];
      for (const [siteId, cid] of this.stripeCustomers) {
        if (cid === customerId) return this.sites.get(siteId) || null;
      }
      return null;
    }
    if (/SELECT \* FROM sites WHERE/.test(sql)) {
      const siteId = params[0];
      return this.sites.get(siteId) || null;
    }
    if (/SELECT email FROM users/.test(sql)) {
      return { email: "user@example.com" };
    }
    if (/SELECT processed_at FROM stripe_events/.test(sql)) {
      return this.stripeEvents.get(params[0]) || null;
    }
    if (/SELECT id FROM payments WHERE site_id=/.test(sql)) {
      return this.payments.find((p) => p.site_id === params[0] && p.note === params[1]) || null;
    }
    return null;
  }
  async all() {
    return [];
  }
  async run(sql, ...params) {
    if (/INSERT INTO stripe_events/.test(sql)) {
      const eventId = params[0];
      if (this.stripeEvents.has(eventId)) {
        throw new Error("duplicate key value violates unique constraint");
      }
      this.stripeEvents.set(eventId, {
        event_id: eventId,
        type: params[1],
        created_at: params[2],
        processed_at: null,
        payload_json: params[4],
      });
      return { changes: 1 };
    }
    if (/UPDATE stripe_events SET processed_at/.test(sql)) {
      const eventId = params[params.length - 1];
      const ev = this.stripeEvents.get(eventId);
      if (ev) ev.processed_at = params[0];
      return { changes: 1 };
    }
    if (/UPDATE stripe_events SET attempts=attempts\+1/.test(sql)) {
      const ev = this.stripeEvents.get(params[params.length - 1]);
      if (ev) ev.attempts = (ev.attempts || 1) + 1;
      return { changes: 1 };
    }
    if (/UPDATE stripe_events SET last_error/.test(sql)) {
      const ev = this.stripeEvents.get(params[params.length - 1]);
      if (ev) ev.last_error = params[0];
      return { changes: 1 };
    }
    if (/INSERT INTO stripe_customers/.test(sql)) {
      const siteId = params[0];
      const customerId = params[1];
      this.stripeCustomers.set(siteId, customerId);
      return { changes: 1 };
    }
    if (/INSERT INTO stripe_subscriptions/.test(sql)) {
      const siteId = params[0];
      this.stripeSubscriptions.set(siteId, {
        subscription_id: params[1],
        status: params[2],
        current_period_end: params[3],
      });
      return { changes: 1 };
    }
    if (/UPDATE stripe_subscriptions SET status/.test(sql)) {
      const siteId = params[2];
      const sub = this.stripeSubscriptions.get(siteId);
      if (sub) {
        sub.status = params[0];
        sub.current_period_end = params[1];
      }
      return { changes: 1 };
    }
    if (/UPDATE stripe_subscriptions SET status='deleted'/.test(sql)) {
      const siteId = params[0];
      const sub = this.stripeSubscriptions.get(siteId);
      if (sub) sub.status = "deleted";
      return { changes: 1 };
    }
    if (/UPDATE sites SET plan='paid', paid_until=\?, grace_until=NULL/.test(sql)) {
      if (this.failNextPaidUpdate) {
        this.failNextPaidUpdate = false;
        throw new Error("simulated_site_update_failure");
      }
      const siteId = params[1];
      const site = this.sites.get(siteId);
      if (site) {
        site.plan = "paid";
        site.paid_until = params[0];
        site.grace_until = null;
        site.billing_status = "active";
        site.trial_end = null;
      }
      return { changes: 1 };
    }
    if (/UPDATE sites SET plan='paid', paid_until=\?, grace_until=\?/.test(sql)) {
      const siteId = params[2];
      const site = this.sites.get(siteId);
      if (site) {
        site.plan = "paid";
        site.paid_until = params[0];
        site.grace_until = params[1];
        site.billing_status = "grace";
      }
      return { changes: 1 };
    }
    if (/UPDATE sites SET plan='free', billing_status='free'/.test(sql)) {
      const siteId = params[0];
      const site = this.sites.get(siteId);
      if (site) {
        site.plan = "free";
        site.billing_status = "free";
      }
      return { changes: 1 };
    }
    if (/UPDATE sites SET billing_status=\?/.test(sql)) {
      const siteId = params[1];
      const site = this.sites.get(siteId);
      if (site) site.billing_status = params[0];
      return { changes: 1 };
    }
    if (/INSERT INTO payments/.test(sql)) {
      this.payments.push({ id: params[0], site_id: params[1], amount: params[3], note: params[5] });
      return { changes: 1 };
    }
    return { changes: 0 };
  }
  async exec() {}
  async close() {}
}

// ── Mock Stripe SDK client ──
function mockStripeClient() {
  const sessions = [];
  return {
    checkout: {
      sessions: {
        create(opts) {
          sessions.push(opts);
          return Promise.resolve({
            id: "cs_test_" + sessions.length,
            url: "https://checkout.stripe.com/c/pay/cs_test_" + sessions.length,
          });
        },
      },
    },
    webhooks: {
      constructEvent(raw, sig, secret) {
        // Simulate signature verification: parse the raw body as the event.
        // In tests we pass a pre-built event JSON as `raw` with a dummy sig.
        const parsed = JSON.parse(raw);
        return parsed;
      },
    },
    _sessions: sessions,
  };
}

const t0 = 1_700_000_000_000;

function makeEvent(id, type, object) {
  return { id, type, data: { object }, created: Math.floor(t0 / 1000) };
}

async function main() {
  // ── stripeConfigFromEnv: null when no key ──
  {
    const cfg = stripeConfigFromEnv({ });
    assert(cfg === null, "no STRIPE_SECRET_KEY → null config");
    const cfg2 = stripeConfigFromEnv({ STRIPE_SECRET_KEY: "sk_test_x", STRIPE_WEBHOOK_SECRET: "whsec_y", STRIPE_PRICE_ID: "price_z" });
    assert(cfg2.secretKey === "sk_test_x" && cfg2.webhookSecret === "whsec_y" && cfg2.priceId === "price_z", "config reads env");
    assert(cfg2.annualUsd === 99, "default annual usd");
    assert(cfg2.periodMs === YEAR_MS, "default period ms");
    assert(cfg2.graceMs === GRACE_MS, "default grace ms");
    console.log("PASS  stripeConfigFromEnv reads env + defaults");
  }

  // ── selectBillingAdapter: mock when no key, stripe when key present ──
  {
    const a1 = selectBillingAdapter({ });
    assert(a1 instanceof MockBillingAdapter, "no key → MockBillingAdapter");
    const a2 = selectBillingAdapter({ STRIPE_SECRET_KEY: "sk_test_x", STRIPE_PRICE_ID: "price_z" });
    assert(a2 instanceof StripeBillingAdapter, "key present → StripeBillingAdapter");
    assert(a2.priceId === "price_z", "stripe adapter has priceId");
    console.log("PASS  selectBillingAdapter picks adapter by env");
  }

  // ── StripeBillingAdapter webhook idempotency + checkout.completed ──
  {
    const db = new MemoryDb();
    const site = { site_id: "site_a", user_id: "u1", plan: "free", paid_until: null, grace_until: null, billing_status: "free", trial_end: null };
    db.sites.set("site_a", site);

    const adapter = new StripeBillingAdapter({
      secretKey: "sk_test_x",
      priceId: "price_z",
      annualUsd: 99,
      periodMs: YEAR_MS,
      graceMs: GRACE_MS,
    });

    // checkout.session.completed → stores customer + subscription mapping
    const evt1 = makeEvent("evt_1", "checkout.session.completed", {
      client_reference_id: "site_a",
      customer: "cus_1",
      subscription: "sub_1",
    });
    const r1 = await adapter.handleWebhookEvent(db, evt1, t0);
    assert(r1.stored === true && r1.processed === true && r1.status === "checkout_completed", "checkout completed stored+processed");
    assert(db.stripeCustomers.get("site_a") === "cus_1", "customer mapping stored");
    assert(db.stripeSubscriptions.get("site_a").subscription_id === "sub_1", "subscription mapping stored");

    // Replay same event → idempotent, not stored again
    const r1b = await adapter.handleWebhookEvent(db, evt1, t0);
    assert(r1b.stored === false && r1b.processed === false && r1b.status === "duplicate", "replay is idempotent duplicate");
    assert(db.stripeEvents.size === 1, "only one event stored");
    console.log("PASS  checkout.session.completed + idempotent replay");
  }

  // ── invoice.paid → site becomes active paid ──
  {
    const db = new MemoryDb();
    const site = { site_id: "site_b", user_id: "u2", plan: "free", paid_until: null, grace_until: null, billing_status: "free", trial_end: null };
    db.sites.set("site_b", site);
    db.stripeCustomers.set("site_b", "cus_b");

    const adapter = new StripeBillingAdapter({ secretKey: "sk_test_x", priceId: "price_z", annualUsd: 99, periodMs: YEAR_MS, graceMs: GRACE_MS });

    const periodEndSec = Math.floor((t0 + YEAR_MS) / 1000);
    const evt = makeEvent("evt_2", "invoice.paid", {
      customer: "cus_b",
      subscription: "sub_b",
      lines: { data: [{ period: { end: periodEndSec } }] },
    });
    const r = await adapter.handleWebhookEvent(db, evt, t0);
    assert(r.status === "paid", "invoice.paid → paid");
    const updated = db.sites.get("site_b");
    assert(updated.plan === "paid" && updated.billing_status === "active", "site active after invoice.paid");
    assert(updated.paid_until === periodEndSec * 1000, "paid_until = period end");
    assert(db.payments.length === 1, "payment row inserted");
    assert(db.payments[0].note === "stripe_invoice_paid:evt_2", "payment note includes event id");
    const replay = await adapter.handleWebhookEvent(db, evt, t0 + 1);
    assert(replay.status === "duplicate" && db.payments.length === 1, "invoice replay does not duplicate payment");
    console.log("PASS  invoice.paid → active paid + payment row");
  }

  // ── invoice.payment_failed → grace ──
  {
    const db = new MemoryDb();
    const paidUntil = t0 - 1000;
    const site = { site_id: "site_c", user_id: "u3", plan: "paid", paid_until: paidUntil, grace_until: null, billing_status: "active", trial_end: null };
    db.sites.set("site_c", site);
    db.stripeCustomers.set("site_c", "cus_c");

    const adapter = new StripeBillingAdapter({ secretKey: "sk_test_x", priceId: "price_z", annualUsd: 99, periodMs: YEAR_MS, graceMs: GRACE_MS });
    const evt = makeEvent("evt_3", "invoice.payment_failed", { customer: "cus_c" });
    const r = await adapter.handleWebhookEvent(db, evt, t0);
    assert(r.status === "grace", "payment_failed → grace");
    const updated = db.sites.get("site_c");
    assert(updated.billing_status === "grace" && updated.grace_until > t0, "grace set");
    const snap = resolveBilling(updated, t0);
    assert(snap.entitlement_plan === "paid", "grace keeps paid entitlement");
    console.log("PASS  invoice.payment_failed → grace (paid retained)");
  }

  // ── failed processing remains retryable ──
  {
    const db = new MemoryDb();
    const site = { site_id: "site_retry", user_id: "u_retry", plan: "free", paid_until: null, grace_until: null, billing_status: "free", trial_end: null };
    db.sites.set("site_retry", site);
    db.stripeCustomers.set("site_retry", "cus_retry");
    db.failNextPaidUpdate = true;

    const adapter = new StripeBillingAdapter({ secretKey: "sk_test_x", priceId: "price_z" });
    const evt = makeEvent("evt_retry", "invoice.paid", {
      customer: "cus_retry",
      subscription: "sub_retry",
      current_period_end: Math.floor((t0 + YEAR_MS) / 1000),
    });
    let failed = false;
    try {
      await adapter.handleWebhookEvent(db, evt, t0);
    } catch (e) {
      failed = String(e.message) === "simulated_site_update_failure";
    }
    assert(failed, "first webhook attempt fails in fixture");
    assert(db.stripeEvents.get("evt_retry").processed_at === null, "failed event remains unprocessed");
    const retried = await adapter.handleWebhookEvent(db, evt, t0 + 1);
    assert(retried.stored === false && retried.processed === true && retried.status === "paid", "failed event retries successfully");
    assert(db.payments.length === 1, "retry creates one payment");
    console.log("PASS  failed webhook remains retryable");
  }

  // ── customer.subscription.deleted → free ──
  {
    const db = new MemoryDb();
    const site = { site_id: "site_d", user_id: "u4", plan: "paid", paid_until: t0 + YEAR_MS, grace_until: null, billing_status: "active", trial_end: null };
    db.sites.set("site_d", site);
    db.stripeCustomers.set("site_d", "cus_d");

    const adapter = new StripeBillingAdapter({ secretKey: "sk_test_x", priceId: "price_z", annualUsd: 99, periodMs: YEAR_MS, graceMs: GRACE_MS });
    const evt = makeEvent("evt_4", "customer.subscription.deleted", { customer: "cus_d" });
    const r = await adapter.handleWebhookEvent(db, evt, t0);
    assert(r.status === "free", "subscription.deleted → free");
    const updated = db.sites.get("site_d");
    assert(updated.plan === "free" && updated.billing_status === "free", "site downgraded to free");
    assert(db.sites.has("site_d"), "site row NOT deleted (data retained)");
    console.log("PASS  customer.subscription.deleted → free (data retained)");
  }

  // ── reconcile: past_due → free, never deletes ──
  {
    const db = new MemoryDb();
    const site = { site_id: "site_e", user_id: "u5", plan: "paid", paid_until: t0 - GRACE_MS - 1, grace_until: t0 - GRACE_MS - 1, billing_status: "past_due", trial_end: null };
    db.sites.set("site_e", site);

    const adapter = new StripeBillingAdapter({ secretKey: "sk_test_x", priceId: "price_z", annualUsd: 99, periodMs: YEAR_MS, graceMs: GRACE_MS });
    const r = await adapter.reconcile(db, site, t0);
    assert(r.status === "free" && r.downgraded === true && r.data_retained === true, "past_due → free, data retained");
    assert(db.sites.has("site_e"), "site row NOT deleted after reconcile");
    console.log("PASS  reconcile past_due → free (data retained)");
  }

  // ── createCheckoutSession with mock client ──
  {
    const db = new MemoryDb();
    const site = { site_id: "site_f", user_id: "u6", plan: "free", paid_until: null, billing_status: "free", trial_end: null };
    db.sites.set("site_f", site);

    const client = mockStripeClient();
    const adapter = new StripeBillingAdapter({ secretKey: "sk_test_x", priceId: "price_z", stripeClient: client, successUrl: "https://example.com/s", cancelUrl: "https://example.com/c" });
    const r = await adapter.createCheckoutSession(db, { site, userId: "u6", now: t0 });
    assert(r.ok === true && r.adapter === "stripe", "checkout session created");
    assert(r.url.startsWith("https://checkout.stripe.com/"), "checkout url returned");
    assert(r.session_id.startsWith("cs_test_"), "session id returned");
    assert(client._sessions[0].line_items[0].price === "price_z", "price id in line item");
    assert(client._sessions[0].client_reference_id === "site_f", "site_id in client_reference_id");
    console.log("PASS  createCheckoutSession with mock client");
  }

  // ── createCheckoutSession without priceId → error ──
  {
    const db = new MemoryDb();
    const adapter = new StripeBillingAdapter({ secretKey: "sk_test_x" });
    let threw = false;
    try {
      await adapter.createCheckoutSession(db, { site: { site_id: "x" }, userId: "u" });
    } catch (e) {
      threw = true;
      assert(String(e.message).includes("price_id"), "missing priceId error");
    }
    assert(threw, "no priceId throws");
    console.log("PASS  createCheckoutSession without priceId throws");
  }

  // ── Gmail config: null when no creds, present with creds ──
  {
    const cfg = gmailConfigFromEnv({});
    assert(cfg === null, "no Gmail creds → null");
    const cfg2 = gmailConfigFromEnv({ GMAIL_USER: "a@b.com", GMAIL_APP_PASSWORD: "pass" });
    assert(cfg2.user === "a@b.com" && cfg2.from === "a@b.com" && cfg2.host === "smtp.gmail.com" && cfg2.port === 465, "gmail config defaults");
    const cfg3 = gmailConfigFromEnv({ GMAIL_USER: "a@b.com", GMAIL_APP_PASSWORD: "pass", GMAIL_FROM: "noreply@x.com", GMAIL_SMTP_HOST: "smtp.mail.com", GMAIL_SMTP_PORT: "587" });
    assert(cfg3.from === "noreply@x.com" && cfg3.host === "smtp.mail.com" && cfg3.port === 587 && cfg3.secure === false, "gmail config overrides");
    console.log("PASS  gmailConfigFromEnv defaults + overrides");
  }

  // ── sendVerificationEmail: lab fallback (no creds) → no-op ──
  {
    const res = await sendVerificationEmail({ to: "x@y.com", rawToken: "tok123", env: {} });
    assert(res.ok === true && res.sent === false && res.fallback === "lab_noop", "lab no-op fallback");
    console.log("PASS  sendVerificationEmail lab no-op fallback");
  }

  // ── sendVerificationEmail: with injected transporter → sends ──
  {
    let sentOpts = null;
    const transporter = {
      sendMail(opts) { sentOpts = opts; return Promise.resolve({ messageId: "test" }); },
      close() {},
    };
    const env = { GMAIL_USER: "a@b.com", GMAIL_APP_PASSWORD: "pass", GR_OFFICIAL_PUBLIC_URL: "https://official.example.com" };
    const res = await sendVerificationEmail({ to: "user@example.com", rawToken: "tok456", env, transporter, publicUrl: "https://official.example.com" });
    assert(res.ok === true && res.sent === true && res.fallback === null, "email sent via transporter");
    assert(sentOpts.to === "user@example.com" && sentOpts.from === "a@b.com", "mail options correct");
    assert(sentOpts.html.includes("verify=tok456"), "verify link contains token");
    assert(sentOpts.text.includes("https://official.example.com/account?verify=tok456"), "text link correct");
    console.log("PASS  sendVerificationEmail with transporter sends + link");
  }

  // ── real Stripe SDK webhook signature verification (P1-2) ──
  // The production route uses stripe.webhooks.constructEvent(raw, sig,
  // webhookSecret). We synthesize the exact v1 HMAC header the same way
  // Stripe does, then run the parsed event through the adapter chain.
  {
    const secret = "whsec_test_only_local_signing_secret";
    const stripeClient = new Stripe("sk_test_x");
    const created = Math.floor(Date.now() / 1000);
    const payload = JSON.stringify({
      id: "evt_sig_1",
      object: "event",
      api_version: "2024-06-20",
      created,
      type: "checkout.session.completed",
      data: { object: { id: "cs_sig", client_reference_id: "site_sig", customer: "cus_sig", subscription: "sub_sig" } },
    });
    const t = String(created);
    const v1 = crypto.createHmac("sha256", secret).update(`${t}.${payload}`).digest("hex");
    const header = `t=${t},v1=${v1}`;

    // signature parses to the expected event
    const evt = stripeClient.webhooks.constructEvent(payload, header, secret);
    assert(evt.type === "checkout.session.completed" && evt.id === "evt_sig_1", "signed webhook parses via real SDK");

    // full chain: signed event → idempotent projection
    const db = new MemoryDb();
    db.sites.set("site_sig", { site_id: "site_sig", plan: "free", paid_until: null, grace_until: null, billing_status: "free", trial_end: null });
    const adapter = new StripeBillingAdapter({ secretKey: "sk_test_x", priceId: "price_z", annualUsd: 99, periodMs: YEAR_MS, graceMs: GRACE_MS });
    const r = await adapter.handleWebhookEvent(db, evt, t0);
    assert(r.processed === true && r.status === "checkout_completed", "signed event processed end-to-end");
    assert(db.stripeCustomers.get("site_sig") === "cus_sig", "signed webhook stored customer mapping");

    // tampered body rejected
    let threw = false;
    try {
      stripeClient.webhooks.constructEvent(payload.replace("site_sig", "site_evil"), header, secret);
    } catch {
      threw = true;
    }
    assert(threw, "tampered webhook body rejected");

    // expired timestamp rejected (outside tolerance)
    const tOld = String(created - 1000);
    const v1Old = crypto.createHmac("sha256", secret).update(`${tOld}.${payload}`).digest("hex");
    threw = false;
    try {
      stripeClient.webhooks.constructEvent(payload, `t=${tOld},v1=${v1Old}`, secret);
    } catch {
      threw = true;
    }
    assert(threw, "expired webhook timestamp rejected");

    // wrong secret rejected
    threw = false;
    try {
      stripeClient.webhooks.constructEvent(payload, header, "whsec_wrong_secret");
    } catch {
      threw = true;
    }
    assert(threw, "wrong webhook secret rejected");
    console.log("PASS  signed stripe webhook: parse + tamper/expiry/wrong-secret reject + processing chain");
  }

  console.log("STRIPE_UNIT_PASS");
}

main().catch((e) => {
  console.error("STRIPE_UNIT_FAIL", e);
  process.exit(1);
});
