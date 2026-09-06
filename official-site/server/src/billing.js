/**
 * BillingAdapter + subscription state machine.
 *
 * Confirmed product rules (iss commercial report, P0 subset):
 * - states: trial | active | grace | past_due | free
 * - grace (~14d after paid_until) still receives paid entitlements
 * - after grace: downgrade to free entitlements; never delete site or probe data
 * - mock pay stays behind this adapter (Stripe/other adapters can replace later)
 *
 * Production path: StripeBillingAdapter (Stripe Checkout + webhooks).
 * Lab path: MockBillingAdapter (no card network).
 */

import { randomToken } from "./crypto_util.js";
import { planEntitlement } from "./strategies.js";

export const GRACE_MS = 14 * 24 * 3600 * 1000;
export const YEAR_MS = 365 * 24 * 3600 * 1000;
export const ANNUAL_USD = 99;

export function trialMsFromEnv(env = process.env) {
  const raw = env.GV6_TRIAL_DAYS;
  if (raw == null || raw === "") return 0;
  const days = Number(raw);
  if (!Number.isFinite(days) || days <= 0) return 0;
  return Math.floor(days * 24 * 3600 * 1000);
}

/**
 * Pure snapshot. Does not mutate the site row.
 * @param {object} site
 * @param {number} [at]
 */
export function resolveBilling(site, at = Date.now()) {
  const plan = String(site?.plan || "free");
  const paidUntil = site?.paid_until != null ? Number(site.paid_until) : 0;
  const trialEnd = site?.trial_end != null ? Number(site.trial_end) : 0;
  const graceUntilStored = site?.grace_until != null ? Number(site.grace_until) : 0;

  if (plan === "paid" && paidUntil > at) {
    return {
      status: "active",
      entitlement_plan: "paid",
      paid_until: paidUntil,
      grace_until: null,
      trial_end: trialEnd || null,
      data_retained: true,
    };
  }

  if (plan === "paid" && paidUntil > 0 && paidUntil <= at) {
    const graceUntil = graceUntilStored > 0 ? graceUntilStored : paidUntil + GRACE_MS;
    if (graceUntil > at) {
      return {
        status: "grace",
        entitlement_plan: "paid",
        paid_until: paidUntil,
        grace_until: graceUntil,
        trial_end: trialEnd || null,
        data_retained: true,
      };
    }
    return {
      status: "past_due",
      entitlement_plan: "free",
      paid_until: paidUntil,
      grace_until: graceUntil,
      trial_end: trialEnd || null,
      data_retained: true,
    };
  }

  if (trialEnd > at) {
    return {
      status: "trial",
      entitlement_plan: "paid",
      paid_until: paidUntil || null,
      grace_until: null,
      trial_end: trialEnd,
      data_retained: true,
    };
  }

  return {
    status: "free",
    entitlement_plan: "free",
    paid_until: paidUntil || null,
    grace_until: null,
    trial_end: trialEnd || null,
    data_retained: true,
  };
}

/** Entitlement lane: paid during trial/active/grace; free after grace. */
export function planActive(site, at = Date.now()) {
  return resolveBilling(site, at).entitlement_plan;
}

export function billingPublicView(site, at = Date.now()) {
  const snap = resolveBilling(site, at);
  const ent = planEntitlement(snap.entitlement_plan);
  return {
    ...snap,
    plan: snap.entitlement_plan,
    billing_status: snap.status,
    rpa_enabled: ent.rpa_enabled,
    device_precisions: ent.device_precisions,
    primary_device_lane: ent.primary_device_lane,
    profile_cap: ent.profile_cap,
    price_usd_per_year: ANNUAL_USD,
  };
}

export class MockBillingAdapter {
  constructor(opts = {}) {
    this.annualUsd = opts.annualUsd ?? ANNUAL_USD;
    this.periodMs = opts.periodMs ?? YEAR_MS;
    this.graceMs = opts.graceMs ?? GRACE_MS;
  }

  /**
   * Idempotent annual payment: extend from max(now, paid_until).
   * Mock only — not a card network.
   */
  async payAnnual(db, { site, userId, now = Date.now(), note = "mock_annual" }) {
    const currentUntil = site?.paid_until != null ? Number(site.paid_until) : 0;
    const base = currentUntil > now ? currentUntil : now;
    const until = base + this.periodMs;
    await db.run(
      "UPDATE sites SET plan='paid', paid_until=?, grace_until=NULL, billing_status='active', trial_end=NULL WHERE site_id=?",
      until,
      site.site_id
    );
    await db.run(
      "INSERT INTO payments (id, site_id, user_id, amount_usd, created_at, note) VALUES (?, ?, ?, ?, ?, ?)",
      randomToken(12),
      site.site_id,
      userId,
      this.annualUsd,
      now,
      note
    );
    return {
      ok: true,
      adapter: "mock",
      plan: "paid",
      status: "active",
      paid_until: until,
      amount_usd: this.annualUsd,
      data_retained: true,
    };
  }

