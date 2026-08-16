//! ApexOS session JSONL → trajectory examples (D13).
//!
//! Pure split / classify / scan; the directory walk is the only I/O. Never opens a live
//! `AGENTD_LOG` (refuses if we can see it). Traces do not become Cerebro memories.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

use crate::convert::filter::Rejection;
use crate::convert::Converted;
use crate::error::{Error, Result};
use crate::example::Example;
use crate::provenance::{InstructionKind, LicenseClass, Provenance};

mod secret;

pub use secret::looks_like_secret;

/// ApexOS worker sessions live in `[1<<62, 1<<63)`. Spawn is `>= 1<<63`.
const WORKER_SESSION_BASE: u64 = 1 << 62;
const SPAWN_SESSION_BASE: u64 = 1 << 63;

/// Model-id prefixes that are closed-API hidden reasoning. Promotion off this
/// list is an allowlist edit, dated in `docs/harvest.md`.
const CLOSED_PREFIXES: &[&str] = &["claude-", "gpt-", "o1-", "o3-", "o4-", "grok-", "gemini-"];

/// How a session directory is mined.
#[derive(Debug, Clone)]
pub struct HarvestConfig {
    /// Overrides the directory name / sidecar `node_id`.
    pub node_id: Option<String>,
    /// Refuse a round whose any tool result exceeds this (chars). Default 4000.
    pub max_tool_result_chars: usize,
    /// Extra `open_reasoning` prefixes. Empty in production — the harvest.md
    /// allowlist is empty until verified on the day of the first mine.
    pub open_prefixes: Vec<String>,
}

impl Default for HarvestConfig {
    fn default() -> Self {
        Self {
            node_id: None,
            max_tool_result_chars: 4000,
            open_prefixes: Vec::new(),
        }
    }
}

