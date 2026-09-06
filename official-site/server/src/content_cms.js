/**
 * Multilingual marketing/content CMS (PostgreSQL).
 */
import crypto from "node:crypto";
import { isCmsStaff } from "./staff.js";
import { sanitizeBlocks } from "./sanitize_html.js";
import {
  PRICING_FAQ,
  MARKETING_PAGE_META,
  EXTRA_CONTENT_PAGES,
} from "./content_seed.js";

export const LOCALES = [
  { code: "en", path: "", hreflang: "en", label: "English", dir: "ltr" },
  { code: "zh-CN", path: "zh-cn", hreflang: "zh-CN", label: "简体中文", dir: "ltr" },
];

export const DEFAULT_LOCALE = "en";
export const PAGE_KINDS = ["marketing", "blog", "changelog", "doc"];


export function localeByCode(code) {
  const c = String(code || "").trim();
  return LOCALES.find((l) => l.code === c || l.path === c.toLowerCase()) || null;
}

export function localePathToCode(pathSeg) {
  const p = String(pathSeg || "").trim().toLowerCase();
  if (!p || p === "en") return DEFAULT_LOCALE;
  return LOCALES.find((l) => l.path && l.path === p)?.code || null;
}

export async function migrateContent(db) {
  await db.exec(`
    CREATE TABLE IF NOT EXISTS content_pages (
      page_key TEXT PRIMARY KEY,
      kind TEXT NOT NULL DEFAULT 'marketing',
      sort_order INTEGER NOT NULL DEFAULT 0,
      created_at BIGINT NOT NULL,
      updated_at BIGINT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS content_locales (
      page_key TEXT NOT NULL REFERENCES content_pages(page_key) ON DELETE CASCADE,
      locale TEXT NOT NULL,
      slug TEXT NOT NULL,
      title TEXT NOT NULL,
      subtitle TEXT,
      meta_title TEXT,
      meta_description TEXT,
      body_json TEXT NOT NULL,
      status TEXT NOT NULL DEFAULT 'draft',
      published_at BIGINT,
      updated_at BIGINT NOT NULL,
      updated_by TEXT,
      PRIMARY KEY (page_key, locale)
    );
    CREATE INDEX IF NOT EXISTS idx_content_locales_slug ON content_locales(locale, slug, status);
  `);
}

function parseBody(raw) {
  if (typeof raw === "object" && raw !== null) return raw;
  try {
    return JSON.parse(String(raw || "{}"));
  } catch {
    return { blocks: [] };
  }
}

export function normalizeBlocks(body) {
  const b = parseBody(body);
  if (!Array.isArray(b.blocks)) b.blocks = [];
  return sanitizeBlocks(b);
}

export function rowToLocale(row) {
  if (!row) return null;
  return {
    page_key: row.page_key,
    locale: row.locale,
    slug: row.slug,
    title: row.title,
    subtitle: row.subtitle || "",
    meta_title: row.meta_title || row.title,
    meta_description: row.meta_description || "",
    body: normalizeBlocks(row.body_json),
    status: row.status,
    published_at: row.published_at || null,
    updated_at: row.updated_at,
    updated_by: row.updated_by || null,
  };
}

export function siteConfig() {
  return { default_locale: DEFAULT_LOCALE, locales: LOCALES };
}

export async function listPages(db, { locale, kind, status = "published" } = {}) {
  let sql = `
    SELECT p.page_key, p.kind, p.sort_order, l.locale, l.slug, l.title, l.subtitle,
           l.meta_title, l.meta_description, l.body_json, l.status, l.published_at, l.updated_at
    FROM content_pages p
    JOIN content_locales l ON l.page_key = p.page_key
    WHERE 1=1
  `;
  const params = [];
  if (locale) {
    sql += " AND l.locale = ?";
    params.push(locale);
  }
  if (kind) {
    sql += " AND p.kind = ?";
    params.push(kind);
  }
  if (status) {
    sql += " AND l.status = ?";
    params.push(status);
  }
  sql += " ORDER BY p.sort_order ASC, p.page_key ASC";
  const rows = await db.all(sql, ...params);
  return rows.map((r) => ({
    page_key: r.page_key,
    kind: r.kind,
    sort_order: r.sort_order,
    ...rowToLocale(r),
  }));
}

