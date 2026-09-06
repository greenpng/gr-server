# Green V7 results SDK

Backend-only result clients (six languages). **They do not collect or relay
browser probes** — probes keep flowing straight to the probe server (browser
FE or nginx / Cloudflare edge injection). These SDKs only **receive and
query** the analysis results per session.

## Languages

| Language | Path | Requirements |
|---|---|---|
| JavaScript / Node | `src/index.js` (`@green-v7/results`) | Node 18+ (`fetch`) |
| Python | `python/gr_results.py` | Python 3.8+ (stdlib) |
| Go | `go/gr_results.go` | Go 1.21+ (stdlib) |
| PHP | `php/GrResultsClient.php` | PHP + curl |
| Shell | `shell/gr_sdk.sh` | bash + curl |
| Rust | `rust/` (`gr-results` crate) | ureq 2 + serde_json |

## Common surface

Every client exposes:

- `getResult(sessionId[, projection])` — fetch one snapshot (`projection`:
  `public` | `sdk` | `diagnostic`; diagnostic requires an ops/admin token —
  site keys are refused by the server).
- `waitForResult(sessionId[, timeoutMs][, intervalMs])` — poll until the
  analysis is no longer pending; raises a pending/timeout error afterwards.
- `query(sessionId[, projection[, wait]])` — single fetch or poll.
- Result requests accept optional `strategy_id`, `response_profile`,
  `profile_cap`, and `lang` controls. Omitted values use the site's
  Strategies result policy. JavaScript uses camelCase option names
  (`strategyId`, `responseProfile`, `profileCap`).
- `cookieFields(result)` — extract the site-owner allowlisted cookies the
  probe server captured server-side at session open/ingest (business
  identifiers); returns `null` when the site has no cookie allowlist or the
  visitor sent no cookies.

Auth: `X-Gr-Sdk-Key` header with the site-scoped backend key
(`GR_SITE_RESULT_KEY`). Never embed the key in the browser.

## JavaScript quick start

```js
import { GrResultClient } from "@green-v7/results";

const client = new GrResultClient({
  baseUrl: "https://probe.example.com",
  apiKey: process.env.GR_SITE_RESULT_KEY,
});

const result = await client.waitForResult("sess_...", { projection: "sdk" });
const cf = GrResultClient.cookieFields(result); // { user_id: "u9" } or null
```

For an explicit projection:

```js
const result = await client.getResult("sess_...", {
  projection: "sdk",
  strategyId: "balanced",
  responseProfile: "standard",
  lang: "en",
});
```

`triggerAnalyze` / `completeSession` write helpers also exist on the JS
client; `Idempotency-Key` is honored (409 on conflict).

## Browser note

Cookie capture is **server-side**: the site owner configures a `cookie_fields`
allowlist in the admin panel; the probe server reads only those names from
the session-open request's `Cookie` header and echoes the values back in
`result.sdk_projection.cookie_fields`. The browser FE is unchanged — cookies
just need to reach the open request (same-site; see docs on nginx / CF header
deployment for passing them from your backend).

Per-language details: see each directory's README.
