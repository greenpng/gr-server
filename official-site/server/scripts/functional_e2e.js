#!/usr/bin/env node
/**
 * Functional E2E: account, members, domain verify, content, unified web redirects.
 */
const API = (process.env.GR_OFFICIAL_PUBLIC_URL ?? process.env.GV6_OFFICIAL_PUBLIC_URL) || "http://127.0.0.1:4101";
const WEB = (process.env.GR_MARKETING_URL ?? process.env.GV6_MARKETING_URL) || "http://127.0.0.1:3000";

async function j(method, path, { body, cookie, base = API } = {}) {
  const headers = { accept: "application/json" };
  if (body !== undefined) headers["content-type"] = "application/json";
  if (cookie) headers.cookie = cookie;
  const res = await fetch(`${base}${path}`, {
    method,
    headers,
    body: body !== undefined ? JSON.stringify(body) : undefined,
    redirect: "manual",
  });
  const setCookie = res.headers.getSetCookie?.() || [];
  const text = await res.text();
  let json = null;
  try {
    json = JSON.parse(text);
  } catch {
    json = { raw: text?.slice(0, 200) };
  }
  return { status: res.status, json, setCookie, location: res.headers.get("location"), text };
}

function cookieFrom(setCookie) {
  return setCookie.map((c) => c.split(";")[0]).join("; ");
}

let pass = 0;
let fail = 0;

function assert(name, cond, detail = "") {
  if (cond) {
    pass++;
    console.log(`PASS  ${name}${detail ? " — " + detail : ""}`);
  } else {
    fail++;
    console.error(`FAIL  ${name}${detail ? " — " + detail : ""}`);
  }
}

