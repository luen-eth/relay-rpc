use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

pub fn now_iso() -> String {
    // Keep runtime dependency-free; milliseconds since epoch are stable and sortable.
    format!("{}ms", now_ms())
}

pub fn to_hex(value: u64) -> String {
    format!("0x{value:x}")
}

pub fn parse_block_tag(value: &Value) -> Option<u64> {
    match value {
        Value::String(tag) if tag == "earliest" => Some(0),
        Value::String(tag) if tag.starts_with("0x") => u64::from_str_radix(&tag[2..], 16).ok(),
        _ => None,
    }
}

pub fn rpc_error_message(value: &Value) -> String {
    if let Some(message) = value.get("message").and_then(Value::as_str) {
        return message.to_string();
    }
    value.to_string()
}
