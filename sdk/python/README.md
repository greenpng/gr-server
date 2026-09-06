# Green V7 results SDK — Python

Backend-only thin client. **Does not collect or relay probes** — it only
reads analysis results for sessions (browser/edge probed).

Requires Python 3.8+ (stdlib only: `urllib`).

## Install

```bash
cp gr_results.py /srv/your-app/lib/   # or add to PYTHONPATH
```

## Usage

```python
import os
from gr_results import GrResultClient, cookie_fields

client = GrResultClient(
    base_url="https://probe.example.com",
    api_key=os.environ["GR_SITE_RESULT_KEY"],
)

# One-shot
result = client.get_result("sess_...", projection="sdk")

# Optional v2 result controls
result = client.get_result(
    "sess_...", projection="sdk", strategy_id="balanced",
    response_profile="standard", lang="en",
)

# Wait until analysis is ready (timeout raises GrAnalysisPending)
result = client.wait_for_result("sess_...", timeout_ms=8000)

# Query helper (wait=False → single fetch)
result = client.query("sess_...", projection="public", wait=True)

# Business identifiers: allowlisted cookies captured server-side
print(cookie_fields(result))   # {"user_id": "u9"} or None
```

## API

| Method | Description |
|---|---|
| `get_result(session_id, projection="sdk", strategy_id=None, response_profile=None, profile_cap=None, lang=None)` | fetch one snapshot |
| `wait_for_result(..., strategy_id=None, response_profile=None, profile_cap=None, lang=None)` | poll until ready |
| `query(..., strategy_id=None, response_profile=None, profile_cap=None, lang=None)` | single fetch or poll |
| `cookie_fields(result)` | extract captured allowlisted cookies |

`projection` ∈ `public` \| `sdk` \| `diagnostic` (diagnostic requires an
ops/admin token; site keys are refused by the server).

Errors: `GrApiError` (HTTP status + parsed body), `GrAnalysisPending`
(timeout in `wait_for_result`).
