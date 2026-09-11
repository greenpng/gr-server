# greenpng results SDK — Shell

Backend-only thin client (bash + curl). **Does not collect or relay
probes.**

## Usage

```bash
source ./gr_sdk.sh
export GR_BASE_URL="https://probe.example.com"
export GR_API_KEY="$GR_SITE_RESULT_KEY"   # site-scoped backend key

gr_get_result "sess_..." "sdk"                      # single snapshot
gr_wait_for_result "sess_..." "sdk" 8000            # poll until ready
gr_query "sess_..." "public" 1 8000                 # wait=1 → poll
gr_get_result "sess_..." "sdk" "balanced" "standard" "en" "advanced"

result=$(gr_get_result "sess_..." "sdk")
gr_cookie_fields "$result"                           # {"user_id": "u9"} or null
```

CLI mode:

```bash
GR_BASE_URL=... GR_API_KEY=... ./gr_sdk.sh get sess_...
./gr_sdk.sh wait sess_... sdk 8000
```

## API

| Function | Description |
|---|---|
| `gr_get_result <session> [projection] [strategy_id] [response_profile] [lang] [profile_cap]` | single snapshot |
| `gr_wait_for_result <session> [projection] [timeout_ms] [strategy_id] [response_profile] [lang] [profile_cap]` | poll until ready |
| `gr_query <session> [projection] [wait] [timeout_ms] [strategy_id] [response_profile] [lang] [profile_cap]` | single fetch or poll |
| `gr_cookie_fields <result_json>` | extract captured allowlisted cookies |

Exit codes: 0 = ok, 1 = transport/HTTP error, 2 = wait timeout,
64 = CLI usage error.

`projection` ∈ `public` \| `sdk` \| `diagnostic` (diagnostic requires an
ops/admin token; site keys are refused).
