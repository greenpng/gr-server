# Green V7 results SDK — PHP

Backend-only thin client (requires PHP with `curl` + `json`). **Does not
collect or relay probes.**

## Usage

```php
require __DIR__ . '/GrResultsClient.php';

$client = new GrResultClient(
    'https://probe.example.com',
    getenv('GR_SITE_RESULT_KEY')
);

$result = $client->waitForResult('sess_...', 'sdk');
$cf = GrResultClient::cookieFields($result); // ['user_id' => 'u9'] or null
```

Optional result controls are passed as a final associative-array argument:
`['strategy_id' => 'balanced', 'response_profile' => 'standard',
'profile_cap' => 'advanced', 'lang' => 'en']`.

## API

| Method | Description |
|---|---|
| `new GrResultClient($baseUrl, $apiKey)` | one client per server + site key |
| `getResult($sessionId, $projection = 'sdk', $options = [])` | single snapshot |
| `waitForResult($sessionId, $projection = 'sdk', $timeoutMs = 8000, $intervalMs = 250, $options = [])` | poll until ready |
| `query($sessionId, $projection = 'sdk', $wait = false, ..., $options = [])` | single fetch or poll |
| `GrResultClient::cookieFields($result)` | extract captured allowlisted cookies |

`projection` ∈ `public` \| `sdk` \| `diagnostic` (diagnostic requires an
ops/admin token; site keys are refused).

Errors: `GrApiError` (HTTP status + parsed body), `GrAnalysisPending`
(timeout in `waitForResult`).
