use serde_json::Value;

use crate::util::parse_block_tag;

#[derive(Clone, Debug)]
pub struct RequestAnalysis {
    pub route_key: String,
    pub max_get_logs_range: Option<u64>,
    pub lowest_block: Option<u64>,
    pub requires_archive: bool,
}

pub fn analyze_body(body: &[u8]) -> RequestAnalysis {
    let Ok(value) = serde_json::from_slice::<Value>(body) else {
        return RequestAnalysis {
            route_key: "invalid-json".to_string(),
            max_get_logs_range: None,
            lowest_block: None,
            requires_archive: false,
        };
    };

    let calls = match value {
        Value::Array(items) => items,
        item => vec![item],
    };

    let mut methods = Vec::new();
    let mut ranges = Vec::new();
    let mut block_numbers = Vec::new();
    let mut requires_archive = false;

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
                let logs_analysis = analyze_get_logs_filter(filter);
                if let Some(range) = logs_analysis.range {
                    ranges.push(range);
                }
                block_numbers.extend(logs_analysis.blocks);
                requires_archive |= logs_analysis.requires_archive;
            }
        } else if let Some(block) = method_block_tag(&call, method) {
            block_numbers.push(block);
        }
    }

    let max_get_logs_range = ranges.into_iter().max();
    let lowest_block = block_numbers.into_iter().min();
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
        lowest_block,
        requires_archive,
    }
}

impl RequestAnalysis {
    pub fn prefers_archive_pool(&self, reference_latest_block: u64, recent_window: u64) -> bool {
        if self.requires_archive {
            return true;
        }

        let Some(lowest_block) = self.lowest_block else {
            return false;
        };

        let recent_floor = reference_latest_block.saturating_sub(recent_window.saturating_sub(1));
        lowest_block < recent_floor
    }
}

#[derive(Debug)]
struct LogsAnalysis {
    range: Option<u64>,
    blocks: Vec<u64>,
    requires_archive: bool,
}

fn analyze_get_logs_filter(filter: &Value) -> LogsAnalysis {
    if filter.get("blockHash").is_some() {
        return LogsAnalysis {
            range: Some(1),
            blocks: Vec::new(),
            requires_archive: true,
        };
    }

    let from = parse_block_tag(
        filter
            .get("fromBlock")
            .unwrap_or(&Value::String("latest".into())),
    );
    let to = parse_block_tag(
        filter
            .get("toBlock")
            .unwrap_or(&Value::String("latest".into())),
    );

    let mut blocks = Vec::new();
    if let Some(from) = from {
        blocks.push(from);
    }
    if let Some(to) = to {
        blocks.push(to);
    }

    LogsAnalysis {
        range: from.zip(to).map(|(from, to)| to.saturating_sub(from) + 1),
        blocks,
        requires_archive: false,
    }
}

fn method_block_tag(call: &Value, method: &str) -> Option<u64> {
    let params = call.get("params").and_then(Value::as_array)?;
    let index = match method {
        "eth_getBalance" | "eth_getCode" | "eth_getTransactionCount" => 1,
        "eth_getStorageAt" => 2,
        "eth_call" | "eth_getProof" => params.len().saturating_sub(1),
        _ => return None,
    };

    params.get(index).and_then(parse_block_tag)
}
