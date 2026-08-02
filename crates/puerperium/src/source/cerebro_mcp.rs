//! A minimal Cerebro MCP client, for writing lineage back.
//!
//! Cerebro speaks newline-delimited JSON-RPC over stdio. This spawns `cerebro-mcp`, performs
//! the handshake, and calls one tool. It is deliberately small — mirroring the shape
//! Prefrontal-RS uses in `core/cortex.rs`, which is the proven pattern in this garden for an
//! MCP *client* rather than server.
//!
//! # What gets written
//!
//! Lineage events are **ordinary tagged memories** (`nursery`, `nursery:<event>`, plus the
//! model and dataset). No new Cerebro schema, no Cerebro changes (charter D2's spirit applied
//! to the memory side).
//!
//! # `agent_id` here is ours
//!
//! When Puerperium runs standalone it supplies its own `agent_id`. Under agentd it would be
//! **stamped** and whatever we pass is overwritten — which is exactly why `trainer_agent`
//! lives in its own field on the records (D6) rather than riding in `agent_id`.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};

use serde_json::Value;

#[derive(Debug, thiserror::Error)]
pub enum McpError {
    #[error("could not start {cmd:?}: {source}")]
    Spawn {
        cmd: String,
        #[source]
        source: std::io::Error,
    },

    #[error("cerebro closed the connection before answering (is {cmd:?} the right binary?)")]
    Closed { cmd: String },

    #[error("cerebro returned an error: {0}")]
    Rpc(String),

    #[error("io talking to cerebro: {0}")]
    Io(#[from] std::io::Error),

    #[error("cerebro sent something unparsable: {0}")]
    Malformed(String),
}

/// One tool call's worth of conversation with a Cerebro process.
///
/// Spawned per call rather than held open: writing a lineage event happens once per deploy,
/// and a long-lived child would need supervision that buys nothing here.
pub struct CerebroMcp {
    cmd: String,
}

impl CerebroMcp {
    /// `$PUERPERIUM_CEREBRO_MCP`, else `cerebro-mcp` on `PATH`.
    pub fn from_env() -> Self {
        Self {
            cmd: std::env::var("PUERPERIUM_CEREBRO_MCP")
                .unwrap_or_else(|_| "cerebro-mcp".to_string()),
        }
    }

    pub fn command(&self) -> &str {
        &self.cmd
    }

    /// Call one tool, returning its result value.
    pub fn call_tool(&self, name: &str, arguments: Value) -> Result<Value, McpError> {
        let mut child = Command::new(&self.cmd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Cerebro logs to stderr; stdout is sacred JSON-RPC. Let its logs through to
            // ours rather than swallowing a diagnosis.
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|source| McpError::Spawn {
                cmd: self.cmd.clone(),
                source,
            })?;

        let result = self.converse(&mut child, name, arguments);

        // Always reap: an orphaned cerebro holding the database open is worse than a failed
        // write, because the next run then fails for an unrelated-looking reason.
        let _ = child.kill();
        let _ = child.wait();
        result
    }

    fn converse(&self, child: &mut Child, name: &str, arguments: Value) -> Result<Value, McpError> {
        let mut stdin = child.stdin.take().ok_or(McpError::Closed {
            cmd: self.cmd.clone(),
        })?;
        let stdout = child.stdout.take().ok_or(McpError::Closed {
            cmd: self.cmd.clone(),
        })?;
        let mut reader = BufReader::new(stdout);

        let send = |w: &mut dyn Write, v: &Value| -> Result<(), McpError> {
            let mut line =
                serde_json::to_string(v).map_err(|e| McpError::Malformed(e.to_string()))?;
            line.push('\n');
            w.write_all(line.as_bytes())?;
            w.flush()?;
            Ok(())
        };

        send(
            &mut stdin,
            &serde_json::json!({
                "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": {
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": { "name": "puerperium", "version": env!("CARGO_PKG_VERSION") }
                }
            }),
        )?;
        read_result(&mut reader, 1, &self.cmd)?;

        // Notifications take no response — sending one and then waiting would hang.
        send(
            &mut stdin,
            &serde_json::json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
        )?;

        send(
            &mut stdin,
            &serde_json::json!({
                "jsonrpc": "2.0", "id": 2, "method": "tools/call",
                "params": { "name": name, "arguments": arguments }
            }),
        )?;
        read_result(&mut reader, 2, &self.cmd)
    }
}

/// Read until the response bearing `id` arrives, skipping notifications and other traffic.
fn read_result(reader: &mut impl BufRead, id: i64, cmd: &str) -> Result<Value, McpError> {
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            // Stray non-JSON on stdout is Cerebro's own invariant to keep; skip rather than
            // die, so one stray line does not lose an otherwise good answer.
            continue;
        };
        if v.get("id").and_then(Value::as_i64) != Some(id) {
            continue;
        }
        if let Some(err) = v.get("error") {
            return Err(McpError::Rpc(err.to_string()));
        }
        return Ok(v.get("result").cloned().unwrap_or(Value::Null));
    }
    Err(McpError::Closed {
        cmd: cmd.to_string(),
    })
}

