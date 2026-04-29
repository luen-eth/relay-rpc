use serde_json::Value;

use crate::util::parse_block_tag;

#[derive(Clone, Debug)]
pub struct RequestAnalysis {
    pub route_key: String,
    pub max_get_logs_range: Option<u64>,
}

pub fn analyze_body(body: &[u8]) -> RequestAnalysis {
    let Ok(value) = serde_json::from_slice::<Value>(body) else {
        return RequestAnalysis {
            route_key: "invalid-json".to_string(),
            max_get_logs_range: None,
        };
    };

    let calls = match value {
        Value::Array(items) => items,
        item => vec![item],
    };

    let mut methods = Vec::new();
    let mut ranges = Vec::new();

    for call in calls {
        let Some(method) = call.get("method").and_then(Value::as_str) else {
            continue;
        };
        methods.push(method.to_string());
        if method == "eth_getLogs" {
            if let Some(filter) = call
                .get("params")
                .and_then(Value::as_array)
                .and_then(|params| params.first())
            {
                if let Some(range) = get_logs_range(filter) {
                    ranges.push(range);
                }
            }
        }
    }

    let max_get_logs_range = ranges.into_iter().max();
    let route_key = if let Some(range) = max_get_logs_range {
        format!("eth_getLogs:{}", ((range + 9999) / 10000) * 10000)
    } else if methods.len() == 1 {
        methods[0].clone()
    } else {
        "batch".to_string()
    };

    RequestAnalysis {
        route_key,
        max_get_logs_range,
    }
}

fn get_logs_range(filter: &Value) -> Option<u64> {
    if filter.get("blockHash").is_some() {
        return Some(1);
    }

    let from = parse_block_tag(
        filter
            .get("fromBlock")
            .unwrap_or(&Value::String("latest".into())),
    )?;
    let to = parse_block_tag(
        filter
            .get("toBlock")
            .unwrap_or(&Value::String("latest".into())),
    )?;
    Some(to.saturating_sub(from) + 1)
}
