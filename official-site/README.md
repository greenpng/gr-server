# Green V7 Official Site

The official site provides account registration, domain verification, site
management, and paid-plan checkout. Production billing uses Stripe Checkout;
application mail uses Gmail SMTP with a Google app password.

Public docs: **/docs/deploy** (deployment cookbook —
nginx / Cloudflare / site-script probe deploy + six-language results SDK);
source at `web/docs-deploy.html`, markdown at `docs/guides/12-DEPLOYMENT-COOKBOOK.md`.

## Production checklist

1. Copy `env.production.example` into a secret-only environment file.
2. Set PostgreSQL, public origins, OAuth/CMS values, Stripe keys and price ID.
3. Set `GMAIL_USER` and `GMAIL_APP_PASSWORD`; never use the account password.
4. Configure Stripe webhooks for checkout completion, invoice success/failure,
   and subscription updates/deletion at
   `/v1/billing/stripe-webhook`.
5. Put the service behind TLS and run `npm run test:all` before release.

If Stripe variables are absent, the lab mock adapter is used. Do not run that
mode in production. If Gmail variables are absent, production verification
mail fails closed rather than returning raw verification tokens.

## Admin-managed integration settings

After login, open the CMS admin page at the configured
`GV6_ADMIN_CMS_PATH`. The **Runtime integrations** section can save Stripe and
Gmail settings without restarting the service. Secrets are encrypted with the
installation key in `GV6_OFFICIAL_DATA/integration_settings.key` (or the
`GV6_OFFICIAL_SETTINGS_KEY` secret) and are never returned by the GET API.
Back up that key together with the database, or encrypted settings cannot be
restored.

Local lab behavior remains mock-by-default. The admin page can be used with
Stripe test keys and a mocked SMTP transport in unit tests; no card network or
Gmail delivery is required for the local suite.

Visitor / request **analysis** — free vs paid = device precision + RPA.  
See also: `docs/PACKAGING_SECURITY_ARCHITECTURE.md` (multi-arch OTA, ECDH algo delivery, domain verify, 2FA).

```bash
cd /home/ubuntu/greenpng/01-official-site
npm install
# 日常开发：ECDH algo-bundle 即可，不必开 wrap-key
GV6_OFFICIAL_LAB_EMAIL=1 npm start
# http://127.0.0.1:4101
npm run test:smoke
npm run test:all          # API smoke + content + functional + security
npm run test:ui           # Playwright responsive + Lighthouse (needs :3000 web)

# 仅当需要测旧 lab wrap-key 回退时：
# GV6_OFFICIAL_EXPOSE_WRAP_KEY=1 GV6_OFFICIAL_LAB_EMAIL=1 npm start
```

日常开发不跑 `build_multiarch.sh` / GitHub Release；发布或测 OTA 时再看 `docs/LOCAL_MULTIARCH_BUILD.md`。

## Plans

| | Free | Paid ($99/site/year) |
|--|------|----------------------|
| Device KV | dv4 / dv5 / dv6 | dv0 + dv4 / dv5 / dv6 |
| RPA | off | on |
| Domain | Panel can add free locally | Official bind + DNS/HTTP verify → panel sync |

## Security APIs (baseline)

| API | Purpose |
|-----|---------|
| `POST /v1/account/email/request-verify` | Email verification token |
| `POST /v1/account/totp/setup` + `/enable` | TOTP 2FA |
| `POST /v1/sites/:id/verify` | DNS TXT / well-known ownership |
| `POST /v1/nodes/enroll` | Panel X25519 pubkey enroll |
| `POST /v1/runtime/algo-bundle` | ECDH v2 encrypted entitlement (preferred) |
| `POST /v1/tickets` | Support tickets |

Env:
- Lab: `GV6_OFFICIAL_EXPOSE_WRAP_KEY=1` (optional lab fallback), `GV6_OFFICIAL_LAB_EMAIL=1`
- Prod: see `env.production.example` — wrap-key off, `GV6_REQUIRE_DOMAIN_VERIFY=1`
- Common: `GV6_OFFICIAL_PORT`, `GV6_OFFICIAL_DATA`, `GV6_ADMIN_OAUTH_REDIRECTS`

Default: wrap-key endpoint is **off**; panel uses ECDH `/v1/runtime/algo-bundle`.

## UI / responsive tests (`official-web`)

```bash
# API + web running (or scripts/run_ui_tests.sh starts them)
cd official-web
UI_QUICK=1 npm run test:responsive    # 22 pages × 2 viewports
npm run test:lighthouse               # a11y / best-practices / SEO
npm run test:ui                       # both

# Full matrix (6 viewports, all CMS paths)
npm run test:responsive
```

Run locally before release; no GitHub Actions workflow is configured for these UI tests.
