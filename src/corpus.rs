//! Verified-corpus collection: every solve and spec-synthesis run may append
//! a training record to `.ontic/corpus/train.jsonl`. Only sieve-approved
//! code becomes supervision — cleanliness by construction. Records carry
//! `gen_key` so fine-tuned samplers can be excluded from gens they trained
//! on (contamination would burn S4 overfit detection).

use crate::sieve;
use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u8 = 1;

/// One killed candidate with its machine-generated critique.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RejectRec {
    pub text: String,
    pub stage: String,
    pub kind: String,
    pub reason: String,
}

/// What produced the record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// forge candidates against one gen (SFT: prompt→winner; DPO: rejects).
    Solve,
    /// paper prompt → spec tree (decompose).
    Spec,
}

/// A single training record.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Record {
    pub schema: u8,
    pub kind: Kind,
    /// Canonical SHA-256 of the gen (solve) or sha of the paper prompt
    /// truncated to 16 hex chars (spec) — identity for exclusion lists.
    pub gen_key: String,
    pub backend: String,
    pub model: String,
    /// Full forge/decompose prompt as sent.
    pub prompt: String,
    /// Winning candidate text (solve) or concatenated file blocks (spec).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub winner: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub rejects: Vec<RejectRec>,
    /// True when the prompt is a reconstruction (backfill), not history.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reconstructed: bool,
}

impl Record {
    pub fn new(kind: Kind, gen_key: String, backend: String, model: String, prompt: String) -> Self {
        Record {
            schema: SCHEMA_VERSION,
            kind,
            gen_key,
            backend,
            model,
            prompt,
            winner: None,
            rejects: Vec::new(),
            reconstructed: false,
        }
    }

    pub fn with_winner(mut self, text: &str) -> Self {
        self.winner = Some(text.to_string());
        self
    }

    pub fn with_rejects(mut self, rej: &[(String, sieve::Rejection)]) -> Self {
        self.rejects = rej
            .iter()
            .map(|(text, r)| RejectRec {
                text: text.clone(),
                stage: r.stage.label().to_string(),
                kind: r.kind.label().to_string(),
                reason: r.reason.clone(),
            })
            .collect();
        self
    }

    pub fn reconstructed(mut self) -> Self {
        self.reconstructed = true;
        self
    }
}

/// Collection is opt-in via env (`.env` counts through the dotenv loader):
/// `ONTIC_COLLECT=1`.
pub fn enabled() -> bool {
    std::env::var("ONTIC_COLLECT")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn corpus_path() -> std::path::PathBuf {
    let dir = std::env::var("ONTIC_VAULT").unwrap_or_else(|_| ".ontic/vault".to_string());
    std::path::Path::new(&dir)
        .parent()
        .unwrap_or(std::path::Path::new(".ontic"))
        .join("corpus")
        .join("train.jsonl")
}

/// Append one record as a JSONL line. Never fatal: collection failures are
/// printed but must not kill a solve whose real work already succeeded.
pub fn append(rec: &Record) {
    let path = corpus_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    use std::io::Write;
    let ok = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut f| {
            writeln!(f, "{}", serde_json::to_string(rec).unwrap_or_default())
        });
    if let Err(e) = ok {
        eprintln!("corpus: append failed (non-fatal): {}", e);
    }
}

/// Capture a full solve outcome (no-op unless enabled()).
pub fn capture_solve(
    gen_key: &str,
    backend_label: &str,
    model: &str,
    prompt: &str,
    report: &sieve::SieveReport,
) {
    if !enabled() || report.survivors.is_empty() && report.rejections.is_empty() {
        return;
    }
    // Rejections carry their candidate texts from the sieve now.
    let mut rec = Record::new(
        Kind::Solve,
        gen_key.to_string(),
        backend_label.to_string(),
        model.to_string(),
        prompt.to_string(),
    );
    // Winner = best survivor (already ranked first by sieve::rank).
    if let Some(s) = report.survivors.first() {
        rec = rec.with_winner(&s.source_text);
    }
    rec = rec.with_rejects(&report.rejections);
    append(&rec);
}

/// Count records grouped by (kind, backend). Backfill-inclusive.
pub fn stats() -> Result<std::collections::HashMap<String, usize>, String> {
    let path = corpus_path();
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| format!("no corpus at {}: {}", path.display(), e))?;
    let mut out: std::collections::HashMap<String, usize> = Default::default();
    for line in raw.lines() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Record>(line) {
            Ok(r) => {
                *out.entry(format!("{:?}/{}", r.kind, r.backend)).or_insert(0) += 1;
            }
            Err(_) => *out.entry("malformed".to_string()).or_insert(0) += 1,
        }
    }
    Ok(out)
}