export async function getPageLocale(db, pageKey, locale, { includeDraft = false } = {}) {
  const row = await db.get(
    `SELECT p.page_key, p.kind, p.sort_order, l.*
     FROM content_pages p
     JOIN content_locales l ON l.page_key = p.page_key
     WHERE p.page_key = ? AND l.locale = ?`,
    pageKey,
    locale
  );
  if (!row) return null;
  if (!includeDraft && row.status !== "published") return null;
  return {
    page_key: row.page_key,
    kind: row.kind,
    sort_order: row.sort_order,
    ...rowToLocale(row),
  };
}

export async function getBySlug(db, locale, slug, { includeDraft = false } = {}) {
  const row = await db.get(
    `SELECT p.page_key, p.kind, p.sort_order, l.*
     FROM content_pages p
     JOIN content_locales l ON l.page_key = p.page_key
     WHERE l.locale = ? AND l.slug = ?`,
    locale,
    slug
  );
  if (!row) return null;
  if (!includeDraft && row.status !== "published") return null;
  return {
    page_key: row.page_key,
    kind: row.kind,
    sort_order: row.sort_order,
    ...rowToLocale(row),
  };
}

export async function upsertPageLocale(
  db,
  { pageKey, kind, locale, slug, title, subtitle, metaTitle, metaDescription, body, status, updatedBy }
) {
  const t = Date.now();
  const existingPage = await db.get("SELECT page_key FROM content_pages WHERE page_key=?", pageKey);
  if (!existingPage) {
    await db.run(
      `INSERT INTO content_pages (page_key, kind, sort_order, created_at, updated_at)
       VALUES (?, ?, ?, ?, ?)`,
      pageKey,
      kind || "marketing",
      0,
      t,
      t
    );
  } else {
    await db.run("UPDATE content_pages SET kind=COALESCE(?, kind), updated_at=? WHERE page_key=?", kind || null, t, pageKey);
  }
  const bodyJson = JSON.stringify(normalizeBlocks(body));
  const st = status === "published" ? "published" : "draft";
  const prev = await db.get("SELECT status, published_at FROM content_locales WHERE page_key=? AND locale=?", pageKey, locale);
  const publishedAt = st === "published" ? prev?.published_at || t : null;

  await db.run(
    `INSERT INTO content_locales (
      page_key, locale, slug, title, subtitle, meta_title, meta_description,
      body_json, status, published_at, updated_at, updated_by
    ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
    ON CONFLICT(page_key, locale) DO UPDATE SET
      slug=EXCLUDED.slug,
      title=EXCLUDED.title,
      subtitle=EXCLUDED.subtitle,
      meta_title=EXCLUDED.meta_title,
      meta_description=EXCLUDED.meta_description,
      body_json=EXCLUDED.body_json,
      status=EXCLUDED.status,
      published_at=CASE WHEN EXCLUDED.status='published' THEN COALESCE(content_locales.published_at, EXCLUDED.published_at) ELSE NULL END,
      updated_at=EXCLUDED.updated_at,
      updated_by=EXCLUDED.updated_by`,
    pageKey,
    locale,
    slug,
    title,
    subtitle || "",
    metaTitle || title,
    metaDescription || "",
    bodyJson,
    st,
    publishedAt,
    t,
    updatedBy || null
  );
  return getPageLocale(db, pageKey, locale, { includeDraft: true });
}

