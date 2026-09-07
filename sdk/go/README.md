# Green V7 results SDK — Go

Backend-only thin client (stdlib `net/http`). **Does not collect or relay
probes.** Query-only surface.

## Usage

```go
package main

import (
	"fmt"
	"os"

	gr "github.com/greenpng/gr-server/sdk/go"
)

func main() {
	apiKey := os.Getenv("GR_SITE_RESULT_KEY")
	c := gr.New("https://probe.example.com", apiKey)

	result, err := c.WaitForResult("sess_...", "sdk", 8000, 250)
	if err != nil { panic(err) }

	cf, err := gr.CookieFields(result) // {"user_id": "u9"} or error
	fmt.Println(cf)
}
```

Pass `gr.ResultOptions{StrategyID: "balanced", ResponseProfile: "standard", Lang: "en"}`
as the optional final argument to `GetResult`, `WaitForResult`, or `Query`, or
use `GetResultWithOptions`.

## API

| Method | Description |
|---|---|
| `New(baseURL, apiKey)` | client for one probe server + site key |
| `GetResult(sessionID, projection, [ResultOptions])` | single snapshot |
| `WaitForResult(sessionID, projection, timeoutMs, intervalMs, [ResultOptions])` | poll until ready |
| `Query(sessionID, projection, wait, timeoutMs, intervalMs, [ResultOptions])` | single fetch or poll |
| `CookieFields(result)` | extract captured allowlisted cookies |

`projection` ∈ `public` \| `sdk` \| `diagnostic` (diagnostic requires an
ops/admin token).

Errors: `*APIError` (HTTP status + parsed body), `*AnalysisPending` (timeout),
plain errors from transport.
