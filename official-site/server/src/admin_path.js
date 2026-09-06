/**
 * Non-guessable CMS admin URL. Production requires GV6_ADMIN_CMS_PATH (≥12 chars).
 */
const IS_PROD = ["prod", "production", "live"].includes(
  ((process.env.GR_DEPLOY_ENV ?? process.env.GV6_DEPLOY_ENV) || process.env.NODE_ENV || "lab").toLowerCase()
);

export function adminCmsSegment() {
  const raw = String((process.env.GR_ADMIN_CMS_PATH ?? process.env.GV6_ADMIN_CMS_PATH) || "").trim().replace(/^\/+|\/+$/g, "");
  if (raw) {
    if (!/^[a-zA-Z0-9_-]+$/.test(raw)) {
      throw new Error("GR_ADMIN_CMS_PATH / GV6_ADMIN_CMS_PATH must be alphanumeric (with _ - only)");
    }
    if (IS_PROD && raw.length < 12) {
      throw new Error("GR_ADMIN_CMS_PATH / GV6_ADMIN_CMS_PATH must be at least 12 characters in production");
    }
    return raw;
  }
  if (IS_PROD) {
    throw new Error("GR_ADMIN_CMS_PATH / GV6_ADMIN_CMS_PATH is required in production (e.g. openssl rand -hex 16)");
  }
  return "cms-lab-dev";
}

export function adminCmsRoutes() {
  const seg = adminCmsSegment();
  return [`/${seg}`, `/${seg}/`];
}

export function isProduction() {
  return IS_PROD;
}
