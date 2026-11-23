use serde_json::{Value, json};
use std::io::Write;
use std::path::{Path, PathBuf};

pub fn path_in(dir: &Path) -> PathBuf {
    dir.join("mcp-audit.jsonl")
}

pub struct Entry<'a> {
    pub time: i64,
    pub tool: &'a str,
    pub account: &'a str,
    pub recipients: &'a [String],
    pub subject: &'a str,
    pub message_id: &'a str,
    pub result: &'a str,
}

pub fn write(dir: &Path, e: &Entry) -> Result<(), String> {
    let when = chrono::DateTime::from_timestamp(e.time, 0).map(|d| d.to_rfc3339()).unwrap_or_default();
    let line = json!({
        "time": when,
        "unix": e.time,
        "tool": e.tool,
        "account": e.account,
        "recipients": e.recipients,
        "subject": e.subject,
        "message_id": e.message_id,
        "result": e.result,
    });
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path_in(dir)).map_err(|e| format!("audit file: {e}"))?;
    writeln!(file, "{line}").map_err(|e| format!("audit file: {e}"))
}

pub fn sent_since(dir: &Path, account: &str, since: i64) -> usize {
    let Ok(text) = std::fs::read_to_string(path_in(dir)) else { return 0 };
    text.lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|v| v["tool"] == "send_message" && v["result"] == "sent" && v["account"] == account && v["unix"].as_i64().unwrap_or(0) >= since)
        .count()
}
