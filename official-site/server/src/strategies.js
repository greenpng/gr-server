/**
 * Plan entitlement payloads for runtime nodes.
 * Users no longer pick analysis algorithms on the official site —
 * free vs paid only changes device precision lanes + RPA probe.
 */

export function planEntitlement(plan) {
  if (plan === "paid") {
    return {
      plan: "paid",
      rpa_enabled: true,
      device_precisions: ["dv0", "dv4", "dv5", "dv6"],
      primary_device_lane: "dv0",
      profile_cap: "advanced",
    };
  }
  return {
    plan: "free",
    rpa_enabled: false,
    device_precisions: ["dv4", "dv5", "dv6"],
    primary_device_lane: "dv4",
    profile_cap: "standard",
  };
}

/** Fixed internal projection preset (not user-configurable). Analysis-only product. */
function builtinStrategy(plan) {
  return {
    strategy_id: "observe_only",
    family: "observe_only",
    shadow_mode: true,
    response_profile_default: plan === "paid" ? "advanced" : "standard",
    note: "visitor_request_analysis_no_intercept",
  };
}

export function buildPlanPayload({ site_id, domain, plan }) {
  const ent = planEntitlement(plan);
  return {
    v: 2,
    site_id,
    domain,
    plan: ent.plan,
    profile_cap: ent.profile_cap,
    strategy_version_id: "builtin_plan@1",
    strategy: builtinStrategy(plan),
    allowed_strategy_ids: [],
    rpa_enabled: ent.rpa_enabled,
    device_precisions: ent.device_precisions,
    primary_device_lane: ent.primary_device_lane,
    product: "visitor_analysis",
    intercepts_requests: false,
    issued_at: Date.now(),
  };
}

/** @deprecated kept for DB seed compat — catalog is not user-facing anymore */
export const STRATEGY_CATALOG = [
  {
    version_id: "builtin_plan@1",
    title: "Builtin plan entitlement (not user-selectable)",
    plan_min: "free",
    body: builtinStrategy("free"),
  },
];

/** @deprecated use buildPlanPayload */
export function buildStrategyPayload(args) {
  return buildPlanPayload(args);
}