  /** Payment failed while still on a paid contract → grace, keep data. */
  async markPaymentFailed(db, { site, now = Date.now() }) {
    const paidUntil = Math.min(Number(site.paid_until) || now, now);
    const graceUntil = Math.max(Number(site.grace_until) || 0, now + this.graceMs);
    await db.run(
      "UPDATE sites SET plan='paid', paid_until=?, grace_until=?, billing_status='grace' WHERE site_id=?",
      paidUntil,
      graceUntil,
      site.site_id
    );
    return resolveBilling(
      { ...site, plan: "paid", paid_until: paidUntil, grace_until: graceUntil },
      now
    );
  }

  /**
   * Apply past_due → free. Never deletes sites, payments, or probe rows.
   */
  async reconcile(db, site, now = Date.now()) {
    const snap = resolveBilling(site, now);
    if (snap.status === "past_due") {
      await db.run(
        "UPDATE sites SET plan='free', billing_status='free' WHERE site_id=?",
        site.site_id
      );
      return {
        ...snap,
        status: "free",
        entitlement_plan: "free",
        downgraded: true,
        data_retained: true,
      };
    }
    if (String(site.billing_status || "") !== snap.status) {
      await db.run(
        "UPDATE sites SET billing_status=? WHERE site_id=?",
        snap.status,
        site.site_id
      );
    }
    return { ...snap, downgraded: false };
  }
}

/**
 * Stripe configuration from environment. Never hardcodes secrets.
 * Returns null when STRIPE_SECRET_KEY is absent (lab / mock path).
 */
export function stripeConfigFromEnv(env = process.env) {
  const secretKey = env.STRIPE_SECRET_KEY || env.GV6_STRIPE_SECRET_KEY || "";
  if (!secretKey) return null;
  return {
    secretKey,
    webhookSecret: env.STRIPE_WEBHOOK_SECRET || env.GV6_STRIPE_WEBHOOK_SECRET || "",
    priceId: env.STRIPE_PRICE_ID || env.GV6_STRIPE_PRICE_ID || "",
    successUrl: env.STRIPE_SUCCESS_URL || env.GV6_STRIPE_SUCCESS_URL || "",
    cancelUrl: env.STRIPE_CANCEL_URL || env.GV6_STRIPE_CANCEL_URL || "",
    annualUsd: Number(env.STRIPE_ANNUAL_USD || ANNUAL_USD),
    periodMs: Number(env.STRIPE_PERIOD_MS || YEAR_MS),
    graceMs: Number(env.STRIPE_GRACE_MS || GRACE_MS),
  };
}

/**
 * Stripe Checkout + webhook billing adapter.
 *
 * - createCheckoutSession: starts a Stripe Checkout Session for the annual plan.
 * - handleWebhookEvent: idempotent event storage + state reconciliation.
 * - reconcile: paid/grace/past_due/free driven by stored subscription status.
 *
 * The Stripe SDK client is lazily imported so lab environments without the
 * `stripe` package (or without STRIPE_SECRET_KEY) never touch the network.
 * A `stripeClient` may be injected for unit testing.
 */
export class StripeBillingAdapter {
  constructor(opts = {}) {
    this.annualUsd = opts.annualUsd ?? ANNUAL_USD;
    this.periodMs = opts.periodMs ?? YEAR_MS;
    this.graceMs = opts.graceMs ?? GRACE_MS;
    this.priceId = opts.priceId || "";
    this.successUrl = opts.successUrl || "";
    this.cancelUrl = opts.cancelUrl || "";
    this.webhookSecret = opts.webhookSecret || "";
    this._stripeClient = opts.stripeClient || null;
    this._secretKey = opts.secretKey || "";
  }

  /** Lazily load the Stripe SDK (async dynamic import). Throws if not configured. */
  async _client() {
    if (this._stripeClient) return this._stripeClient;
    if (!this._secretKey) throw new Error("stripe_not_configured");
    // Lazy dynamic import keeps lab (no stripe pkg / no key) working.
    const mod = await import("stripe");
    const Stripe = mod.default || mod.Stripe || mod;
    if (typeof Stripe !== "function") throw new Error("stripe_sdk_unavailable");
    this._stripeClient = Stripe(this._secretKey, {
      appInfo: { name: "gv6-official-site", version: "1" },
    });
    return this._stripeClient;
  }

