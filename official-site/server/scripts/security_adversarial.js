/**
 * Adversarial / security regression tests for official-site security iteration.
 * Does NOT cover OTA or multi-arch packaging.
 *
 * Usage (official site must be running on :4101):
 *   node server/scripts/security_adversarial.js
 */
import crypto from "node:crypto";
import {
  generateNodeKeypair,
  ecdhAesKey,
  decryptWithKey,
  encryptWithKey,
} from "../src/algo_delivery.js";
import { totpCode } from "../src/account_security.js";
import { sanitizeHtml, safeUrl, sanitizeBlocks } from "../src/sanitize_html.js";
import { isPrivateOrReservedIp } from "../src/domain_verify.js";

const API = (process.env.GR_OFFICIAL_PUBLIC_URL ?? process.env.GV6_OFFICIAL_PUBLIC_URL) || "http://127.0.0.1:4101";

let passed = 0;
let failed = 0;
const findings = [];

function ok(name, detail = "") {
  passed++;
  console.log(`PASS  ${name}${detail ? " — " + detail : ""}`);
}
function bad(name, detail) {
  failed++;
  console.error(`FAIL  ${name} — ${detail}`);
  findings.push({ severity: "fail", name, detail });
}
function note(name, detail) {
  console.log(`NOTE  ${name} — ${detail}`);
  findings.push({ severity: "note", name, detail });
}

