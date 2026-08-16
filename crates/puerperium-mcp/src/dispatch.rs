//! JSON-RPC dispatch — handshake, tools/list, tools/call.
//!
//! Gates (no key, no confirm, not implemented) are tool *results* with
//! `isError: true` and `{"refused": true, ...}` — the tool answered no (D8).
//! Unknown tools and bad arguments are protocol errors. A handler panic is
//! isolated on a dedicated task so it cannot take the process down.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::handlers::{CallError, Face};
use crate::tools;

pub fn handle_initialize(req: &Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": req["id"],
        "result": {
            "protocolVersion": "2024-11-05",
            "capabilities": { "tools": {} },
            "serverInfo": {
                "name": "puerperium-mcp",
                "version": env!("CARGO_PKG_VERSION")
            }
        }
    })
}

pub fn method_not_found(req: &Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": req["id"],
        "error": { "code": -32601, "message": "method not found" }
    })
}

pub fn tools_list(req: &Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": req["id"],
        "result": { "tools": tools::all_tool_schemas() }
    })
}

pub fn ping(req: &Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": req["id"],
        "result": {}
    })
}

/// Route a `tools/call`, isolating handler panics so a fault cannot unwind
/// into the main loop.
pub async fn dispatch_tool(msg: Value, face: Arc<Face>) -> Value {
    let id = msg["id"].clone();

    let handle = tokio::spawn(async move {
        let params = &msg["params"];
        let name = params["name"].as_str().unwrap_or("").to_string();
        let args = params["arguments"].clone();
        face.call(&name, &args)
    });

    match handle.await {
        Ok(Ok(v)) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "content": [{ "type": "text", "text": v.to_string() }] }
        }),
        Ok(Err(CallError::UnknownTool(msg))) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": -32601, "message": msg }
        }),
        Ok(Err(CallError::InvalidArgs(msg))) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": -32602, "message": msg }
        }),
        Ok(Err(CallError::Refused { reason })) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "isError": true,
                "content": [{
                    "type": "text",
                    "text": json!({ "refused": true, "reason": reason }).to_string()
                }]
            }
        }),
        Ok(Err(CallError::Failed(msg))) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "isError": true,
                "content": [{
                    "type": "text",
                    "text": json!({ "error": msg }).to_string()
                }]
            }
        }),
        Err(join_err) => {
            tracing::error!("tool handler panicked: {join_err}");
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32603, "message": "internal error: tool handler panicked" }
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use puerperium::paths::Paths;

    fn face() -> (tempfile::TempDir, Arc<Face>) {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let paths = Paths::new(dir.path());
        paths.ensure().expect("ensure");
        let face = Arc::new(Face { paths });
        (dir, face)
    }

    #[test]
    fn initialize_echoes_id_and_names_the_server() {
        let req = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}});
        let resp = handle_initialize(&req);
        assert_eq!(resp["id"], 1);
        assert_eq!(resp["result"]["serverInfo"]["name"], "puerperium-mcp");
        assert_eq!(resp["result"]["protocolVersion"], "2024-11-05");
    }

    #[test]
    fn tools_list_advertises_every_nursery_name() {
        let resp = tools_list(&json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}));
        let names: Vec<String> = resp["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect();
        for expected in tools::TOOL_NAMES {
            assert!(
                names.contains(&expected.to_string()),
                "must advertise {expected}: {names:?}"
            );
        }
        assert_eq!(names.len(), tools::TOOL_NAMES.len());
        assert!(!names.iter().any(|n| n.contains("rebirth")));
    }

    #[tokio::test]
    async fn unknown_tool_is_protocol_not_found() {
        let msg = json!({
            "jsonrpc":"2.0","id":7,"method":"tools/call",
            "params":{"name":"nursery_rebirth_score","arguments":{}}
        });
        let (_dir, face) = face();
        let resp = dispatch_tool(msg, face).await;
        assert_eq!(resp["error"]["code"], -32601);
    }

    #[tokio::test]
    async fn missing_args_are_invalid_params() {
        let msg = json!({
            "jsonrpc":"2.0","id":8,"method":"tools/call",
            "params":{"name":"nursery_inspect_dataset","arguments":{}}
        });
        let (_dir, face) = face();
        let resp = dispatch_tool(msg, face).await;
        assert_eq!(resp["error"]["code"], -32602);
        assert!(resp["error"]["message"].as_str().unwrap().contains("name"));
    }

    #[tokio::test]
    async fn test_model_refusal_is_a_tool_result() {
        let msg = json!({
            "jsonrpc":"2.0","id":9,"method":"tools/call",
            "params":{"name":"nursery_test_model","arguments":{"model":"m","prompt":"hi"}}
        });
        let (_dir, face) = face();
        let resp = dispatch_tool(msg, face).await;
        assert!(resp["error"].is_null(), "gates are not protocol errors");
        assert_eq!(resp["result"]["isError"], true);
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let body: Value = serde_json::from_str(text).unwrap();
        assert_eq!(body["refused"], true);
        assert!(body["reason"].as_str().unwrap().contains("Watcher"));
    }
}
