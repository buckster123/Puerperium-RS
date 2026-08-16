//! Splitting a memory into teachable units.
//!
//! Sized against the real store: 22 of 59 procedural memories carry `##` sections, and the
//! longest runs 6289 characters. A sectioned reference document yields one chunk per
//! section; everything else stays whole.
//!
//! **Unsectioned content is never paragraph-split.** A lesson cut mid-thought produces two
//! half-lessons, which is worse training data than one long one.

/// One teachable unit, with the heading trail that frames it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// `["VLLM SERVING REFERENCE", "1. CORE CLI", "Essential Flags"]`. May be empty.
    pub heading_path: Vec<String>,
    pub body: String,
}

#[derive(Debug, Clone)]
pub struct ChunkConfig {
    /// Sections shorter than this merge forward into the next.
    pub min_section: usize,
    /// Chunks longer than this split at paragraph boundaries.
    pub max_chunk: usize,
    /// Longest first line still treated as a document title.
    pub max_title: usize,
}

impl Default for ChunkConfig {
    fn default() -> Self {
        Self {
            min_section: 120,
            max_chunk: 6000,
            max_title: 120,
        }
    }
}

/// Split content into chunks.
///
/// Pure. Always returns at least one chunk for non-empty content.
pub fn chunk(content: &str, cfg: &ChunkConfig) -> Vec<Chunk> {
    let content = content.trim();
    if content.is_empty() {
        return Vec::new();
    }

    let title = document_title(content, cfg.max_title);
    let sections = split_sections(content, title.as_deref());

    let chunks: Vec<Chunk> = match sections {
        // No headings: one chunk, whole. Deliberately not paragraph-split.
        None => {
            let path = title.clone().into_iter().collect();
            vec![Chunk {
                heading_path: path,
                body: content.to_string(),
            }]
        }
        Some(secs) => merge_short_forward(secs, cfg.min_section),
    };

    chunks
        .into_iter()
        .flat_map(|c| split_oversized(c, cfg.max_chunk))
        .collect()
}

/// The first line, when it reads like a title.
///
/// Two shapes, both from the live store:
///
/// - **Short banner** (≤ `max_title`, not a sentence). The VLLM-style reference doc.
/// - **Labelled banner** (`PROCEDURE — …`, `ARCHITECTURE DECISION …`, `LABEL — rest`).
///   FORGE's lived procedures write the title as a long sentence. The 120-char /
///   no-period rule dropped every one of them, so a 314-memory FORGE mine framed
///   61 of 77 examples from tags. A labelled banner is a title even when it is
///   long and even when it ends with a period.
///
/// A `##`+ line is a *section*, never the document title — otherwise the same line would be
/// consumed twice, once as the title and once as the first heading.
fn document_title(content: &str, max_title: usize) -> Option<String> {
    let first = content.lines().next()?.trim();
    if first.is_empty() || heading_of(first).is_some() {
        return None;
    }

    let stripped = first.trim_start_matches('#').trim();
    if stripped.is_empty() {
        return None;
    }
    if is_labelled_banner(stripped) {
        return Some(stripped.to_string());
    }
    if stripped.chars().count() > max_title {
        return None;
    }

    // An ATX `#` heading is a title by construction. Otherwise require it to not end like
    // a sentence — a reference doc starts with a banner, prose starts with a statement.
    if first.starts_with('#') || !stripped.ends_with('.') {
        Some(stripped.to_string())
    } else {
        None
    }
}

/// A first line that names the document, even when it is long or sentence-shaped.
///
/// Conservative on purpose: a prose opener that happens to contain an em-dash
/// later in the sentence is not a title. The dash has to land in the first 50
/// characters, and the left-hand side has to look like a label.
fn is_labelled_banner(line: &str) -> bool {
    let head = line
        .chars()
        .take(48)
        .collect::<String>()
        .to_ascii_uppercase();
    if head.starts_with("PROCEDURE ")
        || head.starts_with("PROCEDURE:")
        || head.starts_with("PROCEDURE—")
        || head.starts_with("ARCHITECTURE DECISION")
    {
        return true;
    }
    labelled_emdash_title(line)
}