  /** Resolve the site row for a Stripe customer/subscription id. */
  async _siteByCustomer(db, customerId) {
    return db.get(
      "SELECT s.* FROM sites s JOIN stripe_customers sc ON sc.site_id=s.site_id WHERE sc.customer_id=?",
      customerId
    );
  }

  /**
   * Start a Stripe Checkout Session for the annual plan.
   * Stores the customer mapping once the session is created (customer is
   * materialized by Stripe on session completion; we store session_id as a
   * placeholder and update on webhook).
   */
  async createCheckoutSession(db, { site, userId, now = Date.now() }) {
    if (!this.priceId) throw new Error("stripe_price_id_missing");
    const stripe = await this._client();
    const session = await stripe.checkout.sessions.create({
      mode: "subscription",
      line_items: [{ price: this.priceId, quantity: 1 }],
      client_reference_id: site.site_id,
      customer_email: undefined,
      subscription_data: { metadata: { site_id: site.site_id, user_id: userId } },
      success_url: this.successUrl || undefined,
      cancel_url: this.cancelUrl || undefined,
      metadata: { site_id: site.site_id, user_id: userId },
    });
    return {
      ok: true,
      adapter: "stripe",
      url: session.url,
      session_id: session.id,
    };
  }

  /**
   * Idempotently store + process a Stripe webhook event.
   * Returns { stored: bool, processed: bool, status }.
   * Replays of the same event_id are no-ops (stored=false).
   */
  async handleWebhookEvent(db, event, now = Date.now()) {
    const eventId = String(event?.id || "");
    const type = String(event?.type || "");
    if (!eventId) throw new Error("stripe_event_missing_id");
    // Claim the event first so failed attempts and their metadata survive the
    // projection transaction rollback.
    let stored = true;
    try {
      await db.run(
        "INSERT INTO stripe_events (event_id, type, created_at, processed_at, payload_json, attempts, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
        eventId,
        type,
        now,
        null,
        JSON.stringify(event), 1, now
      );
    } catch (e) {
      // A duplicate that was fully processed is a no-op. If the previous
      // attempt failed, leave it retryable instead of losing the event.
      const existing = await db.get(
        "SELECT processed_at FROM stripe_events WHERE event_id=?",
        eventId
      );
      if (!existing) throw e;
      if (existing.processed_at != null) {
        return { stored: false, processed: false, status: "duplicate", type };
      }
      stored = false;
      await db.run("UPDATE stripe_events SET attempts=attempts+1, updated_at=?, last_error=NULL WHERE event_id=?", now, eventId);
    }
    try {
      // Business projection and completion marker are atomic.
      return await db.transaction(async (tx) => {
      const obj = event?.data?.object || {};
      let status = "ignored";
      switch (type) {
        case "checkout.session.completed": {
          const siteId = obj.client_reference_id || obj.metadata?.site_id;
          const customerId = obj.customer;
          const subId = obj.subscription;
          if (siteId && customerId) {
            await tx.run(
              "INSERT INTO stripe_customers (site_id, customer_id) VALUES (?, ?) ON CONFLICT(site_id) DO UPDATE SET customer_id=excluded.customer_id",
              siteId,
              customerId
            );
          }
          if (siteId && subId) {
            await tx.run(
              "INSERT INTO stripe_subscriptions (site_id, subscription_id, status, current_period_end) VALUES (?, ?, ?, ?) ON CONFLICT(site_id) DO UPDATE SET subscription_id=excluded.subscription_id, status=excluded.status, current_period_end=excluded.current_period_end",
              siteId,
              subId,
              "active",
              0
            );
          }
          status = "checkout_completed";
          break;
        }
        case "invoice.paid":
        case "invoice.payment_succeeded": {
          const subId = obj.subscription || obj.parent_subscription;
          const customerId = obj.customer;
          const periodEnd = Number(obj.lines?.data?.[0]?.period?.end || obj.current_period_end || 0) * 1000;
          const site = await this._siteByCustomer(tx, customerId);
          if (site) {
            const until = periodEnd > 0 ? periodEnd : now + this.periodMs;
            await tx.run(
              "UPDATE sites SET plan='paid', paid_until=?, grace_until=NULL, billing_status='active', trial_end=NULL WHERE site_id=?",
              until,
              site.site_id
            );
            const paymentNote = `stripe_invoice_paid:${eventId}`;
            const payment = await tx.get(
              "SELECT id FROM payments WHERE site_id=? AND note=? LIMIT 1",
              site.site_id,
              paymentNote
            );
            if (!payment) {
              await tx.run(
                "INSERT INTO payments (id, site_id, user_id, amount_usd, created_at, note) VALUES (?, ?, ?, ?, ?, ?)",
                randomToken(12),
                site.site_id,
                site.user_id,
                this.annualUsd,
                now,
                paymentNote
              );
            }
            if (subId) {
              await tx.run(
                "UPDATE stripe_subscriptions SET status='active', current_period_end=? WHERE site_id=?",
                until,
                site.site_id
              );
            }
            status = "paid";
          }
          break;
        }
        case "invoice.payment_failed": {
          const customerId = obj.customer;
          const site = await this._siteByCustomer(tx, customerId);
          if (site) {
            const snap = await this.markPaymentFailed(tx, { site, now });
            status = snap.status;
          }
          break;
        }
        case "customer.subscription.updated": {
          const subId = obj.id;
          const customerId = obj.customer;
          const statusVal = String(obj.status || "");
          const periodEnd = Number(obj.current_period_end || 0) * 1000;
          const site = await this._siteByCustomer(tx, customerId);
          if (site) {
            await tx.run(
              "UPDATE stripe_subscriptions SET status=?, current_period_end=? WHERE site_id=?",
              statusVal,
              periodEnd,
              site.site_id
            );
            if (["active", "trialing"].includes(statusVal) && periodEnd > 0) {
              await tx.run(
                "UPDATE sites SET plan='paid', paid_until=?, grace_until=NULL, billing_status='active', trial_end=NULL WHERE site_id=?",
                periodEnd,
                site.site_id
              );
              status = "active";
            } else if (["past_due", "unpaid"].includes(statusVal)) {
              const snap = await this.markPaymentFailed(tx, { site, now });
              status = snap.status;
            } else if (["canceled", "cancelled", "expired"].includes(statusVal)) {
              await tx.run(
                "UPDATE sites SET plan='free', billing_status='free' WHERE site_id=?",
                site.site_id
              );
              status = "free";
            }
          }
          break;
        }
        case "customer.subscription.deleted": {
          const customerId = obj.customer;
          const site = await this._siteByCustomer(tx, customerId);
          if (site) {
            await tx.run(
              "UPDATE sites SET plan='free', billing_status='free' WHERE site_id=?",
              site.site_id
            );
            await tx.run(
              "UPDATE stripe_subscriptions SET status='deleted' WHERE site_id=?",
              site.site_id
            );
            status = "free";
          }
          break;
        }
        default:
          status = "ignored";
      }
      await tx.run("UPDATE stripe_events SET processed_at=?, last_error=NULL, updated_at=? WHERE event_id=?", now, now, eventId);
      return { stored, processed: true, status, type };
      });
    } catch (e) {
      await db.run("UPDATE stripe_events SET last_error=?, updated_at=? WHERE event_id=?", String(e.message || e).slice(0, 500), now, eventId);
      throw e;
    }
  }

