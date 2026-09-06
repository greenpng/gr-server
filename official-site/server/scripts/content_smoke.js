#!/usr/bin/env node
/** Smoke: public content API + seeded pages */
const API = (process.env.GR_OFFICIAL_PUBLIC_URL ?? process.env.GV6_OFFICIAL_PUBLIC_URL) || "http://127.0.0.1:4101";

async function j(path) {
  const res = await fetch(`${API}${path}`);
  const json = await res.json().catch(() => ({}));
  return { status: res.status, json };
}

async function main() {
  let r = await j("/v1/content/site-config");
  if (r.status !== 200 || !r.json.locales?.length) throw new Error("site-config " + JSON.stringify(r.json));
  console.log("locales", r.json.locales.map((l) => l.code).join(", "));

  for (const loc of ["en", "zh-CN"]) {
    r = await j(`/v1/content/page/home?locale=${encodeURIComponent(loc)}`);
    if (r.status !== 200) throw new Error("home " + loc + " " + JSON.stringify(r.json));
    console.log("home", loc, r.json.page.title.slice(0, 40));
  }

  r = await j("/v1/content/by-slug/pricing?locale=en");
  if (r.status !== 200) throw new Error("pricing en");
  console.log("pricing en OK");

  console.log("CONTENT_SMOKE_PASS");
}

main().catch((e) => {
  console.error("CONTENT_SMOKE_FAIL", e);
  process.exit(1);
});