/// `LABEL — rest` where the dash is a title separator, not list punctuation.
///
/// Real titles: `THE CALLOSUM FIELD LOOP — how FORGE field-proves…`,
/// `ApexOS-RS — VISION TOOL-RESULT PATTERN…`.
/// Not a title: `…binary updates: (1) cargo test — all pass` (dash at column 70).
fn labelled_emdash_title(line: &str) -> bool {
    let Some(idx) = line.find(" — ") else {
        return false;
    };
    if idx == 0 || idx > 50 {
        return false;
    }
    looks_like_label(&line[..idx])
}

fn looks_like_label(s: &str) -> bool {
    let s = s.trim();
    if s.is_empty() || s.ends_with('.') {
        return false;
    }
    let words = s.split_whitespace().count();
    (1..=8).contains(&words) && s.chars().count() <= 50
}

/// Split on `##`+ headings. Returns `None` when the content has no headings at all.
///
/// When a document title was lifted from the first line, that line is **skipped** here: it
/// already lives in every chunk's `heading_path`, and leaving it in the body would emit a
/// one-line preamble chunk that then merges forward and overwrites the real section's path.
/// (Unsectioned content keeps its first line, because there the body is the whole memory.)
fn split_sections(content: &str, title: Option<&str>) -> Option<Vec<Chunk>> {
    let base: Vec<String> = title.map(|t| vec![t.to_string()]).unwrap_or_default();

    let mut sections: Vec<Chunk> = Vec::new();
    let mut trail: Vec<(usize, String)> = Vec::new(); // (level, text)
    let mut current: Vec<&str> = Vec::new();
    let mut current_path = base.clone();
    let mut saw_heading = false;

    let skip = usize::from(title.is_some());
    for line in content.lines().skip(skip) {
        match heading_of(line) {
            Some((level, text)) => {
                saw_heading = true;
                flush(&mut sections, &mut current, &current_path);

                trail.retain(|(l, _)| *l < level);
                trail.push((level, text));

                current_path = base.clone();
                current_path.extend(trail.iter().map(|(_, t)| t.clone()));
            }
            None => current.push(line),
        }
    }
    flush(&mut sections, &mut current, &current_path);

    if saw_heading {
        Some(sections)
    } else {
        None
    }
}

fn flush(out: &mut Vec<Chunk>, buf: &mut Vec<&str>, path: &[String]) {
    let body = buf.join("\n");
    buf.clear();
    let body = body.trim();
    if !body.is_empty() {
        out.push(Chunk {
            heading_path: path.to_vec(),
            body: body.to_string(),
        });
    }
}

/// `## Heading` → `(2, "Heading")`. Only levels 2–6; `#` is the document title.
fn heading_of(line: &str) -> Option<(usize, String)> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with("##") {
        return None;
    }
    let level = trimmed.chars().take_while(|c| *c == '#').count();
    if !(2..=6).contains(&level) {
        return None;
    }
    let text = trimmed[level..].trim().trim_end_matches(':').trim();
    if text.is_empty() {
        return None;
    }
    Some((level, text.to_string()))
}

/// Merge a too-short section into the one that follows it.
///
/// The merged chunk keeps the **earlier** heading path: a short section is almost always
/// framing for what comes next ("## 1. CORE CLI" then two lines then "### Essential Flags"),
/// and the framing is the more useful context to train against.
fn merge_short_forward(sections: Vec<Chunk>, min_section: usize) -> Vec<Chunk> {
    let mut out: Vec<Chunk> = Vec::new();
    let mut carry: Option<Chunk> = None;

    for mut sec in sections {
        if let Some(prev) = carry.take() {
            sec.body = format!("{}\n\n{}", prev.body, sec.body);
            sec.heading_path = prev.heading_path;
        }
        if sec.body.chars().count() < min_section {
            carry = Some(sec);
        } else {
            out.push(sec);
        }
    }

    // A trailing short section has nothing to merge into — keep it rather than lose it.
    if let Some(last) = carry {
        out.push(last);
    }
    out
}