  /** Payment failed while still on a paid contract → grace, keep data. */
  async markPaymentFailed(db, { site, now = Date.now() }) {
    const paidUntil = Math.min(Number(site.paid_until) || now, now);
    const graceUntil = Math.max(Number(site.grace_until) || 0, now + this.graceMs);
    await db.run(
      "UPDATE sites SET plan='paid', paid_until=?, grace_until=?, billing_status='grace' WHERE site_id=?",
      paidUntil,
      graceUntil,
      site.site_id
    );
    return resolveBilling(
      { ...site, plan: "paid", paid_until: paidUntil, grace_until: graceUntil },
      now
    );
  }

  /**
   * Apply past_due → free. Never deletes sites, payments, or probe rows.
   * Same state machine as MockBillingAdapter.
   */
  async reconcile(db, site, now = Date.now()) {
    const snap = resolveBilling(site, now);
    if (snap.status === "past_due") {
      await db.run(
        "UPDATE sites SET plan='free', billing_status='free' WHERE site_id=?",
        site.site_id
      );
      return {
        ...snap,
        status: "free",
        entitlement_plan: "free",
        downgraded: true,
        data_retained: true,
      };
    }
    if (String(site.billing_status || "") !== snap.status) {
      await db.run(
        "UPDATE sites SET billing_status=? WHERE site_id=?",
        snap.status,
        site.site_id
      );
    }
    return { ...snap, downgraded: false };
  }
}

/**
 * Select the production billing adapter.
 * - Stripe when STRIPE_SECRET_KEY is configured (production).
 * - Mock otherwise (lab / tests).
 * The Stripe SDK is loaded lazily only when a key is present.
 */
export function selectBillingAdapter(env = process.env) {
  const cfg = stripeConfigFromEnv(env);
  if (cfg) {
    return new StripeBillingAdapter(cfg);
  }
  return new MockBillingAdapter();
}

export const defaultBillingAdapter = selectBillingAdapter();
