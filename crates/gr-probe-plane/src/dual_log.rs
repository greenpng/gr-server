//! Two log channels: process (`system`) and probe-business (`probe_business`).
//! Emission is best-effort and must never block ingest/analyze.

use serde_json::{json, Map, Value};

const SENSITIVE_KEYS: &[&str] = &[
    "password",
    "secret",
    "token",
    "dsn",
    "database_url",
    "authorization",
    "cookie",
    "grant",
    "seal_grant",
    "private_key",
    "payload_json",
    "fields_json",
    "result_json",
    "raw_body",
    "wrap_key",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    System,
    ProbeBusiness,
}

impl Channel {
    pub fn as_str(self) -> &'static str {
        match self {
            Channel::System => "system",
            Channel::ProbeBusiness => "probe_business",
        }
    }
}

pub fn redact_value(v: Value) -> Value {
    match v {
        Value::Object(map) => Value::Object(redact_map(map)),
        Value::Array(arr) => Value::Array(arr.into_iter().map(redact_value).collect()),
        other => other,
    }
}

fn redact_map(map: Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    for (k, v) in map {
        let kl = k.to_ascii_lowercase();
        if SENSITIVE_KEYS.iter().any(|s| kl.contains(s)) {
            out.insert(k, json!("[redacted]"));
        } else {
            out.insert(k, redact_value(v));
        }
    }
    out
}

/// Non-blocking structured emit. Failures are ignored.
pub fn emit(channel: Channel, code: &str, fields: Value) {
    let body = json!({
        "channel": channel.as_str(),
        "code": code,
        "fields": redact_value(fields),
    });
    match channel {
        Channel::System => log::info!(target: "gr_system", "{}", body),
        Channel::ProbeBusiness => log::info!(target: "gr_probe_business", "{}", body),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_dsn_and_payload() {
        let v = json!({
            "ok": true,
            "dsn": "postgresql://gr:secret@127.0.0.1/db",
            "nested": {"payload_json": {"raw": 1}, "site_id": "s1"}
        });
        let r = redact_value(v);
        assert_eq!(r["dsn"], "[redacted]");
        assert_eq!(r["nested"]["payload_json"], "[redacted]");
        assert_eq!(r["nested"]["site_id"], "s1");
    }
}