/// Split an oversized chunk at blank lines. Never mid-line.
fn split_oversized(c: Chunk, max_chunk: usize) -> Vec<Chunk> {
    if c.body.chars().count() <= max_chunk {
        return vec![c];
    }

    let mut out = Vec::new();
    let mut buf = String::new();

    for para in c.body.split("\n\n") {
        let candidate = if buf.is_empty() {
            para.len()
        } else {
            buf.len() + 2 + para.len()
        };
        if !buf.is_empty() && candidate > max_chunk {
            out.push(Chunk {
                heading_path: c.heading_path.clone(),
                body: buf.trim().to_string(),
            });
            buf = para.to_string();
        } else {
            if !buf.is_empty() {
                buf.push_str("\n\n");
            }
            buf.push_str(para);
        }
    }
    if !buf.trim().is_empty() {
        out.push(Chunk {
            heading_path: c.heading_path,
            body: buf.trim().to_string(),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsectioned_content_stays_whole() {
        let body = "A single long operational lesson that runs on for a while without any \
                    markdown headings at all, because most semantic memories look like this.";
        let chunks = chunk(body, &ChunkConfig::default());
        assert_eq!(
            chunks.len(),
            1,
            "must never paragraph-split unsectioned content"
        );
        assert_eq!(chunks[0].body, body);
    }

    /// Shaped after the real VLLM reference memory in the live store.
    #[test]
    fn sectioned_reference_yields_one_chunk_per_section_with_heading_path() {
        let doc = "VLLM SERVING CONFIGURATION REFERENCE\n\
                   \n## 1. CORE CLI\n\
                   \nBasic invocation of the server binary, with the legacy module form also \
                   still accepted for older deployments that have not migrated yet.\n\
                   \n### Essential Flags\n\
                   \n--model is the HuggingFace id or a local path. --tensor-parallel-size sets \
                   the GPU count and must divide the attention heads evenly across devices.\n";

        let chunks = chunk(doc, &ChunkConfig::default());
        assert_eq!(chunks.len(), 2);

        assert_eq!(
            chunks[0].heading_path,
            vec!["VLLM SERVING CONFIGURATION REFERENCE", "1. CORE CLI"]
        );
        assert_eq!(
            chunks[1].heading_path,
            vec![
                "VLLM SERVING CONFIGURATION REFERENCE",
                "1. CORE CLI",
                "Essential Flags"
            ]
        );
        assert!(chunks[1].body.starts_with("--model"));
    }

    #[test]
    fn deeper_heading_does_not_leak_into_the_next_sibling() {
        let doc = "Doc\n\n## A\n\n### A1\n\nbody one\n\n## B\n\nbody two\n";
        let cfg = ChunkConfig {
            min_section: 1,
            ..ChunkConfig::default()
        };
        let chunks = chunk(doc, &cfg);
        let b = chunks
            .iter()
            .find(|c| c.body == "body two")
            .expect("section B");
        assert_eq!(
            b.heading_path,
            vec!["Doc", "B"],
            "A1 must not persist into B"
        );
    }

    #[test]
    fn short_section_merges_forward_and_keeps_the_framing_path() {
        let doc = "Doc\n\n## Framing\n\ntiny\n\n### Detail\n\n\
                   A much longer body that comfortably clears the minimum section length so \
                   that it is kept as a chunk of its own rather than being merged onward.\n";
        let chunks = chunk(doc, &ChunkConfig::default());
        assert_eq!(chunks.len(), 1);
        assert!(
            chunks[0].body.starts_with("tiny"),
            "short section's text is preserved"
        );
        assert!(chunks[0].body.contains("much longer body"));
        assert_eq!(
            chunks[0].heading_path,
            vec!["Doc", "Framing"],
            "keeps the framing path"
        );
    }

    #[test]
    fn trailing_short_section_is_kept_not_dropped() {
        let doc = "Doc\n\n## Big\n\n".to_string()
            + &"long enough body text ".repeat(20)
            + "\n\n## Tail\n\ntiny\n";
        let chunks = chunk(&doc, &ChunkConfig::default());
        assert!(
            chunks.iter().any(|c| c.body.contains("tiny")),
            "a trailing short section has nothing to merge into and must survive"
        );
    }

    #[test]
    fn oversized_chunk_splits_only_at_paragraph_boundaries() {
        let para = "word ".repeat(60); // ~300 chars
        let body = std::iter::repeat_n(para.trim(), 10)
            .collect::<Vec<_>>()
            .join("\n\n");
        let cfg = ChunkConfig {
            max_chunk: 800,
            ..ChunkConfig::default()
        };
        let chunks = chunk(&body, &cfg);

        assert!(chunks.len() > 1, "should have split");
        for c in &chunks {
            assert!(!c.body.starts_with(' ') && !c.body.ends_with(' '));
            // Every piece must be whole paragraphs — no dangling partial line.
            assert!(c.body.split("\n\n").all(|p| p.trim() == para.trim()));
        }
    }

    #[test]
    fn empty_content_yields_no_chunks() {
        assert!(chunk("   \n  ", &ChunkConfig::default()).is_empty());
    }

    /// Captured from the FORGE store (2026-08-16). The first line is a labelled
    /// banner, longer than `max_title`, and ends with a period — both of the
    /// old title rules rejected it, so the procedure framed from tags.
    #[test]
    fn procedure_banner_is_a_title_even_when_long_and_sentence_shaped() {
        let title = concat!(
            "PROCEDURE — Diagnosing a hung write behind a tokio RwLock ",
            "(write-starvation), and the live-probe toolkit. Forged in ",
            "ApexOS-RS session 21 hunting the mesh add-peer hang.",
        );
        let body = format!(
            "{title}\n\n\
             SYMPTOM PATTERN: an HTTP write hangs forever while reads \
             on the same resource return immediately. That split is the \
             signature of a writer waiting on a lock the readers keep \
             refreshing."
        );
        let chunks = chunk(&body, &ChunkConfig::default());
        assert_eq!(chunks.len(), 1, "still one unsectioned lesson");
        assert_eq!(chunks[0].heading_path, vec![title]);
    }

    #[test]
    fn architecture_decision_banner_is_a_title() {
        let body = "ARCHITECTURE DECISION (2026-06-20, André) — Occipital recall \
                    is INTENTIONALLY node-global; do NOT add per-agent scoping.\n\n\
                    web_recall means search pages this NODE has fetched. That is \
                    the tool working as designed, not a leak of someone else's memory.";
        let chunks = chunk(body, &ChunkConfig::default());
        assert!(
            chunks[0].heading_path[0].starts_with("ARCHITECTURE DECISION"),
            "got {:?}",
            chunks[0].heading_path
        );
    }

    #[test]
    fn label_emdash_banner_is_a_title() {
        let body = "THE CALLOSUM FIELD LOOP — how FORGE field-proves an \
                    ApexOS-RS slice on apex1 (proven 3×).\n\n\
                    Run the smokes on the node, not in CI, and write the \
                    result back as a session note before calling the slice done.";
        let chunks = chunk(body, &ChunkConfig::default());
        assert!(
            chunks[0].heading_path[0].starts_with("THE CALLOSUM FIELD LOOP"),
            "got {:?}",
            chunks[0].heading_path
        );
    }

    /// Em-dash used as list punctuation late in a how-to sentence is not a title.
    #[test]
    fn late_emdash_in_prose_is_not_a_title() {
        let body = "Pi deploy workflow for CerebroCortex-RS binary updates: \
                    (1) cargo test — all pass. (2) git add + git commit + push. \
                    (3) pull on the Pi and restart the unit.";
        let chunks = chunk(body, &ChunkConfig::default());
        assert!(
            chunks[0].heading_path.is_empty(),
            "list punctuation must not become a title, got {:?}",
            chunks[0].heading_path
        );
    }

    #[test]
    fn ordinary_prose_opener_is_still_not_a_title() {
        let body = "CerebroCortex-RS is a pure-Rust port of the Python CerebroCortex. \
                    Goal: single binary drop-in with the same SQLite file and the \
                    same MCP tool names the agents already call.";
        let chunks = chunk(body, &ChunkConfig::default());
        assert!(chunks[0].heading_path.is_empty());
    }
}
