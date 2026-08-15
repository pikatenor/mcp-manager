//! Minimal newline-delimited JSON-RPC MCP server for connector tests.

use std::io::{self, BufRead, Write};

use serde_json::{json, Value};

fn main() {
    let secret = std::env::var("TEST_SECRET").unwrap_or_default();
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let line = line.expect("stdin");
        if line.trim().is_empty() {
            continue;
        }
        let request: Value = serde_json::from_str(&line).expect("json");
        if request.get("id").is_none() {
            continue;
        }
        let id = request["id"].clone();
        let method = request["method"].as_str().unwrap_or("");
        let result = match method {
            "initialize" => json!({
                "protocolVersion": "2025-03-26",
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "stdio-fixture", "version": "0" }
            }),
            "ping" => json!({}),
            "tools/list" => json!({
                "tools": [{
                    "name": "echo",
                    "description": "echo",
                    "inputSchema": { "type": "object", "properties": {} }
                }]
            }),
            "tools/call" => {
                let arguments = request["params"]["arguments"].clone();
                json!({
                    "content": [{
                        "type": "text",
                        "text": format!("secret={secret} args={arguments}")
                    }],
                    "isError": false
                })
            }
            other => json!({ "unsupported": other }),
        };
        let response = json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result
        });
        writeln!(stdout, "{response}").expect("stdout");
        stdout.flush().expect("flush");
    }
}
