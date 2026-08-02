//! Reading memories from a Cerebro SQLite snapshot.
//!
//! # Read-only, always
//!
//! A Cerebro database is another tool's state directory — and usually a **live daily
//! driver**. This module opens it `SQLITE_OPEN_READ_ONLY` and never writes, never migrates,
//! never creates. The same posture ApexRouter takes toward `~/.vastai-gguf/`: read it, never
//! write it.
//!
//! Prefer pointing this at a **snapshot** rather than a live file:
//!
//! ```text
//! sqlite3 /var/lib/cerebro/cerebro.db ".backup /tmp/snapshot.db"
//! ```
//!
//! `.backup` is safe against a database being written concurrently; copying the file is not.

use std::path::Path;

use rusqlite::{Connection, OpenFlags};

use crate::memory::{MemoryRecord, MemoryType};

#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error("could not open {path} read-only: {source}")]
    Open {
        path: String,
        #[source]
        source: rusqlite::Error,
    },

    #[error(
        "no `memories` table in {path} — if this is a pre-migration Python Cerebro database \
         (it would have `memory_nodes` instead), open it once with a current cerebro binary to \
         migrate it, then snapshot again"
    )]
    NotACerebroDb { path: String },

    #[error("query failed: {0}")]
    Query(#[from] rusqlite::Error),
}

/// What to pull out of a snapshot.
#[derive(Debug, Clone, Default)]
pub struct Query {
    /// Restrict to one agent's memory space. `None` reads every agent's.
    ///
    /// This selects **whose memories to mine**; it is not the trainer (charter D6).
    pub agent_id: Option<String>,
    /// Keep only memories carrying at least one of these tags (case-insensitive).
    pub any_tags: Vec<String>,
    /// Cap the number returned, highest salience first. `None` reads all.
    pub limit: Option<usize>,
}

/// Read memories from a Cerebro snapshot.
///
/// Soft-deleted rows are excluded — Cerebro's trash is not training data.
pub fn read(path: &Path, query: &Query) -> Result<Vec<MemoryRecord>, SourceError> {
    let display = path.display().to_string();
    let conn =
        Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|source| {
            SourceError::Open {
                path: display.clone(),
                source,
            }
        })?;

    if !has_table(&conn, "memories")? {
        return Err(SourceError::NotACerebroDb { path: display });
    }

    let mut sql = String::from(
        "SELECT id, content, memory_type, COALESCE(tags,'[]'), agent_id, COALESCE(salience, 0.5) \
         FROM memories WHERE deleted_at IS NULL",
    );
    if query.agent_id.is_some() {
        sql.push_str(" AND agent_id = ?1");
    }
    sql.push_str(" ORDER BY salience DESC");

    let mut stmt = conn.prepare(&sql)?;
    let map = |row: &rusqlite::Row<'_>| -> rusqlite::Result<(String, String, String, String, Option<String>, f64)> {
        Ok((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
            row.get(5)?,
        ))
    };
    let rows: Vec<_> = match &query.agent_id {
        Some(agent) => stmt.query_map([agent], map)?.collect::<Result<_, _>>()?,
        None => stmt.query_map([], map)?.collect::<Result<_, _>>()?,
    };

    let wanted: Vec<String> = query.any_tags.iter().map(|t| t.to_lowercase()).collect();
    let mut out = Vec::new();

    for (id, content, memory_type, tags_json, agent_id, salience) in rows {
        // An unrecognised memory_type is skipped rather than guessed into a variant — the
        // same refusal-to-guess that governs upstream job statuses.
        let Some(memory_type) = parse_type(&memory_type) else {
            continue;
        };
        let tags = parse_tags(&tags_json);

        if !wanted.is_empty() {
            let lower: Vec<String> = tags.iter().map(|t| t.to_lowercase()).collect();
            if !lower.iter().any(|t| wanted.contains(t)) {
                continue;
            }
        }

        out.push(MemoryRecord {
            id,
            content,
            memory_type,
            tags,
            agent_id: agent_id.clone(),
            salience: salience as f32,
        });

        if query.limit.is_some_and(|n| out.len() >= n) {
            break;
        }
    }

    Ok(out)
}

/// Agents present in a snapshot, with how many live memories each holds.
///
/// Answers "whose knowledge is even in here?" before committing to a mining run.
pub fn agents(path: &Path) -> Result<Vec<(String, usize)>, SourceError> {
    let display = path.display().to_string();
    let conn =
        Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|source| {
            SourceError::Open {
                path: display.clone(),
                source,
            }
        })?;
    if !has_table(&conn, "memories")? {
        return Err(SourceError::NotACerebroDb { path: display });
    }

    let mut stmt = conn.prepare(
        "SELECT COALESCE(agent_id,'(none)'), COUNT(*) FROM memories \
         WHERE deleted_at IS NULL GROUP BY 1 ORDER BY 2 DESC",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as usize))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn has_table(conn: &Connection, name: &str) -> Result<bool, rusqlite::Error> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
        [name],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

