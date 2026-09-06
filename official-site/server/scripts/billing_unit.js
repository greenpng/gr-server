/**
 * Unit: BillingAdapter state machine (no HTTP / no Postgres).
 */
import {
  GRACE_MS,
  YEAR_MS,
  resolveBilling,
  planActive,
  MockBillingAdapter,
} from "../src/billing.js";

function assert(cond, msg) {
  if (!cond) throw new Error(msg);
}

class MemoryDb {
  constructor(site) {
    this.site = { ...site };
    this.payments = [];
    this.deleted = false;
  }
  async run(sql, ...params) {
    if (/DELETE FROM sites/i.test(sql)) this.deleted = true;
    if (/UPDATE sites SET plan='paid', paid_until=\?, grace_until=NULL/.test(sql)) {
      this.site.plan = "paid";
      this.site.paid_until = params[0];
      this.site.grace_until = null;
      this.site.billing_status = "active";
      this.site.trial_end = null;
    } else if (/UPDATE sites SET plan='paid', paid_until=\?, grace_until=\?/.test(sql)) {
      this.site.plan = "paid";
      this.site.paid_until = params[0];
      this.site.grace_until = params[1];
      this.site.billing_status = "grace";
    } else if (/UPDATE sites SET plan='free', billing_status='free'/.test(sql)) {
      this.site.plan = "free";
      this.site.billing_status = "free";
    } else if (/UPDATE sites SET billing_status=\?/.test(sql)) {
      this.site.billing_status = params[0];
    } else if (/INSERT INTO payments/.test(sql)) {
      this.payments.push({ id: params[0], amount: params[3], note: params[5] });
    }
    return { changes: 1 };
  }
}

const t0 = 1_700_000_000_000;

const free = resolveBilling({ plan: "free" }, t0);
assert(free.status === "free" && free.entitlement_plan === "free", "free");

const trial = resolveBilling({ plan: "free", trial_end: t0 + 86400000 }, t0);
assert(trial.status === "trial" && trial.entitlement_plan === "paid", "trial still paid lanes");
assert(planActive({ plan: "free", trial_end: t0 + 86400000 }, t0) === "paid", "planActive trial");

const active = resolveBilling({ plan: "paid", paid_until: t0 + YEAR_MS }, t0);
assert(active.status === "active" && active.entitlement_plan === "paid", "active");

const inGrace = resolveBilling({ plan: "paid", paid_until: t0 - 1000 }, t0);
assert(inGrace.status === "grace" && inGrace.entitlement_plan === "paid", "grace keeps paid");
assert(inGrace.grace_until === t0 - 1000 + GRACE_MS, "grace window");

const past = resolveBilling({ plan: "paid", paid_until: t0 - GRACE_MS - 1 }, t0);
assert(past.status === "past_due" && past.entitlement_plan === "free", "past_due free lanes");
assert(past.data_retained === true, "past_due retains data");

const adapter = new MockBillingAdapter();
const site = { site_id: "site_lab", plan: "free", paid_until: null };
const db = new MemoryDb(site);
const pay = await adapter.payAnnual(db, { site, userId: "u1", now: t0 });
assert(pay.status === "active" && pay.paid_until === t0 + YEAR_MS, "mock pay");
assert(db.payments.length === 1, "payment row");
assert(db.deleted === false, "pay does not delete");

const failed = await adapter.markPaymentFailed(db, { site: db.site, now: t0 + YEAR_MS });
assert(failed.status === "grace" && failed.entitlement_plan === "paid", "fail → grace");

const afterGrace = await adapter.reconcile(db, db.site, t0 + YEAR_MS + GRACE_MS + 1);
assert(afterGrace.status === "free" && afterGrace.downgraded === true, "grace expired → free");
assert(afterGrace.data_retained === true, "downgrade retains data");
assert(db.deleted === false, "reconcile never deletes site");
assert(db.site.plan === "free", "row plan free after grace");

console.log("BILLING_UNIT_PASS");
