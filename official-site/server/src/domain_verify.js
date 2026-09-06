/**
 * Domain ownership verification (DNS TXT / HTTP well-known).
 * Aligns with IETF DCV + WorkOS/Auth0-style TXT tokens.
 */
import dns from "node:dns/promises";
import net from "node:net";
import crypto from "node:crypto";

export function makeVerifyToken(siteId) {
  const nonce = crypto.randomBytes(16).toString("hex");
  return {
    token: `gv6-site=${siteId}.${nonce}`,
    dns_name: `_gv6-verify`,
    http_path: `/.well-known/gv6-verify.txt`,
    nonce,
  };
}

export function dnsFqdn(domain, dnsName = "_gv6-verify") {
  const d = String(domain || "")
    .trim()
    .toLowerCase()
    .replace(/\.$/, "");
  return `${dnsName}.${d}`;
}

function ipv4ToInt(ip) {
  return ip.split(".").reduce((acc, o) => (acc << 8) + Number(o), 0) >>> 0;
}

export function isPrivateOrReservedIp(ip) {
  const ver = net.isIP(ip);
  if (!ver) return true;
  if (ver === 4) {
    const n = ipv4ToInt(ip);
    const ranges = [
      [ipv4ToInt("0.0.0.0"), ipv4ToInt("0.255.255.255")],
      [ipv4ToInt("10.0.0.0"), ipv4ToInt("10.255.255.255")],
      [ipv4ToInt("127.0.0.0"), ipv4ToInt("127.255.255.255")],
      [ipv4ToInt("169.254.0.0"), ipv4ToInt("169.254.255.255")],
      [ipv4ToInt("172.16.0.0"), ipv4ToInt("172.31.255.255")],
      [ipv4ToInt("192.168.0.0"), ipv4ToInt("192.168.255.255")],
      [ipv4ToInt("100.64.0.0"), ipv4ToInt("100.127.255.255")],
      [ipv4ToInt("224.0.0.0"), ipv4ToInt("255.255.255.255")],
    ];
    return ranges.some(([a, b]) => n >= a && n <= b);
  }
  const v6 = ip.toLowerCase();
  return (
    v6 === "::1" ||
    v6 === "::" ||
    v6.startsWith("fc") ||
    v6.startsWith("fd") ||
    v6.startsWith("fe80:") ||
    v6.startsWith("::ffff:")
  );
}

export async function assertPublicHostname(host) {
  const h = String(host || "")
    .trim()
    .toLowerCase()
    .replace(/\.$/, "");
  if (!h || h === "localhost" || h.endsWith(".localhost") || h.endsWith(".local")) {
    throw new Error("blocked_host");
  }
  if (net.isIP(h)) {
    if (isPrivateOrReservedIp(h)) throw new Error("blocked_ip");
    return;
  }
  const seen = await dns.lookup(h, { all: true, verbatim: true });
  if (!seen.length) throw new Error("dns_empty");
  for (const rec of seen) {
    if (isPrivateOrReservedIp(rec.address)) throw new Error("blocked_ip");
  }
}

export async function checkDnsTxt(domain, expectedToken) {
  const name = dnsFqdn(domain);
  try {
    const records = await dns.resolveTxt(name);
    const flat = records.map((r) => r.join("")).join("\n");
    return {
      ok: flat.includes(expectedToken),
      method: "dns_txt",
      name,
      seen: flat.slice(0, 500),
    };
  } catch (e) {
    return { ok: false, method: "dns_txt", name, error: String(e.message || e) };
  }
}

export async function checkHttpWellKnown(domain, expectedToken) {
  const wantHost = String(domain || "")
    .trim()
    .toLowerCase()
    .replace(/\.$/, "");
  const url = `https://${wantHost}/.well-known/gv6-verify.txt`;
  try {
    await assertPublicHostname(wantHost);
    // Do NOT follow cross-host redirects — that would allow DCV hijack via open redirect.
    const res = await fetch(url, {
      redirect: "manual",
      signal: AbortSignal.timeout(8000),
    });
    if (res.status >= 300 && res.status < 400) {
      const loc = res.headers.get("location") || "";
      let locHost = "";
      try {
        locHost = new URL(loc, url).hostname.toLowerCase();
      } catch {
        locHost = "";
      }
      if (locHost && locHost !== wantHost && !locHost.endsWith(`.${wantHost}`)) {
        return {
          ok: false,
          method: "http_well_known",
          url,
          status: res.status,
          error: "redirect_off_host",
          location: loc.slice(0, 200),
        };
      }
      // Same-host redirect: follow once with manual again.
      if (loc) {
        const next = new URL(loc, url);
        await assertPublicHostname(next.hostname);
        const res2 = await fetch(next.href, {
          redirect: "manual",
          signal: AbortSignal.timeout(8000),
        });
        const text2 = await res2.text();
        return {
          ok: res2.ok && text2.includes(expectedToken),
          method: "http_well_known",
          url: new URL(loc, url).href,
          status: res2.status,
        };
      }
    }
    const text = await res.text();
    return {
      ok: res.ok && text.includes(expectedToken),
      method: "http_well_known",
      url,
      status: res.status,
    };
  } catch (e) {
    return { ok: false, method: "http_well_known", url, error: String(e.message || e) };
  }
}

export async function verifyDomain(domain, expectedToken) {
  const dnsR = await checkDnsTxt(domain, expectedToken);
  if (dnsR.ok) return dnsR;
  const httpR = await checkHttpWellKnown(domain, expectedToken);
  if (httpR.ok) return httpR;
  return { ok: false, tried: [dnsR, httpR] };
}