/// The arguments for a lineage event. Pure, so the shape is testable without a process.
///
/// `trainer_agent` rides in the **content**, not in `agent_id` — under agentd the latter is
/// stamped and would silently become whoever called (D6).
pub fn lineage_event_args(
    event: &str,
    model: &str,
    dataset: Option<&str>,
    dataset_sha256: Option<&str>,
    trainer_agent: &str,
    agent_id: &str,
    detail: &str,
) -> Value {
    let mut tags = vec![
        "nursery".to_string(),
        format!("nursery:{event}"),
        format!("model:{model}"),
    ];
    if let Some(d) = dataset {
        tags.push(format!("dataset:{d}"));
    }

    let mut content =
        format!("[nursery:{event}] {model} — {detail}\ntrainer_agent: {trainer_agent}");
    if let (Some(d), Some(h)) = (dataset, dataset_sha256) {
        content.push_str(&format!("\ndataset: {d} ({h})"));
    }

    serde_json::json!({
        "content": content,
        "memory_type": "episodic",
        "tags": tags,
        "agent_id": agent_id,
        "salience": 0.8,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lineage_args_carry_the_hash_and_tag_for_retrieval() {
        let args = lineage_event_args(
            "model_registered",
            "apexos-worker-v1",
            Some("ap-deploy-data"),
            Some("9f3ee896831c"),
            "FORGE",
            "FORGE",
            "registered as Router alias apexos-worker",
        );

        let tags: Vec<&str> = args["tags"]
            .as_array()
            .expect("tags")
            .iter()
            .map(|t| t.as_str().expect("str"))
            .collect();
        assert!(tags.contains(&"nursery"));
        assert!(tags.contains(&"nursery:model_registered"));
        assert!(tags.contains(&"model:apexos-worker-v1"));
        assert!(tags.contains(&"dataset:ap-deploy-data"));

        let content = args["content"].as_str().expect("content");
        assert!(
            content.contains("9f3ee896831c"),
            "the hash is the lineage link"
        );
        assert!(content.contains("trainer_agent: FORGE"));
    }

    /// Under agentd `agent_id` is stamped; attribution must not depend on it.
    #[test]
    fn trainer_agent_rides_in_content_not_in_agent_id() {
        let args = lineage_event_args(
            "model_registered",
            "m",
            None,
            None,
            "FORGE",
            "APEX", // a different, stamped identity
            "d",
        );
        assert_eq!(args["agent_id"], "APEX");
        assert!(
            args["content"]
                .as_str()
                .expect("content")
                .contains("trainer_agent: FORGE"),
            "the trainer must survive a stamped agent_id"
        );
    }

    #[test]
    fn a_dataset_without_a_hash_omits_the_line_rather_than_half_stating_it() {
        let args = lineage_event_args("e", "m", Some("d"), None, "FORGE", "FORGE", "x");
        assert!(!args["content"]
            .as_str()
            .expect("content")
            .contains("dataset: d ("));
    }

    #[test]
    fn a_response_for_another_id_is_skipped_not_returned() {
        let stream = "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"a\":1}}\n\
                      {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\"}\n\
                      {\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"b\":2}}\n";
        let got = read_result(&mut stream.as_bytes(), 2, "x").expect("read");
        assert_eq!(got["b"], 2);
    }

    #[test]
    fn stray_non_json_on_stdout_does_not_lose_the_answer() {
        let stream = "not json at all\n{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"ok\":true}}\n";
        let got = read_result(&mut stream.as_bytes(), 1, "x").expect("read");
        assert_eq!(got["ok"], true);
    }

    #[test]
    fn an_rpc_error_is_surfaced_not_swallowed() {
        let stream = "{\"jsonrpc\":\"2.0\",\"id\":1,\"error\":{\"code\":-32601,\"message\":\"no such tool\"}}\n";
        let err = read_result(&mut stream.as_bytes(), 1, "x").expect_err("must fail");
        assert!(err.to_string().contains("no such tool"), "got {err}");
    }

    #[test]
    fn a_closed_stream_says_which_binary_it_tried() {
        let err = read_result(&mut "".as_bytes(), 1, "cerebro-mcp").expect_err("must fail");
        assert!(err.to_string().contains("cerebro-mcp"), "got {err}");
    }
}
