# greenpng results SDK — Rust

Backend-only thin client (`ureq`, blocking, no async runtime requirement).
**Does not collect or relay probes.**

## Usage

```toml
[dependencies]
gr-results = { path = "sdk/rust" }
```

```rust
use gr_results::{Client, cookie_fields};
use std::time::Duration;

let api_key = std::env::var("GR_SITE_RESULT_KEY").unwrap();
let client = Client::new("https://probe.example.com", &api_key);

let result = client.wait_for_result(
    "sess_...",
    "sdk",
    Duration::from_secs(8),
    Duration::from_millis(250),
)?;

if let Some(cf) = cookie_fields(&result) {
    println!("{cf}"); // {"user_id": "u9"}
}
```

Use `ResultOptions` with the explicit v2 methods:

```rust
use gr_results::{Client, ResultOptions};
let options = ResultOptions {
    strategy_id: Some("balanced".into()),
    response_profile: Some("standard".into()),
    lang: Some("en".into()),
    ..Default::default()
};
let result = client.get_result_with_options("sess_...", "sdk", &options)?;
```

## API

| Method | Description |
|---|---|
| `Client::new(base_url, api_key)` | one client per server + site key |
| `get_result(session_id, projection)` | single snapshot |
| `get_result_with_options(session_id, projection, &ResultOptions)` | snapshot with v2 controls |
| `wait_for_result(session_id, projection, timeout, interval)` | poll until ready |
| `wait_for_result_with_options(...)` | poll with v2 controls |
| `query(session_id, projection, wait, timeout, interval)` | single fetch or poll |
| `query_with_options(...)` | single fetch or poll with v2 controls |
| `cookie_fields(&result)` | extract captured allowlisted cookies |

`projection` ∈ `public` \| `sdk` \| `diagnostic` (diagnostic requires an
ops/admin token; site keys are refused).

Errors: `ApiError` (HTTP status + parsed body), `AnalysisPending` (timeout
in `wait_for_result`), transport errors from `ureq`.
