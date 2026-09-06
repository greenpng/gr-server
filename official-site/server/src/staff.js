/**
 * Account roles, sub-accounts, and permission checks.
 */
export const ROLES = ["owner", "admin", "editor", "billing", "viewer"];

export const PERMS = {
  CMS_EDIT: "cms:edit",
  CMS_PUBLISH: "cms:publish",
  SITES_READ: "sites:read",
  SITES_MANAGE: "sites:manage",
  BILLING: "billing:pay",
  MEMBERS_MANAGE: "members:manage",
  ACCOUNT_SECURITY: "account:security",
};

const ROLE_PERMS = {
  owner: Object.values(PERMS),
  admin: [
    PERMS.CMS_EDIT,
    PERMS.CMS_PUBLISH,
    PERMS.SITES_READ,
    PERMS.SITES_MANAGE,
    PERMS.BILLING,
    PERMS.MEMBERS_MANAGE,
    PERMS.ACCOUNT_SECURITY,
  ],
  editor: [PERMS.CMS_EDIT, PERMS.CMS_PUBLISH],
  billing: [PERMS.SITES_READ, PERMS.SITES_MANAGE, PERMS.BILLING],
  viewer: [PERMS.SITES_READ],
};

export function rolePermissions(role) {
  return ROLE_PERMS[role] || [];
}

export async function loadUserRow(db, userId) {
  return db.get("SELECT * FROM users WHERE id=?", userId);
}

export function orgOwnerId(userRow) {
  if (!userRow) return null;
  return userRow.parent_user_id || userRow.id;
}

export function effectiveRole(userRow) {
  if (!userRow) return null;
  const role = userRow.account_role || "owner";
  return ROLES.includes(role) ? role : "viewer";
}

export function userPermissions(userRow) {
  return rolePermissions(effectiveRole(userRow));
}

export function hasPerm(userRow, perm) {
  return userPermissions(userRow).includes(perm);
}

export function isOrgOwner(userRow) {
  return userRow && !userRow.parent_user_id && effectiveRole(userRow) === "owner";
}

export function isCmsStaff(userRow) {
  if (!userRow?.email) return false;
  const allow = ((process.env.GR_CMS_ADMIN_EMAILS ?? process.env.GV6_CMS_ADMIN_EMAILS) || "")
    .split(",")
    .map((s) => s.trim().toLowerCase())
    .filter(Boolean);
  if (allow.length > 0) {
    return allow.includes(String(userRow.email).toLowerCase());
  }
  const deploy = String((process.env.GR_DEPLOY_ENV ?? process.env.GV6_DEPLOY_ENV) || process.env.NODE_ENV || "").toLowerCase();
  // Production: CMS is allowlist-only. Ordinary self-registered owners must not edit public pages.
  if (["prod", "production", "live"].includes(deploy)) {
    return false;
  }
  return hasPerm(userRow, PERMS.CMS_EDIT);
}

export function canManageMembers(userRow) {
  return hasPerm(userRow, PERMS.MEMBERS_MANAGE);
}

export async function publicUserProfile(db, userId) {
  const u = await loadUserRow(db, userId);
  if (!u) return null;
  const ownerId = orgOwnerId(u);
  const owner = ownerId === u.id ? u : await loadUserRow(db, ownerId);
  return {
    id: u.id,
    email: u.email,
    email_verified: !!u.email_verified,
    totp_enabled: !!u.totp_enabled,
    account_role: effectiveRole(u),
    permissions: userPermissions(u),
    org_owner_id: ownerId,
    org_owner_email: owner?.email || u.email,
    is_org_owner: isOrgOwner(u),
  };
}

export async function listOrgMembers(db, ownerId) {
  const rows = await db.all(
    `SELECT id, email, account_role, parent_user_id, email_verified, created_at
     FROM users WHERE id=? OR parent_user_id=? ORDER BY created_at ASC`,
    ownerId,
    ownerId
  );
  return rows.map((m) => ({
    id: m.id,
    email: m.email,
    role: effectiveRole(m),
    email_verified: !!m.email_verified,
    is_owner: !m.parent_user_id,
    created_at: m.created_at,
  }));
}
