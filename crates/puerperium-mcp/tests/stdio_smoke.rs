//! Spawn the real `puerperium-mcp` binary and speak newline-delimited JSON-RPC
//! over its stdio. No upstream, no key, isolated state dir (D5).

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};

use serde_json::{json, Value};

struct Server {
    child: Child,
    reader: BufReader<std::process::ChildStdout>,
    next_id: u64,
}

impl Server {
    fn spawn(state_dir: &str, env_file: &str) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_puerperium-mcp"))
            .env("PUERPERIUM_STATE_DIR", state_dir)
            .env("PUERPERIUM_ENV_FILE", env_file)
            .env_remove("TOGETHER_API_KEY")
            .env("RUST_LOG", "warn")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn puerperium-mcp");
        let reader = BufReader::new(child.stdout.take().unwrap());
        let mut s = Self {
            child,
            reader,
            next_id: 1,
        };
        let init = s.request("initialize", json!({}));
        assert_eq!(init["result"]["serverInfo"]["name"], "puerperium-mcp");
        assert_eq!(init["result"]["protocolVersion"], "2024-11-05");
        s.notify("notifications/initialized");
        s
    }

    fn send(&mut self, v: &Value) {
        let stdin = self.child.stdin.as_mut().unwrap();
        let mut line = v.to_string();
        line.push('\n');
        stdin.write_all(line.as_bytes()).unwrap();
        stdin.flush().unwrap();
    }

    fn notify(&mut self, method: &str) {
        self.send(&json!({"jsonrpc":"2.0","method": method}));
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }));
        let mut line = String::new();
        self.reader
            .read_line(&mut line)
            .expect("read json-rpc line");
        let v: Value = serde_json::from_str(line.trim()).expect("parse json-rpc");
        assert_eq!(v["id"], id);
        v
    }

    fn call(&mut self, name: &str, arguments: Value) -> Value {
        self.request(
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
        )
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn refused_reason(resp: &Value) -> String {
    assert!(
        resp["error"].is_null(),
        "gates are tool results, not protocol errors: {resp}"
    );
    assert_eq!(resp["result"]["isError"], true);
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    let body: Value = serde_json::from_str(text).unwrap();
    assert_eq!(body["refused"], true, "{body}");
    body["reason"].as_str().unwrap().to_string()
}

fn payload(resp: &Value) -> Value {
    assert!(resp["error"].is_null(), "unexpected error: {resp}");
    assert!(resp["result"]["isError"].is_null() || resp["result"]["isError"] == false);
    serde_json::from_str(resp["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}

#[test]
fn agent_can_compose_the_nursery_surface() {
    let dir = tempfile::tempdir().expect("tempdir");
    let env_file = dir.path().join("empty.env");
    std::fs::write(&env_file, "").expect("empty env file so secrets::load stops here");
    let state = dir.path().join("state");
    std::fs::create_dir_all(&state).unwrap();

    let mut s = Server::spawn(state.to_str().unwrap(), env_file.to_str().unwrap());

    let listed = s.request("tools/list", json!({}));
    let names: Vec<String> = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    for expected in [
        "nursery_generate_data",
        "nursery_list_datasets",
        "nursery_inspect_dataset",
        "nursery_estimate_cost",
        "nursery_quote",
        "nursery_upload",
        "nursery_train",
        "nursery_job_status",
        "nursery_list_jobs",
        "nursery_cancel_job",
        "nursery_list_models",
        "nursery_register_model",
        "nursery_test_model",
        "nursery_create_apprentice",
        "nursery_list_apprentices",
        "nursery_lineage",
    ] {
        assert!(
            names.contains(&expected.to_string()),
            "missing {expected} in {names:?}"
        );
    }
    assert_eq!(names.len(), 16);
    assert!(!names.iter().any(|n| n.contains("rebirth")));

    let datasets = payload(&s.call("nursery_list_datasets", json!({})));
    assert_eq!(datasets["count"], 0);

    let jobs = payload(&s.call("nursery_list_jobs", json!({})));
    assert_eq!(jobs["count"], 0);

    let reason = refused_reason(&s.call(
        "nursery_train",
        json!({
            "id": "j1",
            "dataset": "missing",
            "output_name": "w",
            "training_file_id": "file-x",
            "dry_run": false,
        }),
    ));
    assert!(reason.contains("confirm"), "{reason}");

    let reason = refused_reason(&s.call(
        "nursery_test_model",
        json!({ "model": "worker-v1", "prompt": "hello" }),
    ));
    assert!(reason.contains("Watcher"), "{reason}");

    let reason = refused_reason(&s.call("nursery_quote", json!({ "training_file_id": "file-x" })));
    assert!(
        reason.contains("TOGETHER_API_KEY") || reason.contains("no API key"),
        "{reason}"
    );

    let ping = s.request("ping", json!({}));
    assert!(ping["error"].is_null(), "{ping}");
}