impl HarvestConfig {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Mine a copied export directory. `memories_used + rejections.total()` is the
/// number of rounds-or-skipped-sessions considered — nothing vanishes.
pub fn mine_sessions(dir: &Path, cfg: &HarvestConfig) -> Result<Converted> {
    if is_live_sessions_dir(dir) {
        return Err(Error::LiveSessionsDir(dir.to_path_buf()));
    }
    let files = collect_jsonl(dir)?;
    if files.is_empty() {
        return Err(Error::NoSessionFiles(dir.to_path_buf()));
    }

    let bundle = read_bundle(dir);
    let node = cfg
        .node_id
        .clone()
        .or_else(|| bundle.as_ref().and_then(|b| b.node_id.clone()))
        .or_else(|| {
            dir.file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| "unknown".into());

    let mut out = Converted::default();
    for path in files {
        convert_file(&path, &node, bundle.as_ref(), cfg, &mut out)?;
    }
    Ok(out)
}

/// True when `dir` is this host's live `<AGENTD_LOG>/sessions`.
pub fn is_live_sessions_dir(dir: &Path) -> bool {
    let log = match std::env::var("AGENTD_LOG") {
        Ok(s) if !s.is_empty() => PathBuf::from(s),
        _ => return false,
    };
    same_sessions_dir(dir, &log)
}

fn same_sessions_dir(dir: &Path, agentd_log: &Path) -> bool {
    let live = agentd_log.join("sessions");
    match (dir.canonicalize(), live.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

fn collect_jsonl(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let rd = fs::read_dir(dir).map_err(|e| Error::io(dir, e))?;
    for ent in rd {
        let ent = ent.map_err(|e| Error::io(dir, e))?;
        let path = ent.path();
        if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
            files.push(path);
            continue;
        }
        if path.is_dir() && path.file_name().and_then(|n| n.to_str()) == Some("archive") {
            let inner = fs::read_dir(&path).map_err(|e| Error::io(&path, e))?;
            for ent in inner {
                let ent = ent.map_err(|e| Error::io(&path, e))?;
                let p = ent.path();
                if p.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                    files.push(p);
                }
            }
        }
    }
    files.sort();
    Ok(files)
}

#[derive(Debug, Deserialize)]
struct Bundle {
    #[serde(default)]
    node_id: Option<String>,
    #[serde(default)]
    sessions: Vec<SessionStamp>,
}

#[derive(Debug, Deserialize)]
struct SessionStamp {
    session_id: u64,
    #[serde(default)]
    agent_id: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    license_class: Option<LicenseClass>,
}

fn read_bundle(dir: &Path) -> Option<Bundle> {
    let path = dir.join("harvest.json");
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn convert_file(
    path: &Path,
    node: &str,
    bundle: Option<&Bundle>,
    cfg: &HarvestConfig,
    out: &mut Converted,
) -> Result<()> {
    let Some(session_id) = session_id_from_path(path) else {
        out.rejections.record(Rejection::Unparsable);
        return Ok(());
    };

    match session_kind(session_id) {
        SessionKind::Zero => {
            out.rejections.record(Rejection::SessionZero);
            return Ok(());
        }
        SessionKind::Spawn => {
            out.rejections.record(Rejection::Spawn);
            return Ok(());
        }
        SessionKind::Normal | SessionKind::Worker => {}
    }

    let stamp = bundle.and_then(|b| b.sessions.iter().find(|s| s.session_id == session_id));
    let text = fs::read_to_string(path).map_err(|e| Error::io(path, e))?;
    let messages = parse_messages(&text, &mut out.rejections);
    let rounds = split_rounds(&messages);

    for (turn_index, round) in rounds.into_iter().enumerate() {
        match convert_round(&round, node, session_id, turn_index as u32, stamp, cfg) {
            Ok(example) => {
                *out.framing.entry(InstructionKind::LivedTurn).or_insert(0) += 1;
                out.examples.push(example);
                out.memories_used += 1;
            }
            Err(reason) => out.rejections.record(reason),
        }
    }
    Ok(())
}

enum SessionKind {
    Zero,
    Normal,
    Worker,
    Spawn,
}

fn session_kind(id: u64) -> SessionKind {
    if id == 0 {
        SessionKind::Zero
    } else if id >= SPAWN_SESSION_BASE {
        SessionKind::Spawn
    } else if id >= WORKER_SESSION_BASE {
        SessionKind::Worker
    } else {
        SessionKind::Normal
    }
}

fn session_id_from_path(path: &Path) -> Option<u64> {
    let stem = path.file_stem()?.to_str()?;
    let rest = stem.strip_prefix("session-").unwrap_or(stem);
    rest.parse().ok()
}

/// ApexOS `Message` wire shape — local copy so this crate stays standalone (D1).
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case")]
enum ApexMessage {
    User { content: Vec<Block> },
    Assistant { content: Vec<Block> },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Block {
    Text {
        text: String,
    },
    Thinking {
        thinking: String,
        signature: String,
    },
    ToolUse {
        #[allow(dead_code)]
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        #[allow(dead_code)]
        tool_use_id: String,
        content: Value,
        is_error: bool,
    },
    Image {
        #[allow(dead_code)]
        media_type: String,
        #[allow(dead_code)]
        data: String,
    },
    #[serde(other)]
    Other,
}

fn parse_messages(
    text: &str,
    rejections: &mut crate::convert::filter::RejectionLedger,
) -> Vec<ApexMessage> {
    let mut out = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<ApexMessage>(line) {
            Ok(m) => out.push(m),
            Err(_) => rejections.record(Rejection::Unparsable),
        }
    }
    out
}

/// A new round starts on a user message that carries non-empty text.
fn starts_round(msg: &ApexMessage) -> bool {
    match msg {
        ApexMessage::User { content } => content.iter().any(|b| match b {
            Block::Text { text } => !text.trim().is_empty(),
            _ => false,
        }),
        ApexMessage::Assistant { .. } => false,
    }
}

fn split_rounds(messages: &[ApexMessage]) -> Vec<Vec<ApexMessage>> {
    let mut rounds = Vec::new();
    let mut cur: Vec<ApexMessage> = Vec::new();
    for msg in messages {
        if starts_round(msg) && !cur.is_empty() {
            rounds.push(std::mem::take(&mut cur));
        }
        cur.push(msg.clone());
    }
    if !cur.is_empty() {
        rounds.push(cur);
    }
    rounds
}

fn convert_round(
    round: &[ApexMessage],
    node: &str,
    session_id: u64,
    turn_index: u32,
    stamp: Option<&SessionStamp>,
    cfg: &HarvestConfig,
) -> std::result::Result<Example, Rejection> {
    let user_text = user_text(round);
    let has_image = round
        .iter()
        .any(|m| blocks(m).iter().any(|b| matches!(b, Block::Image { .. })));
    if user_text.trim().is_empty() && has_image {
        return Err(Rejection::ImageOnly);
    }
    if user_text.trim().is_empty() {
        return Err(Rejection::EmptyAssistant);
    }
    if crate::convert::filter::is_chatter(&user_text) {
        return Err(Rejection::Chatter);
    }

    if tool_result_too_long(round, cfg.max_tool_result_chars) {
        return Err(Rejection::ToolResultTooLong);
    }

    let hint = thinking_hint(round);
    let license = stamp
        .and_then(|s| s.license_class)
        .unwrap_or_else(|| classify(stamp.and_then(|s| s.model.as_deref()), hint.signed, cfg));
    let include_thinking = license == LicenseClass::OpenReasoning;
    let assistant = render_assistant(round, include_thinking);
    if assistant.trim().is_empty() {
        return Err(Rejection::EmptyAssistant);
    }

    let scan = format!("{user_text}\n{assistant}");
    if looks_like_secret(&scan) {
        return Err(Rejection::Secret);
    }

    Ok(Example::instruction(
        user_text,
        assistant,
        Provenance::SessionTurn {
            node_id: node.to_string(),
            session_id,
            turn_index,
            agent_id: stamp.and_then(|s| s.agent_id.clone()),
            license_class: license,
            model: stamp.and_then(|s| s.model.clone()),
        },
        InstructionKind::LivedTurn,
    ))
}

fn blocks(msg: &ApexMessage) -> &[Block] {
    match msg {
        ApexMessage::User { content } | ApexMessage::Assistant { content } => content,
    }
}

fn user_text(round: &[ApexMessage]) -> String {
    let mut parts = Vec::new();
    for msg in round {
        if let ApexMessage::User { content } = msg {
            for b in content {
                if let Block::Text { text } = b {
                    if !text.trim().is_empty() {
                        parts.push(text.clone());
                    }
                }
            }
        }
    }
    parts.join("\n")
}

fn thinking_hint(round: &[ApexMessage]) -> ThinkingHint {
    let mut signed = false;
    for msg in round {
        for b in blocks(msg) {
            if let Block::Thinking { signature, .. } = b {
                if !signature.trim().is_empty() {
                    signed = true;
                }
            }
        }
    }
    ThinkingHint { signed }
}

#[derive(Clone, Copy)]
struct ThinkingHint {
    signed: bool,
}

/// Explicit allowlist, never "we parsed a field so we may train on it."
pub fn classify(model: Option<&str>, signed_thinking: bool, cfg: &HarvestConfig) -> LicenseClass {
    if signed_thinking {
        return LicenseClass::ClosedHidden;
    }
    if let Some(m) = model {
        let lower = m.trim().to_ascii_lowercase();
        if CLOSED_PREFIXES.iter().any(|p| lower.starts_with(p)) {
            return LicenseClass::ClosedHidden;
        }
        if cfg
            .open_prefixes
            .iter()
            .any(|p| lower.starts_with(&p.to_ascii_lowercase()))
        {
            return LicenseClass::OpenReasoning;
        }
    }
    LicenseClass::AnswerOnly
}

fn tool_result_too_long(round: &[ApexMessage], max: usize) -> bool {
    for msg in round {
        for b in blocks(msg) {
            if let Block::ToolResult { content, .. } = b {
                if value_text(content).chars().count() > max {
                    return true;
                }
            }
        }
    }
    false
}

fn value_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn render_assistant(round: &[ApexMessage], include_thinking: bool) -> String {
    let mut parts = Vec::new();
    for msg in round {
        match msg {
            ApexMessage::Assistant { content } => {
                for b in content {
                    match b {
                        Block::Thinking { thinking, .. } if include_thinking => {
                            if !thinking.trim().is_empty() {
                                parts.push(format!("Thinking:\n{}", thinking.trim()));
                            }
                        }
                        Block::Text { text } if !text.trim().is_empty() => {
                            parts.push(text.clone());
                        }
                        Block::ToolUse { name, input, .. } => {
                            parts.push(format!("🔧 `{name}`({})", compact_json(input)));
                        }
                        _ => {}
                    }
                }
            }
            ApexMessage::User { content } => {
                for b in content {
                    if let Block::ToolResult {
                        content, is_error, ..
                    } = b
                    {
                        let body = value_text(content);
                        if *is_error {
                            parts.push(format!("⚠ {body}"));
                        } else {
                            parts.push(format!("↳ {body}"));
                        }
                    }
                }
            }
        }
    }
    parts.join("\n\n")
}

fn compact_json(v: &Value) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "{}".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::filter::RejectionLedger;

    fn user(text: &str) -> String {
        serde_json::json!({"role":"user","content":[{"type":"text","text":text}]}).to_string()
    }
    fn assistant(text: &str) -> String {
        serde_json::json!({"role":"assistant","content":[{"type":"text","text":text}]}).to_string()
    }
    fn thinking(text: &str, sig: &str) -> String {
        serde_json::json!({
            "role":"assistant",
            "content":[
                {"type":"thinking","thinking":text,"signature":sig},
                {"type":"text","text":"done."}
            ]
        })
        .to_string()
    }
    fn tool_round() -> String {
        let u = user("bind LAN on the studio box so apex1 can reach the proxy");
        let a = serde_json::json!({
            "role":"assistant",
            "content":[
                {"type":"text","text":"I'll flip the LAN extra bind."},
                {"type":"tool_use","id":"c1","name":"apexrouter_lan","input":{"on":true}}
            ]
        });
        let r = serde_json::json!({
            "role":"user",
            "content":[{
                "type":"tool_result",
                "tool_use_id":"c1",
                "content":"listening on 192.168.0.138:8888",
                "is_error":false
            }]
        });
        let a2 = assistant("LAN extra bind is up; claim on the proxy, not :2739.");
        format!("{u}\n{a}\n{r}\n{a2}\n")
    }

    fn write_session(dir: &Path, id: u64, body: &str) {
        fs::write(dir.join(format!("session-{id}.jsonl")), body).expect("write");
    }

    #[test]
    fn a_tool_round_becomes_one_lived_turn() {
        let dir = tempfile::TempDir::new().unwrap();
        write_session(dir.path(), 22, &tool_round());
        let out = mine_sessions(dir.path(), &HarvestConfig::new()).expect("mine");
        assert_eq!(out.examples.len(), 1);
        assert_eq!(out.memories_used, 1);
        assert_eq!(out.rejections.total(), 0);
        assert_eq!(out.memories_used + out.rejections.total(), 1);
        let ex = &out.examples[0];
        assert_eq!(ex.instruction_kind, InstructionKind::LivedTurn);
        assert!(ex.messages[0].content.contains("bind LAN"));
        assert!(ex.messages[1].content.contains("apexrouter_lan"));
        assert!(ex.messages[1].content.contains("192.168.0.138:8888"));
        match &ex.provenance {
            Provenance::SessionTurn {
                session_id,
                turn_index,
                license_class,
                ..
            } => {
                assert_eq!(*session_id, 22);
                assert_eq!(*turn_index, 0);
                assert_eq!(*license_class, LicenseClass::AnswerOnly);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn closed_thinking_is_stripped_and_open_thinking_is_kept_only_on_allowlist() {
        let dir = tempfile::TempDir::new().unwrap();
        let body = format!(
            "{}\n{}\n",
            user("why did the route table go empty after restart"),
            thinking("because vast-47728841 was not armed", "sig-anthropic")
        );
        write_session(dir.path(), 7, &body);
        let out = mine_sessions(dir.path(), &HarvestConfig::new()).expect("mine");
        assert_eq!(out.examples.len(), 1);
        assert!(
            !out.examples[0].messages[1]
                .content
                .contains("vast-47728841"),
            "closed CoT must not land in the example: {}",
            out.examples[0].messages[1].content
        );
        assert_eq!(
            match &out.examples[0].provenance {
                Provenance::SessionTurn { license_class, .. } => *license_class,
                _ => panic!("kind"),
            },
            LicenseClass::ClosedHidden
        );

        let dir2 = tempfile::TempDir::new().unwrap();
        let open = format!(
            "{}\n{}\n",
            user("why did the route table go empty after restart"),
            thinking("because vast-47728841 was not armed", "")
        );
        write_session(dir2.path(), 8, &open);
        fs::write(
            dir2.path().join("harvest.json"),
            r#"{"node_id":"apex1","sessions":[{"session_id":8,"model":"qwen-test-open"}]}"#,
        )
        .unwrap();
        let mut cfg = HarvestConfig::new();
        cfg.open_prefixes = vec!["qwen-test-open".into()];
        let kept = mine_sessions(dir2.path(), &cfg).expect("mine");
        assert!(
            kept.examples[0].messages[1]
                .content
                .contains("vast-47728841"),
            "open reasoning on the allowlist is kept"
        );
        assert_eq!(
            match &kept.examples[0].provenance {
                Provenance::SessionTurn { license_class, .. } => *license_class,
                _ => panic!("kind"),
            },
            LicenseClass::OpenReasoning
        );
    }

    #[test]
    fn unknown_thinking_is_not_promoted_to_open() {
        assert_eq!(
            classify(Some("studio-llm"), false, &HarvestConfig::new()),
            LicenseClass::AnswerOnly
        );
        assert_eq!(
            classify(Some("claude-opus"), false, &HarvestConfig::new()),
            LicenseClass::ClosedHidden
        );
        assert_eq!(
            classify(Some("gemini-2.5-pro"), false, &HarvestConfig::new()),
            LicenseClass::ClosedHidden
        );
    }

    #[test]
    fn session_zero_and_spawn_are_one_rejection_each() {
        let dir = tempfile::TempDir::new().unwrap();
        write_session(
            dir.path(),
            0,
            &format!("{}\n{}\n", user("sensor"), assistant("ok")),
        );
        write_session(
            dir.path(),
            SPAWN_SESSION_BASE,
            &format!("{}\n{}\n", user("spawn"), assistant("nope")),
        );
        write_session(dir.path(), WORKER_SESSION_BASE + 3, &tool_round());
        let out = mine_sessions(dir.path(), &HarvestConfig::new()).expect("mine");
        assert_eq!(out.rejections.count_of(Rejection::SessionZero), 1);
        assert_eq!(out.rejections.count_of(Rejection::Spawn), 1);
        assert_eq!(out.examples.len(), 1);
        assert_eq!(out.memories_used + out.rejections.total(), 3);
    }

    #[test]
    fn chatter_and_secrets_and_empty_assistant_are_distinct() {
        let dir = tempfile::TempDir::new().unwrap();
        write_session(
            dir.path(),
            1,
            &format!(
                "{}\n{}\n",
                user("Hey FORGE can you hear me"),
                assistant("yes")
            ),
        );
        write_session(
            dir.path(),
            2,
            &format!(
                "{}\n{}\n",
                user("put this in env"),
                assistant("TOGETHER_API_KEY=sk-not-a-real-key-value-at-all")
            ),
        );
        write_session(
            dir.path(),
            3,
            &format!("{}\n{}\n", user("what next"), assistant("")),
        );
        let out = mine_sessions(dir.path(), &HarvestConfig::new()).expect("mine");
        assert_eq!(out.examples.len(), 0);
        assert_eq!(out.rejections.count_of(Rejection::Chatter), 1);
        assert_eq!(out.rejections.count_of(Rejection::Secret), 1);
        assert_eq!(out.rejections.count_of(Rejection::EmptyAssistant), 1);
        assert_eq!(out.memories_used + out.rejections.total(), 3);
    }

    #[test]
    fn tool_result_over_cap_refuses_the_round() {
        let dir = tempfile::TempDir::new().unwrap();
        let huge = "x".repeat(50);
        let u = user("dump the log");
        let a = serde_json::json!({
            "role":"assistant",
            "content":[{"type":"tool_use","id":"c1","name":"run","input":{"cmd":"cat"}}]
        });
        let r = serde_json::json!({
            "role":"user",
            "content":[{"type":"tool_result","tool_use_id":"c1","content":huge,"is_error":false}]
        });
        write_session(dir.path(), 4, &format!("{u}\n{a}\n{r}\n"));
        let mut cfg = HarvestConfig::new();
        cfg.max_tool_result_chars = 20;
        let out = mine_sessions(dir.path(), &cfg).expect("mine");
        assert_eq!(out.rejections.count_of(Rejection::ToolResultTooLong), 1);
        assert!(out.examples.is_empty());
    }

    #[test]
    fn live_agentd_log_is_refused() {
        let live = tempfile::TempDir::new().unwrap();
        let sessions = live.path().join("sessions");
        fs::create_dir_all(&sessions).unwrap();
        write_session(&sessions, 1, &tool_round());
        assert!(same_sessions_dir(&sessions, live.path()));
        let other = tempfile::TempDir::new().unwrap();
        assert!(!same_sessions_dir(other.path(), live.path()));
    }

    #[test]
    fn empty_dir_is_an_honest_error() {
        let dir = tempfile::TempDir::new().unwrap();
        let err = mine_sessions(dir.path(), &HarvestConfig::new()).expect_err("empty");
        assert!(matches!(err, Error::NoSessionFiles(_)), "{err}");
    }

    #[test]
    fn split_attaches_tool_results_to_the_issuing_round() {
        let mut led = RejectionLedger::default();
        let msgs = parse_messages(&tool_round(), &mut led);
        let rounds = split_rounds(&msgs);
        assert_eq!(rounds.len(), 1);
        assert_eq!(rounds[0].len(), 4);
    }
}
