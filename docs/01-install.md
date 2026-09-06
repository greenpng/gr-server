# Green V7 User Guide

## Install and first login

Run the public installer on a supported Linux host:

```bash
curl -fsSL -O https://raw.githubusercontent.com/greenpng/gr-server/main/install/install.sh
bash install.sh --version 8.0.2
```

The free self-hosted install does not require an Official Site account. The
installer creates a local administrator and writes the one-time credentials to
`/opt/green-v7/data/admin/admin_bootstrap_once.txt`. Change the password after
the first login. Official Site OAuth is optional and is only needed for
cloud-managed or paid features.

## Domains, PV and GV

Set the control-plane bind address and public URLs in `.env`. Keep the
control-plane service on loopback when it is behind a reverse proxy:

```dotenv
GR_BIND=127.0.0.1
GR_ADMIN_BIND=127.0.0.1
GR_PUBLIC_ORIGIN=https://pv.example.com
GR_GATEWAY_ORIGIN=https://gv.example.com
```

PV is the site-bound probe-script origin. GV is the site-bound browser
open/ingest/gateway/result origin. The control plane under `02-probe-analysis/panel/` is separate.

The service itself terminates TLS for each bound pv/gv domain. A reverse proxy
may serve the probe script only. It must not proxy browser upload traffic:
direct Pingora TLS preserves real client IP, TLS and JA4 evidence. Validate
DNS, the certificate chain, and `/v1/health` before onboarding a site.

## Site and embed flow

1. Add the site in **Sites**.
2. If the site is hosted on the Official Site, complete DNS TXT or
   `/.well-known/` verification.
3. Open **SDK / API keys**, create a backend key or browser embed key, and
   restrict embed keys to exact HTTPS origins.
4. Copy the embed snippet into the site. Keep backend secrets server-side.
5. Use the backend SDK to query completed results with the returned session
   correlation ID. Results are asynchronous, so retry with bounded backoff
   until terminal status.

The full secret for a new key is displayed once. Rotate keys during a planned
change and revoke the old key after all callers are updated.

## Free and paid plans

Free sites retain results and use the free device lanes. Paid sites unlock
additional precision and RPA. Paid checkout is handled by Stripe. Entitlements
are applied only after a verified webhook, not after redirecting back from
Checkout. Failed renewal enters grace and then downgrades without deleting
the site or historical results.