async function main() {
  // ── public ──
  let r = await j("GET", "/v1/health");
  assert("health", r.status === 200 && r.json.ok, r.status);

  r = await j("GET", "/v1/public-config");
  assert("public-config", r.status === 200 && r.json.config?.panel_oauth_client_id, r.json?.config?.panel_oauth_client_id);
  const authOn = r.json.config?.public_auth === true;

  r = await j("GET", "/v1/jwks");
  assert("jwks", r.status === 200 && r.json.keys?.length, "keys");

  r = await j("GET", "/v1/content/site-config");
  assert("content site-config", r.status === 200 && r.json.locales?.length >= 2);

  for (const loc of ["en", "zh-CN"]) {
    r = await j("GET", `/v1/content/page/home?locale=${encodeURIComponent(loc)}`);
    assert(`content home ${loc}`, r.status === 200 && r.json.page?.title);
    r = await j("GET", `/v1/content/by-slug/pricing?locale=${encodeURIComponent(loc)}`);
    assert(`content pricing ${loc}`, r.status === 200);
    const faq = r.json.page?.body?.blocks?.some((b) => b.type === "faq");
    assert(`pricing FAQ ${loc}`, faq, faq ? "has faq block" : "missing faq");
  }

  // ── auth ──
  if (!authOn) {
    r = await j("POST", "/v1/auth/register", { body: { email: `func_${Date.now()}@example.com`, password: "test-password-99" } });
    assert("register disabled", r.status === 403 && r.json.error === "auth_disabled", String(r.status));
    r = await j("POST", "/v1/auth/login", { body: { email: "a@b.com", password: "test-password-99" } });
    assert("login disabled", r.status === 403 && r.json.error === "auth_disabled", String(r.status));
  } else {
  const email = `func_${Date.now()}@example.com`;
  const password = "test-password-99";
  r = await j("POST", "/v1/auth/register", { body: { email, password } });
  assert("register", r.status === 200, JSON.stringify(r.json));
  let cookie = cookieFrom(r.setCookie);

  r = await j("GET", "/v1/me", { cookie });
  assert("me after register", r.status === 200 && r.json.user?.email === email);
  assert("me permissions owner", r.json.user?.permissions?.includes("members:manage"));

  r = await j("GET", "/v1/me");
  assert("me unauthenticated", r.status === 401);

  // ── sub-account ──
  r = await j("POST", "/v1/account/members", {
    cookie,
    body: { email: `sub_${Date.now()}@example.com`, password: "test-password-99", role: "editor" },
  });
  assert("add sub-account editor", r.status === 200, JSON.stringify(r.json));

  r = await j("GET", "/v1/account/members", { cookie });
  assert("list members", r.status === 200 && r.json.members?.length >= 2, String(r.json.members?.length));

  // ── sites + verify + pay ──
  const domain = `shop-${Date.now()}.gv6.local`;
  r = await j("POST", "/v1/sites", { cookie, body: { domain, name: "Func Shop" } });
  assert("add site", r.status === 200, JSON.stringify(r.json));
  const siteId = r.json.site?.site_id;

  r = await j("GET", "/v1/sites", { cookie });
  assert("list sites", r.status === 200 && r.json.sites?.some((s) => s.site_id === siteId));

  r = await j("POST", `/v1/sites/${siteId}/verify`, { cookie, body: {} });
  assert("domain verify unverified shape", r.status === 200 && r.json.instruction?.dns_txt, r.json?.error);

  r = await j("POST", `/v1/sites/${siteId}/pay`, { cookie, body: {} });
  assert("mock pay", r.status === 200 && r.json.plan === "paid");

  // ── email verify (lab) ──
  if ((process.env.GR_OFFICIAL_LAB_EMAIL ?? process.env.GV6_OFFICIAL_LAB_EMAIL) === "1") {
    r = await j("POST", "/v1/account/email/request-verify", { cookie, body: {} });
    assert("email verify request", r.status === 200 && r.json.lab_token, "");
    r = await j("POST", "/v1/account/email/confirm", { cookie, body: { token: r.json.lab_token } });
    assert("email verify confirm", r.status === 200 && r.json.email_verified);
  } else {
    console.log("SKIP  email verify lab_token (set GV6_OFFICIAL_LAB_EMAIL=1)");
  }

  // ── OAuth redirect when logged out ──
  r = await j(
    "GET",
    "/oauth/authorize?response_type=code&client_id=gr-admin-panel&redirect_uri=http%3A%2F%2F127.0.0.1%3A28680%2Foauth%2Fcallback&code_challenge=abc&code_challenge_method=S256"
  );
  assert("oauth redirect to login when anonymous", r.status === 302 && r.location?.includes("account"), r.location);

  r = await j("POST", "/v1/auth/logout", { cookie });
  assert("logout", r.status === 200);
  r = await j("GET", "/v1/me", { cookie });
  assert("me after logout", r.status === 401);
  }

  // ── unified web redirects (API) ──
  r = await j("GET", "/login");
  if (r.json?.config?.unified_web || (process.env.GR_OFFICIAL_WEB_ORIGIN ?? process.env.GV6_OFFICIAL_WEB_ORIGIN)) {
    assert("API /login redirect", r.status === 302, String(r.status));
  }

  // ── marketing site (optional) ──
  try {
    r = await j("GET", "/", { base: WEB });
    if (r.status === 0 || r.text === undefined) throw new Error("down");
    assert("marketing /", r.status === 200 && /greenpng/i.test(r.text), r.status);
    r = await j("GET", "/pricing", { base: WEB });
    assert("marketing /pricing", r.status === 200 && /pricing|FAQ|faq/i.test(r.text), r.status);
    r = await j("GET", "/login", { base: WEB });
    assert("marketing /login", r.status === 200 && /Sign in|登录/.test(r.text), r.status);
    r = await j("GET", "/docs", { base: WEB });
    assert("marketing /docs", r.status === 200 && /Documentation|使用教程/.test(r.text), r.status);
    r = await j("GET", "/v1/public-config", { base: WEB });
    assert("marketing proxy /v1", r.status === 200 && r.json.config, r.status);
  } catch (e) {
    console.log("SKIP  marketing site (:3000 not running) — start official-web for full UI test");
  }

  console.log(`\n== functional: ${pass} passed, ${fail} failed ==`);
  if (fail) process.exit(1);
  console.log("FUNCTIONAL_E2E_PASS");
}

main().catch((e) => {
  console.error("FUNCTIONAL_E2E_FAIL", e);
  process.exit(1);
});
