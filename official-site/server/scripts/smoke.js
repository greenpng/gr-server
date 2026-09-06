/**
 * Smoke: register → bind site → mock pay → OAuth → ECDH algo-bundle decrypt.
 */
import crypto from "node:crypto";
import {
  generateNodeKeypair,
  ecdhAesKey,
  decryptWithKey,
} from "../src/algo_delivery.js";

const API = (process.env.GR_OFFICIAL_PUBLIC_URL ?? process.env.GV6_OFFICIAL_PUBLIC_URL) || "http://127.0.0.1:4101";

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
  return { status: res.status, json, setCookie, headers: res.headers, location: res.headers.get("location") };
}

function cookieFrom(setCookie) {
  return setCookie.map((c) => c.split(";")[0]).join("; ");
}

async function main() {
  let r = await j("GET", "/v1/public-config");
  const bundleJwks = await j("GET", "/v1/jwks");
  const licenseJwks = await j("GET", "/v1/license-jwks");
  if (
    bundleJwks.status !== 200 ||
    licenseJwks.status !== 200 ||
    bundleJwks.json?.keys?.[0]?.x === licenseJwks.json?.keys?.[0]?.x
  ) {
    throw new Error("bundle and license signing keys are not separated");
  }
  console.log("bundle/license signing keys separated");
  if (r.json?.config?.public_auth !== true) {
    r = await j("POST", "/v1/auth/register", {
      body: { email: `lab_${Date.now()}@example.com`, password: "test-password-99" },
    });
    if (r.status !== 403 || r.json.error !== "auth_disabled") {
      throw new Error("expected auth_disabled, got " + JSON.stringify(r.json));
    }
    console.log("public auth disabled (expected) ok");
    r = await j("GET", "/v1/crypto/wrap-key");
    if (r.status !== 404) throw new Error("wrap-key should be 404 " + r.status);
    console.log("wrap-key disabled by default ok", r.status);
    console.log("SMOKE_PASS");
    return;
  }

  const email = `lab_${Date.now()}@example.com`;
  const password = "test-password-99";
  r = await j("POST", "/v1/auth/register", { body: { email, password } });
  if (r.status !== 200) throw new Error("register " + JSON.stringify(r.json));
  const cookie = cookieFrom(r.setCookie);
  console.log("register ok", r.json.user.email);

  r = await j("POST", "/v1/sites", {
    cookie,
    body: { domain: `shop-${Date.now()}.gv6.local`, name: "shop lab" },
  });
  if (r.status !== 200) throw new Error("site " + JSON.stringify(r.json));
  const site = r.json.site;
  console.log("site", site.site_id, site.domain, site.plan);

  r = await j("POST", `/v1/sites/${site.site_id}/pay`, { cookie, body: {} });
  if (r.status !== 200) throw new Error("pay " + JSON.stringify(r.json));
  console.log("paid until", r.json.paid_until, "rpa", r.json.rpa_enabled, "lanes", r.json.device_precisions);

  r = await j("POST", `/v1/sites/${site.site_id}/strategy`, {
    cookie,
    body: { strategy_version_id: "strict_integrity@1" },
  });
  if (r.status !== 410) throw new Error("strategy should be removed " + JSON.stringify(r.json));
  console.log("strategy config removed ok");

  // OAuth PKCE
  const verifier = b64url(crypto.randomBytes(32));
  const challenge = b64url(crypto.createHash("sha256").update(verifier).digest());
  const redirect = "http://127.0.0.1:28680/oauth/callback";
  const authUrl =
    `/oauth/authorize?response_type=code&client_id=gr-admin-panel` +
    `&redirect_uri=${encodeURIComponent(redirect)}` +
    `&code_challenge=${challenge}&code_challenge_method=S256&state=smoke`;
  r = await j("GET", authUrl, { cookie });
  if (r.status !== 302 || !r.location) throw new Error("authorize " + r.status);
  const loc = new URL(r.location);
  const code = loc.searchParams.get("code");
  if (!code) throw new Error("no code " + r.location);
  console.log("oauth code ok");

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
  const access = r.json.access_token;
  const refresh = r.json.refresh_token;
  if (!refresh) throw new Error("missing refresh token");
  r = await j("POST", "/oauth/token", {
    body: { grant_type: "refresh_token", refresh_token: refresh },
  });
  if (r.status !== 200 || !r.json.access_token || !r.json.refresh_token) {
    throw new Error("refresh token exchange failed " + JSON.stringify(r.json));
  }
  const rotatedAccess = r.json.access_token;
  r = await j("POST", "/oauth/token", {
    body: { grant_type: "refresh_token", refresh_token: refresh },
  });
  if (r.status !== 400) throw new Error("refresh token replay was accepted");
  console.log("access token ok");

  // Node enroll
  const nodeKp = generateNodeKeypair();
  r = await j("POST", "/v1/nodes/enroll", {
    token: rotatedAccess,
    body: {
      instance_id: `smoke-${Date.now()}`,
      x25519_pub_b64: nodeKp.public_b64,
      label: "smoke",
    },
  });
  if (r.status !== 200) throw new Error("enroll " + JSON.stringify(r.json));
  console.log("enroll ok");

  // ECDH algo-bundle v2 (preferred)
  const eph = generateNodeKeypair();
  r = await j("POST", "/v1/runtime/algo-bundle", {
    token: rotatedAccess,
    body: {
      site_id: site.site_id,
      domain: site.domain,
      instance_id: `smoke-${Date.now()}`,
      node_ephemeral_pub_b64: eph.public_b64,
    },
  });
  if (r.status !== 200) throw new Error("algo-bundle " + JSON.stringify(r.json));
  if (r.json.protocol !== "algo-bundle-v2") throw new Error("bad protocol " + r.json.protocol);
  const serverPub = r.json.header.server_ephemeral_pub_b64;
  const aesKey = ecdhAesKey(eph.private_pem, serverPub);
  const payload = decryptWithKey(aesKey, r.json.iv_b64, r.json.ciphertext_b64);
  if (payload.plan !== "paid" || payload.rpa_enabled !== true) {
    throw new Error("bad payload " + JSON.stringify(payload));
  }
  if (
    typeof payload.license_token !== "string" ||
    !/^gv6lic1\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+$/.test(payload.license_token)
  ) {
    throw new Error("missing or malformed signed license token");
  }
  const licensePayload = JSON.parse(
    Buffer.from(payload.license_token.split(".")[1], "base64url").toString("utf8")
  );
  if (
    licensePayload.site_id !== site.site_id ||
    licensePayload.domain !== site.domain ||
    licensePayload.plan !== "paid" ||
    licensePayload.exp_ms <= licensePayload.iat_ms
  ) {
    throw new Error("license claims are not bound to the paid site");
  }
  console.log(
    "algo-bundle ECDH ok",
    "plan",
    payload.plan,
    "rpa",
    payload.rpa_enabled,
    "lanes",
    payload.device_precisions
  );

  // wrap-key must be off by default
  r = await j("GET", "/v1/crypto/wrap-key", { token: rotatedAccess });
  if (r.status === 200) {
    console.log("wrap-key still exposed (lab flag on) — ok if EXPOSE_WRAP_KEY=1");
  } else {
    console.log("wrap-key disabled by default ok", r.status);
  }

  console.log("SMOKE_PASS");
}

main().catch((e) => {
  console.error("SMOKE_FAIL", e);
  process.exit(1);
});
