# Green V7 Operations

## Production requirements

- Run the service as the dedicated `greenv7` user with the hardened systemd
  unit.
- Store `.env`, database credentials, Stripe secrets, and the Gmail app
  password outside source control with mode `0600`.
- Inject `GR_LICENSE_SIGNING_KEY_PEM` from the production KMS/HSM secret
  provider. The license Ed25519 key is separate from the bundle-signing key;
  production startup fails closed if it is not configured. Do not put this
  private key in PostgreSQL or source control.
- Inject the independent bundle secrets through
  `GR_BUNDLE_SIGNING_KEY_PEM` and `GR_BUNDLE_WRAP_KEY_B64`, or mount a
  `0600` secret at `GR_BUNDLE_KEY_FILE`. Existing `crypto_keys` rows are
  migrated once to the protected file and the legacy table is dropped.
  Production starts fail-closed when no configured secret, mounted key file,
  or legacy migration source is available.
- Set `GR_OAUTH_TOKEN_ENCRYPTION_KEY` to a 32-byte secret from the same
  secret provider. The panel uses it to encrypt its persisted OAuth refresh
  token; never log or back it up in plaintext.
- The admin-managed integration settings key
  `${GR_OFFICIAL_DATA}/integration_settings.key` is a production secret and
  must be included in encrypted backups.
- Use TLS for PV, GV, the Official Site, and Stripe webhooks.
- Restrict PostgreSQL and Redis to private networks. Do not expose them to the
  public internet.
- Production panel password login is disabled by default. OAuth is required.
  Emergency access requires `GR_ADMIN_BREAK_GLASS=1`, a random 32+ character
  `GR_ADMIN_BREAK_GLASS_TOKEN`, and the same value in the
  `X-Gr-Break-Glass-Token` request header. It is accepted only from the
  kernel-reported loopback peer and each successful use is written to the
  admin audit log. Disable both variables immediately after recovery.

## Backup and retention

Back up PostgreSQL with WAL/PITR enabled and test restores monthly. Encrypt
backup objects using the storage provider's KMS and retain an offline copy.
Keep application data and the backup manifest together so a restore preserves
site, key, billing, and result relationships.

Set a retention window appropriate to the plan and monitor database growth.
Analysis JSON is the largest growth source; use compression/archival before
deleting history. Run cleanup in batches and alert when database or disk
usage exceeds 70/85/95 percent.

## Stripe and Gmail

Operators can configure both integrations from the authenticated CMS admin
page under **Runtime integrations**. The API is `GET/PUT
/v1/admin/integrations`; it requires the CMS staff allowlist and `cms:edit`.
The GET response is metadata only and masks all secret values. A blank secret
in an update preserves the existing value; send JSON `null` deliberately to
clear one.

Stripe production requires `STRIPE_SECRET_KEY`, `STRIPE_PRICE_ID`, and
`STRIPE_WEBHOOK_SECRET`. Configure the webhook endpoint as
`/v1/billing/stripe-webhook`; it must receive the raw request body for
signature verification. Keep event processing idempotent and monitor failed
invoices and webhook retries.

The official site exposes separate Ed25519 public keys at `/v1/jwks`
(bundle envelopes) and `/v1/license-jwks` (short-lived `grlic1` licenses).
Keep their private-key permissions and rotation procedures separate.

Gmail production requires a dedicated mailbox and an app password. Do not
store or log the app password. Monitor SMTP authentication failures and
delivery errors; verification requests should fail closed rather than expose
tokens.

## Load balancer and multi-node onboarding

Give each node a unique node ID and enroll its public key through the control
plane. Permit only the LB-to-node health and probe paths. Start new nodes in
drain mode, verify heartbeats and `/v1/health`, then shift traffic gradually.
Use a canary node before a full rollout. If health or result completeness
drops, drain the node and roll back to the previous signed release.

## Upgrade and rollback

Verify the signed release manifest and artifact hashes before activation.
Back up the database and `.env`, stage the new runtime, run health and smoke
checks, then activate. Keep the previous runtime and module set available
until the acceptance checks pass. Rollback means restoring the previous
signed artifacts and database migration state, not deleting user data.
