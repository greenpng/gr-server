/**
 * Public + admin HTTP routes for multilingual content CMS.
 */
import {
  siteConfig,
  listPages,
  getPageLocale,
  getBySlug,
  upsertPageLocale,
  publishLocale,
  localeByCode,
  PAGE_KINDS,
  newPageKey,
} from "./content_cms.js";
import { PERMS, hasPerm } from "./staff.js";

export function registerContentRoutes(app, { db, requireUser, requireUserRow, isCmsStaff, now }) {
  async function requireCms(req, reply, { publish = false } = {}) {
    const row = await requireUserRow(req, reply);
    if (!row) return null;
    if (!isCmsStaff(row)) {
      reply.code(403).send({
        ok: false,
        error: "cms_forbidden",
        hint: "Need cms:edit role or GV6_CMS_ADMIN_EMAILS.",
      });
      return null;
    }
    const need = publish ? PERMS.CMS_PUBLISH : PERMS.CMS_EDIT;
    if (!hasPerm(row, need)) {
      reply.code(403).send({ ok: false, error: "cms_forbidden", required: need });
      return null;
    }
    return row;
  }

  app.get("/v1/content/site-config", async () => ({ ok: true, ...siteConfig() }));

  app.get("/v1/content/pages", async (req, reply) => {
    const locale = String(req.query?.locale || "").trim();
    if (!localeByCode(locale)) {
      return reply.code(400).send({ ok: false, error: "bad_locale", locales: siteConfig().locales });
    }
    const kind = req.query?.kind ? String(req.query.kind) : undefined;
    const status = req.query?.status === "all" ? null : "published";
    const pages = await listPages(db, { locale, kind, status });
    return { ok: true, locale, pages };
  });

  app.get("/v1/content/page/:pageKey", async (req, reply) => {
    const locale = String(req.query?.locale || "").trim();
    if (!localeByCode(locale)) {
      return reply.code(400).send({ ok: false, error: "bad_locale" });
    }
    const page = await getPageLocale(db, req.params.pageKey, locale);
    if (!page) return reply.code(404).send({ ok: false, error: "not_found" });
    return { ok: true, page };
  });

  app.get("/v1/content/by-slug/:slug", async (req, reply) => {
    const locale = String(req.query?.locale || "").trim();
    if (!localeByCode(locale)) {
      return reply.code(400).send({ ok: false, error: "bad_locale" });
    }
    const slug = req.params.slug === "_" ? "" : String(req.params.slug || "");
    const page = await getBySlug(db, locale, slug);
    if (!page) return reply.code(404).send({ ok: false, error: "not_found" });
    return { ok: true, page };
  });

  app.get("/v1/admin/content/pages", async (req, reply) => {
    const u = await requireCms(req, reply);
    if (!u) return;
    const locale = req.query?.locale ? String(req.query.locale) : undefined;
    const pages = await listPages(db, { locale, status: null });
    const keys = await db.all(
      "SELECT page_key, kind, sort_order, updated_at FROM content_pages ORDER BY sort_order, page_key"
    );
    return { ok: true, pages, page_keys: keys, locales: siteConfig().locales };
  });

  app.post("/v1/admin/content/pages", async (req, reply) => {
    const u = await requireCms(req, reply);
    if (!u) return;
    const pageKey = String(req.body?.page_key || newPageKey()).trim();
    const kind = String(req.body?.kind || "marketing");
    if (!PAGE_KINDS.includes(kind)) {
      return reply.code(400).send({ ok: false, error: "bad_kind", kinds: PAGE_KINDS });
    }
    const t = now();
    await db.run(
      `INSERT INTO content_pages (page_key, kind, sort_order, created_at, updated_at)
       VALUES (?, ?, ?, ?, ?)
       ON CONFLICT(page_key) DO UPDATE SET kind=EXCLUDED.kind, updated_at=EXCLUDED.updated_at`,
      pageKey,
      kind,
      Number(req.body?.sort_order) || 0,
      t,
      t
    );
    return { ok: true, page_key: pageKey, kind };
  });

  app.put("/v1/admin/content/pages/:pageKey/locales/:locale", async (req, reply) => {
    const u = await requireCms(req, reply);
    if (!u) return;
    const locale = String(req.params.locale || "").trim();
    if (!localeByCode(locale)) {
      return reply.code(400).send({ ok: false, error: "bad_locale" });
    }
    const pageKey = String(req.params.pageKey || "").trim();
    const title = String(req.body?.title || "").trim();
    if (!title) return reply.code(400).send({ ok: false, error: "title_required" });
    const slug = req.body?.slug != null ? String(req.body.slug).trim() : pageKey;
    const updatedBy = u.email || u.id;
    const page = await upsertPageLocale(db, {
      pageKey,
      kind: req.body?.kind,
      locale,
      slug,
      title,
      subtitle: req.body?.subtitle,
      metaTitle: req.body?.meta_title,
      metaDescription: req.body?.meta_description,
      body: req.body?.body,
      status: req.body?.status === "published" ? "published" : "draft",
      updatedBy,
    });
    return { ok: true, page };
  });

  app.post("/v1/admin/content/pages/:pageKey/locales/:locale/publish", async (req, reply) => {
    const u = await requireCms(req, reply, { publish: true });
    if (!u) return;
    const updatedBy = u.email || u.id;
    const page = await publishLocale(db, req.params.pageKey, req.params.locale, updatedBy);
    if (!page) return reply.code(404).send({ ok: false, error: "not_found" });
    return { ok: true, page };
  });
}