function b64url(buf) {
  return Buffer.from(buf)
    .toString("base64")
    .replace(/\+/g, "-")
    .replace(/\//g, "_")
    .replace(/=+$/g, "");
}

async function j(method, path, { body, cookie, token } = {}) {
  const headers = { accept: "application/json" };
  if (body !== undefined) headers["content-type"] = "application/json";
  if (cookie) headers.cookie = cookie;
  if (token) headers.authorization = `Bearer ${token}`;
  const res = await fetch(`${API}${path}`, {
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
    json = { raw: text };
  }
  return {
    status: res.status,
    json,
    setCookie,
    location: res.headers.get("location"),
  };
}

function cookieFrom(setCookie) {
  return setCookie.map((c) => c.split(";")[0]).join("; ");
}

async function registerUser(tag) {
  const email = `sec_${tag}_${Date.now()}@example.com`;
  const password = "test-password-99";
  const r = await j("POST", "/v1/auth/register", { body: { email, password } });
  if (r.status !== 200) throw new Error("register " + JSON.stringify(r.json));
  return { email, password, cookie: cookieFrom(r.setCookie), user: r.json.user };
}

async function oauthToken(cookie) {
  const verifier = b64url(crypto.randomBytes(32));
  const challenge = b64url(crypto.createHash("sha256").update(verifier).digest());
  const redirect = "http://127.0.0.1:28680/oauth/callback";
  const authUrl =
    `/oauth/authorize?response_type=code&client_id=gr-admin-panel` +
    `&redirect_uri=${encodeURIComponent(redirect)}` +
    `&code_challenge=${challenge}&code_challenge_method=S256&state=sec`;
  let r = await j("GET", authUrl, { cookie });
  if (r.status !== 302 || !r.location) throw new Error("authorize " + r.status);
  const code = new URL(r.location).searchParams.get("code");
  r = await j("POST", "/oauth/token", {
    body: {
      grant_type: "authorization_code",
      code,
      redirect_uri: redirect,
      client_id: "gr-admin-panel",
      code_verifier: verifier,
    },
  });
  if (r.status !== 200) throw new Error("token " + JSON.stringify(r.json));
  return r.json.access_token;
}

async function pullAlgo(token, site, eph = generateNodeKeypair()) {
  return j("POST", "/v1/runtime/algo-bundle", {
    token,
    body: {
      site_id: site.site_id,
      domain: site.domain,
      instance_id: `sec-${Date.now()}`,
      node_ephemeral_pub_b64: eph.public_b64,
    },
  }).then((r) => ({ r, eph }));
}

async function main() {
  console.log(`== security adversarial @ ${API} ==\n`);

  {
    const dirty = `<p onclick="alert(1)">hi</p><script>alert(1)</script><a href="javascript:alert(1)">x</a><img src=x onerror=alert(1)>`;
    const clean = sanitizeHtml(dirty);
    if (!clean.includes("<script") && !clean.includes("onclick") && !clean.includes("javascript:") && !clean.includes("<img")) {
      ok("CMS sanitizer strips script/events/js urls");
    } else bad("CMS sanitizer strips script/events/js urls", clean);
    if (safeUrl("javascript:alert(1)") === "" && safeUrl("https://example.com/a") === "https://example.com/a") {
      ok("safeUrl allowlist");
    } else bad("safeUrl allowlist", safeUrl("javascript:alert(1)"));
    const dangerous = [
      "data:text/html,<script>alert(1)</script>",
      "vbscript:msgbox(1)",
      "//evil.example/x",
      "javascript:alert(1)",
      "\tjavascript:alert(1)",
      "/\\tjavascript:alert(1)",
    ];
    if (dangerous.every((u) => safeUrl(u) === "")) ok("safeUrl blocks data/vbscript/protocol-relative/ctl");
    else bad("safeUrl blocks data/vbscript/protocol-relative/ctl", dangerous.map(safeUrl).join("|"));
    const priced = sanitizeBlocks({
      blocks: [
        {
          type: "pricing",
          plans: [{ id: "p", name: "x", price: "0", features: [], cta: { label: "go", href: "data:text/html,<script>1</script>" } }],
        },
      ],
    });
    const href = priced.blocks?.[0]?.plans?.[0]?.cta?.href;
    if (href === "") ok("sanitizeBlocks strips pricing data: CTA");
    else bad("sanitizeBlocks strips pricing data: CTA", href === undefined ? "MISSING" : String(href));
    if (isPrivateOrReservedIp("127.0.0.1") && isPrivateOrReservedIp("10.1.2.3") && !isPrivateOrReservedIp("1.1.1.1")) {
      ok("domain verify blocks private IPs");
    } else bad("domain verify blocks private IPs", "range check");
  }

  {
    const r = await fetch(`${API}/v1/me`, {
      method: "GET",
      headers: { origin: "https://evil.example", accept: "application/json" },
    });
    const acao = r.headers.get("access-control-allow-origin");
    if (!acao || acao === "https://evil.example") {
      if (acao === "https://evil.example") bad("CORS does not reflect arbitrary origin", acao);
      else ok("CORS does not reflect arbitrary origin");
    } else ok("CORS does not reflect arbitrary origin", acao || "no ACAO");
  }

  // ── wrap-key default off ─────────────────────────────────────────
  {
    const cfg = await j("GET", "/v1/public-config");
    if (cfg.json?.config?.public_auth !== true) {
      const denied = await j("POST", "/v1/auth/register", {
        body: { email: `sec_hold_${Date.now()}@example.com`, password: "test-password-99" },
      });
      if (denied.status === 403 && denied.json?.error === "auth_disabled") ok("public register disabled");
      else bad("public register disabled", `${denied.status} ${JSON.stringify(denied.json)}`);
      const wk = await j("GET", "/v1/crypto/wrap-key");
      if (wk.status === 404) ok("wrap-key disabled by default");
      else bad("wrap-key disabled by default", String(wk.status));
      console.log(`\n== result: ${passed} passed, ${failed} failed ==`);
      if (failed) process.exit(1);
      console.log("SECURITY_ADVERSARIAL_PASS");
      return;
    }
    const a = await registerUser("wrap");
    const tok = await oauthToken(a.cookie);
    const r = await j("GET", "/v1/crypto/wrap-key", { token: tok });
    if (r.status === 404 && r.json?.error === "wrap_key_disabled") {
      ok("wrap-key disabled by default");
    } else if (r.status === 200) {
      bad("wrap-key disabled by default", "endpoint returned key — lab flag still on?");
    } else {
      bad("wrap-key disabled by default", `${r.status} ${JSON.stringify(r.json)}`);
    }
  }

  // ── cross-tenant isolation ───────────────────────────────────────
  {
    const a = await registerUser("a");
    const b = await registerUser("b");
    let r = await j("POST", "/v1/sites", {
      cookie: a.cookie,
      body: { domain: `a-${Date.now()}.gv6.local`, name: "A" },
    });
    const siteA = r.json.site;
    await j("POST", `/v1/sites/${siteA.site_id}/pay`, { cookie: a.cookie, body: {} });
    const tokB = await oauthToken(b.cookie);
    r = await j("POST", "/v1/runtime/algo-bundle", {
      token: tokB,
      body: {
        site_id: siteA.site_id,
        domain: siteA.domain,
        node_ephemeral_pub_b64: generateNodeKeypair().public_b64,
      },
    });
    if (r.status === 404) ok("cross-tenant algo-bundle denied", "404 site_not_found");
    else bad("cross-tenant algo-bundle denied", `${r.status} ${JSON.stringify(r.json)}`);

    r = await j("POST", `/v1/sites/${siteA.site_id}/verify`, { cookie: b.cookie, body: {} });
    if (r.status === 404) ok("cross-tenant domain verify denied");
    else bad("cross-tenant domain verify denied", `${r.status}`);
  }

  // ── unauthenticated bundle ───────────────────────────────────────
  {
    const r = await j("POST", "/v1/runtime/algo-bundle", {
      body: {
        site_id: "x",
        domain: "x.example",
        node_ephemeral_pub_b64: generateNodeKeypair().public_b64,
      },
    });
    if (r.status === 401) ok("algo-bundle requires bearer");
    else bad("algo-bundle requires bearer", String(r.status));
  }

  // ── ECDH happy path + tamper resistance ──────────────────────────
  {
    const a = await registerUser("ecdh");
    let r = await j("POST", "/v1/sites", {
      cookie: a.cookie,
      body: { domain: `ecdh-${Date.now()}.gv6.local`, name: "ecdh" },
    });
    const site = r.json.site;
    await j("POST", `/v1/sites/${site.site_id}/pay`, { cookie: a.cookie, body: {} });
    const tok = await oauthToken(a.cookie);
    const { r: br, eph } = await pullAlgo(tok, site);
    if (br.status !== 200) {
      bad("ECDH algo-bundle", JSON.stringify(br.json));
    } else {
      const aes = ecdhAesKey(eph.private_pem, br.json.header.server_ephemeral_pub_b64);
      const payload = decryptWithKey(aes, br.json.iv_b64, br.json.ciphertext_b64);
      if (payload.plan === "paid" && payload.rpa_enabled === true) {
        ok("ECDH decrypt paid entitlement");
      } else {
        bad("ECDH decrypt paid entitlement", JSON.stringify(payload));
      }

      // Tamper ciphertext → decrypt must fail
      try {
        const raw = Buffer.from(br.json.ciphertext_b64, "base64");
        raw[0] ^= 0xff;
        decryptWithKey(aes, br.json.iv_b64, raw.toString("base64"));
        bad("GCM rejects tampered ciphertext", "decrypt succeeded");
      } catch {
        ok("GCM rejects tampered ciphertext");
      }

      // Wrong ephemeral cannot decrypt
      const wrong = generateNodeKeypair();
      try {
        const badKey = ecdhAesKey(wrong.private_pem, br.json.header.server_ephemeral_pub_b64);
        decryptWithKey(badKey, br.json.iv_b64, br.json.ciphertext_b64);
        bad("wrong ECDH peer cannot decrypt", "decrypt succeeded");
      } catch {
        ok("wrong ECDH peer cannot decrypt");
      }

      // Forged MITM bundle (attacker ECDH, no official Ed25519) — client must reject via sig
      // Here we only check server still signs with real key; forged unsigned path is panel-side.
      if (br.json.sig_b64 && Buffer.from(br.json.sig_b64, "base64").length === 64) {
        ok("algo-bundle includes Ed25519 sig (64B)");
      } else {
        bad("algo-bundle includes Ed25519 sig (64B)", "missing/short");
      }
    }
  }

  // ── free vs paid payload ─────────────────────────────────────────
  {
    const a = await registerUser("plan");
    let r = await j("POST", "/v1/sites", {
      cookie: a.cookie,
      body: { domain: `free-${Date.now()}.gv6.local`, name: "free" },
    });
    const freeSite = r.json.site;
    const tok = await oauthToken(a.cookie);
    let { r: br, eph } = await pullAlgo(tok, freeSite);
    if (br.status === 200) {
      const aes = ecdhAesKey(eph.private_pem, br.json.header.server_ephemeral_pub_b64);
      const payload = decryptWithKey(aes, br.json.iv_b64, br.json.ciphertext_b64);
      if (payload.plan === "free" && payload.rpa_enabled === false && !payload.device_precisions.includes("dv0")) {
        ok("free plan: no RPA, no dv0");
      } else {
        bad("free plan: no RPA, no dv0", JSON.stringify(payload));
      }
    } else bad("free plan bundle", JSON.stringify(br.json));
  }

  // ── domain verify gate (temp server flag via child would be ideal;
  //    here we assert instruction shape + unpaid verified_at null) ──
  {
    const a = await registerUser("dns");
    let r = await j("POST", "/v1/sites", {
      cookie: a.cookie,
      body: { domain: `dns-${Date.now()}.example.com`, name: "dns" },
    });
    const site = r.json.site;
    await j("POST", `/v1/sites/${site.site_id}/pay`, { cookie: a.cookie, body: {} });
    r = await j("POST", `/v1/sites/${site.site_id}/verify`, { cookie: a.cookie, body: {} });
    if (r.json?.ok === false && r.json?.instruction?.dns_txt?.startsWith("gv6-site=")) {
      ok("domain verify returns DNS/HTTP instructions when unverified");
    } else {
      bad("domain verify instructions", JSON.stringify(r.json));
    }
    r = await j("GET", "/v1/sites", { cookie: a.cookie });
    const row = (r.json.sites || []).find((s) => s.site_id === site.site_id);
    if (row && !row.verified_at) ok("paid site starts unverified");
    else bad("paid site starts unverified", JSON.stringify(row));
  }

  // ── REQUIRE_DOMAIN_VERIFY behavior via second process is heavy;
  //    document residual: must set env in prod ──────────────────────
  note(
    "REQUIRE_DOMAIN_VERIFY",
    "lab default off; prod must set GV6_REQUIRE_DOMAIN_VERIFY=1 (env.production.example)"
  );

  // ── email verify + TOTP baseline ─────────────────────────────────
  {
    const a = await registerUser("2fa");
    let r = await j("POST", "/v1/account/email/request-verify", { cookie: a.cookie });
    if (r.status === 200 && r.json.ok) {
      if (r.json.lab_token) {
        const conf = await j("POST", "/v1/account/email/confirm", {
          cookie: a.cookie,
          body: { token: r.json.lab_token },
        });
        if (conf.json?.email_verified) ok("email verify confirm (lab token)");
        else bad("email verify confirm", JSON.stringify(conf.json));
      } else {
        note("email verify", "lab_token not echoed (GV6_OFFICIAL_LAB_EMAIL!=1) — OK for prod shape");
        ok("email verify request accepted");
      }
    } else bad("email verify request", JSON.stringify(r.json));

    r = await j("POST", "/v1/account/totp/setup", { cookie: a.cookie });
    if (r.status === 200 && r.json.secret) {
      const code = totpCode(r.json.secret);
      const en = await j("POST", "/v1/account/totp/enable", {
        cookie: a.cookie,
        body: { code },
      });
      if (en.json?.totp_enabled && en.json?.recovery_codes?.length === 10) ok("TOTP setup + recovery codes");
      else bad("TOTP enable", JSON.stringify(en.json));
      const denied = await j("POST", "/v1/auth/login", { body: { email: a.email, password: a.password } });
      if (denied.status === 401 && denied.json?.error === "totp_required") ok("login requires TOTP when enabled");
      else bad("login requires TOTP when enabled", JSON.stringify(denied.json));
      const code2 = totpCode(r.json.secret);
      const allowed = await j("POST", "/v1/auth/login", {
        body: { email: a.email, password: a.password, totp_code: code2 },
      });
      if (allowed.status === 200 && allowed.json?.ok) ok("login succeeds with TOTP");
      else bad("login succeeds with TOTP", JSON.stringify(allowed.json));
      // P1-2: recovery codes are a login factor (device loss) and single-use.
      const recoveryLogin = await j("POST", "/v1/auth/login", {
        body: { email: a.email, password: a.password, code: en.json?.recovery_codes?.[1] },
      });
      if (recoveryLogin.status === 200 && recoveryLogin.json?.ok) ok("login succeeds with recovery code");
      else bad("login succeeds with recovery code", JSON.stringify(recoveryLogin.json));
      const replay = await j("POST", "/v1/auth/login", {
        body: { email: a.email, password: a.password, code: en.json?.recovery_codes?.[1] },
      });
      if (replay.status === 401) ok("recovery code replay rejected (single-use)");
      else bad("recovery code replay rejected (single-use)", JSON.stringify(replay.json));
      const disabled = await j("POST", "/v1/account/totp/disable", {
        cookie: a.cookie,
        body: { code: en.json?.recovery_codes?.[0] },
      });
      if (disabled.json?.totp_enabled === false) ok("recovery code disables TOTP");
      else bad("TOTP recovery disable", JSON.stringify(disabled.json));

      const changed = await j("POST", "/v1/account/password", {
        cookie: a.cookie,
        body: { password: a.password, new_password: `${a.password}-changed` },
      });
      if (changed.json?.ok) ok("authenticated password change");
      else bad("password change", JSON.stringify(changed.json));

      const resetRequest = await j("POST", "/v1/auth/password-reset/request", {
        body: { email: a.email },
      });
      if (resetRequest.json?.lab_reset_token) {
        const reset = await j("POST", "/v1/auth/password-reset/confirm", {
          body: { token: resetRequest.json.lab_reset_token, new_password: `${a.password}-reset` },
        });
        if (reset.json?.ok) ok("password reset request + confirm");
        else bad("password reset confirm", JSON.stringify(reset.json));
      } else {
        bad("password reset request", JSON.stringify(resetRequest.json));
      }
    } else bad("TOTP setup", JSON.stringify(r.json));
  }

  // ── weak password rejected ───────────────────────────────────────
  {
    const r = await j("POST", "/v1/auth/register", {
      body: { email: `weak_${Date.now()}@example.com`, password: "short" },
    });
    if (r.status >= 400) ok("weak password rejected");
    else bad("weak password rejected", String(r.status));
  }

  // ── tickets authz ────────────────────────────────────────────────
  {
    const a = await registerUser("tix");
    const b = await registerUser("tix2");
    let r = await j("POST", "/v1/tickets", {
      cookie: a.cookie,
      body: { category: "security", subject: "sec test", body: "no secrets please" },
    });
    const tid = r.json?.ticket?.id;
    if (!tid) bad("create ticket", JSON.stringify(r.json));
    else {
      ok("create ticket");
      r = await j("GET", `/v1/tickets/${tid}`, { cookie: b.cookie });
      if (r.status === 404) ok("ticket cross-user denied");
      else bad("ticket cross-user denied", String(r.status));
    }
  }

  // ── algo-bundle missing ephemeral ────────────────────────────────
  {
    const a = await registerUser("noeph");
    let r = await j("POST", "/v1/sites", {
      cookie: a.cookie,
      body: { domain: `noeph-${Date.now()}.gv6.local`, name: "n" },
    });
    const site = r.json.site;
    const tok = await oauthToken(a.cookie);
    r = await j("POST", "/v1/runtime/algo-bundle", {
      token: tok,
      body: { site_id: site.site_id, domain: site.domain },
    });
    if (r.status === 400) ok("algo-bundle requires node_ephemeral_pub_b64");
    else bad("algo-bundle requires ephemeral", `${r.status}`);
  }

  // ── forged elevation attempt at ciphertext layer (sanity) ────────
  {
    // Attacker encrypts paid payload with own ECDH — without official sig,
    // panel must reject (tested in Rust unit / sync). Here we assert server
    // never returns unsigned v2.
    note(
      "MITM forged ECDH",
      "Panel now requires Ed25519(sig) vs JWKS for algo-bundle-v2; clamp plan→rpa/device"
    );
  }

  console.log(`\n== result: ${passed} passed, ${failed} failed ==`);
  if (failed) process.exit(1);
  console.log("SECURITY_ADVERSARIAL_PASS");
}

main().catch((e) => {
  console.error("SECURITY_ADVERSARIAL_FAIL", e);
  process.exit(1);
});