export async function publishLocale(db, pageKey, locale, updatedBy) {
  const t = Date.now();
  const r = await db.run(
    `UPDATE content_locales SET status='published', published_at=COALESCE(published_at, ?), updated_at=?, updated_by=?
     WHERE page_key=? AND locale=?`,
    t,
    t,
    updatedBy || null,
    pageKey,
    locale
  );
  if (!r.changes) return null;
  return getPageLocale(db, pageKey, locale, { includeDraft: true });
}

export async function refreshGreenpngContent(db) {
  for (const [pageKey, meta] of Object.entries(MARKETING_PAGE_META)) {
    for (const locale of ["en", "zh-CN"]) {
      const loc = meta[locale];
      await upsertPageLocale(db, {
        pageKey,
        kind: meta.kind,
        locale,
        slug: loc.slug,
        title: loc.title,
        subtitle: loc.subtitle,
        metaTitle: loc.meta_title,
        metaDescription: loc.meta_description,
        body: loc.body,
        status: "published",
        updatedBy: "seed",
      });
      await db.run("UPDATE content_pages SET sort_order=? WHERE page_key=?", meta.sort_order, pageKey);
    }
  }
  await upsertContentPages(db, EXTRA_CONTENT_PAGES);
  return true;
}

export async function seedContentIfEmpty(db) {
  const row = await db.get("SELECT COUNT(*)::int AS c FROM content_pages");
  const n = row?.c ?? 0;
  if (n > 0) return false;
  await refreshGreenpngContent(db);
  return true;
}

async function upsertContentPages(db, pages) {
  for (const p of pages) {
    for (const [locale, loc] of Object.entries(p.locales)) {
      await upsertPageLocale(db, {
        pageKey: p.page_key,
        kind: p.kind,
        locale,
        slug: loc.slug,
        title: loc.title,
        subtitle: loc.subtitle,
        metaTitle: loc.meta_title,
        metaDescription: loc.meta_description,
        body: loc.body,
        status: "published",
        updatedBy: "seed",
      });
      await db.run("UPDATE content_pages SET sort_order=? WHERE page_key=?", p.sort_order, p.page_key);
    }
  }
}

export async function seedExtraContentPages(db) {
  await upsertContentPages(db, EXTRA_CONTENT_PAGES);
}

export async function patchLocaleUrls(db) {
  const rows = await db.all("SELECT page_key, locale, body_json FROM content_locales");
  for (const row of rows) {
    const raw = String(row.body_json || "");
    if (!raw.includes("/en/") && !raw.includes('"/en"')) continue;
    const next = raw.replaceAll("/en/", "/").replaceAll('"/en"', '"/"');
    if (next !== raw) {
      await db.run(
        "UPDATE content_locales SET body_json=?, updated_at=? WHERE page_key=? AND locale=?",
        next,
        Date.now(),
        row.page_key,
        row.locale
      );
    }
  }
}

export function isCmsAdmin(user) {
  return isCmsStaff(user);
}

export function newPageKey(prefix = "page") {
  return `${prefix}_${crypto.randomBytes(4).toString("hex")}`;
}

export async function patchPricingFaq(db) {
  for (const locale of ["en", "zh-CN"]) {
    const row = await db.get("SELECT body_json FROM content_locales WHERE page_key='pricing' AND locale=?", locale);
    if (!row) continue;
    let body;
    try {
      body = JSON.parse(row.body_json || "{}");
    } catch {
      continue;
    }
    if (!Array.isArray(body.blocks)) body.blocks = [];
    if (body.blocks.some((b) => b.type === "faq")) continue;
    body.blocks.push({ type: "faq", items: PRICING_FAQ[locale].items });
    await db.run("UPDATE content_locales SET body_json=?, updated_at=? WHERE page_key='pricing' AND locale=?", JSON.stringify(body), Date.now(), locale);
  }
}

/** Force-publish current greenpng marketing + docs/blog/changelog copy. */
export async function patchMarketingLayout(db) {
  await refreshGreenpngContent(db);
}