/// Cerebro stores tags as a JSON array string. A malformed value yields no tags rather than
/// failing the whole read — one bad row must not cost the other three hundred.
fn parse_tags(json: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(json).unwrap_or_default()
}

fn parse_type(raw: &str) -> Option<MemoryType> {
    serde_json::from_value(serde_json::Value::String(raw.to_lowercase())).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a snapshot shaped like Cerebro's real schema.
    fn fixture(dir: &Path) -> std::path::PathBuf {
        let path = dir.join("cerebro.db");
        let conn = Connection::open(&path).expect("create");
        conn.execute_batch(
            "CREATE TABLE memories (
                id TEXT PRIMARY KEY, content TEXT, memory_type TEXT, salience REAL,
                tags TEXT, agent_id TEXT, deleted_at TEXT
             );
             INSERT INTO memories VALUES
               ('m1','procedural body','procedural',0.9,'[\"deploy\",\"pi\"]','FORGE',NULL),
               ('m2','semantic body','semantic',0.7,'[\"mesh\"]','FORGE',NULL),
               ('m3','other agent','procedural',0.8,'[\"deploy\"]','APEX',NULL),
               ('m4','deleted one','procedural',0.9,'[\"deploy\"]','FORGE','2026-08-01'),
               ('m5','bad type','wat',0.9,'[\"deploy\"]','FORGE',NULL),
               ('m6','bad tags','semantic',0.6,'not json','FORGE',NULL);",
        )
        .expect("seed");
        path
    }

    #[test]
    fn reads_live_memories_and_excludes_the_trash() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = fixture(dir.path());
        let got = read(&db, &Query::default()).expect("read");

        let ids: Vec<&str> = got.iter().map(|m| m.id.as_str()).collect();
        assert!(
            !ids.contains(&"m4"),
            "soft-deleted memories are not training data"
        );
        assert!(ids.contains(&"m1") && ids.contains(&"m2") && ids.contains(&"m3"));
    }

    #[test]
    fn an_unrecognised_memory_type_is_skipped_not_guessed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = fixture(dir.path());
        let got = read(&db, &Query::default()).expect("read");
        assert!(
            !got.iter().any(|m| m.id == "m5"),
            "'wat' must not become a variant"
        );
    }

    #[test]
    fn a_malformed_tags_value_costs_only_its_own_tags() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = fixture(dir.path());
        let got = read(&db, &Query::default()).expect("read");
        let m6 = got.iter().find(|m| m.id == "m6").expect("row still read");
        assert!(
            m6.tags.is_empty(),
            "bad tags parse to none, the row survives"
        );
    }

    #[test]
    fn agent_filter_selects_one_memory_space() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = fixture(dir.path());
        let got = read(
            &db,
            &Query {
                agent_id: Some("FORGE".into()),
                ..Query::default()
            },
        )
        .expect("read");
        assert!(got.iter().all(|m| m.agent_id.as_deref() == Some("FORGE")));
        assert!(!got.iter().any(|m| m.id == "m3"));
    }

    #[test]
    fn tag_filter_is_case_insensitive_and_matches_any() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = fixture(dir.path());
        let got = read(
            &db,
            &Query {
                any_tags: vec!["DEPLOY".into()],
                ..Query::default()
            },
        )
        .expect("read");
        let ids: Vec<&str> = got.iter().map(|m| m.id.as_str()).collect();
        assert!(ids.contains(&"m1") && ids.contains(&"m3"));
        assert!(!ids.contains(&"m2"), "mesh-only memory must not match");
    }

    #[test]
    fn results_are_highest_salience_first_so_a_limit_keeps_the_best() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = fixture(dir.path());
        let got = read(
            &db,
            &Query {
                limit: Some(2),
                ..Query::default()
            },
        )
        .expect("read");
        assert_eq!(got.len(), 2);
        assert!(got[0].salience >= got[1].salience);
    }

    #[test]
    fn agents_reports_who_is_in_there() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = fixture(dir.path());
        let got = agents(&db).expect("agents");
        let forge = got
            .iter()
            .find(|(a, _)| a == "FORGE")
            .expect("FORGE present");
        assert_eq!(
            forge.1, 4,
            "live rows only — the deleted one is not counted"
        );
    }

    /// A pre-migration Python database should say so, not fail cryptically.
    #[test]
    fn a_non_cerebro_database_names_the_likely_cause() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("other.db");
        Connection::open(&path)
            .expect("create")
            .execute_batch("CREATE TABLE memory_nodes (id TEXT);")
            .expect("seed");

        let err = read(&path, &Query::default()).expect_err("must refuse");
        assert!(
            matches!(err, SourceError::NotACerebroDb { .. }),
            "got {err:?}"
        );
        assert!(
            err.to_string().contains("memory_nodes"),
            "should name the legacy table"
        );
    }

    /// The database is another tool's state; opening it must never create one.
    #[test]
    fn opening_a_missing_file_does_not_create_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("nope.db");
        assert!(read(&path, &Query::default()).is_err());
        assert!(!path.exists(), "read-only open must not create the file");
    }
}
