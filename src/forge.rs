//! Forge: stochastic candidate generation against the local transformer.
//!
//! THE WALL: this module *generates* candidates only. It never sees sieve
//! verdicts beyond the machine-readable rejection strings it is told to
//! retry on, and it never judges anything. Prompts contain ONLY transparent
//! evidence (AGENTS.md Golden Rule 3).

use crate::http::HttpClient;
use crate::sketch;
use crate::wish::{Value, Wish};
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

/// Default endpoint matches VITRIOL's memory-mode shim port.
pub const DEFAULT_FORGE: &str = "127.0.0.1:8279";
const MAX_TOKENS: usize = 512;
/// Parallel TCP workers — deliberately low: the llama-server endpoint is
/// often SHARED and frequently launched with --parallel 1; slot exhaustion
/// manifests as connection resets. Override via ONTIC_FORGE_WORKERS.
fn worker_count() -> usize {
    std::env::var("ONTIC_FORGE_WORKERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|&w| w >= 1)
        .unwrap_or(2)
}
/// Per-sample transport retries (fresh connection each try) before a sample
/// is abandoned. The batch still succeeds if any other sample survives.
const SAMPLE_TRIES: usize = 3;
/// Retryable server statuses — typically transient overload on shared hosts.
fn retryable_status(status: u16) -> bool {
    matches!(status, 500 | 502 | 503 | 504)
}

#[derive(Debug, Clone)]
pub struct ForgeConfig {
    pub host: String,
    pub port: u16,
    pub samples: usize,
    pub seed: u64,
    pub temperature: f64,
}

impl Default for ForgeConfig {
    fn default() -> Self {
        let (host, port) = parse_endpoint(DEFAULT_FORGE);
        ForgeConfig {
            host,
            port,
            samples: 32,
            seed: 0x5EED,
            temperature: 0.8,
        }
    }
}

/// Parse `host:port`; falls back to the default endpoint on garbage.
pub fn parse_endpoint(s: &str) -> (String, u16) {
    match s.rsplit_once(':') {
        Some((h, p)) => match p.parse::<u16>() {
            Ok(port) => (h.to_string(), port),
            Err(_) => parse_endpoint(DEFAULT_FORGE),
        },
        None => parse_endpoint(DEFAULT_FORGE),
    }
}

/// Render a wish parameter list for prompts.
fn sig_text(wish: &Wish) -> String {
    let params: Vec<String> = wish
        .params
        .iter()
        .map(|(n, t)| format!("%{}: {}", n, t.name()))
        .collect();
    format!("fn {}({}) -> {}", wish.path, params.join(", "), wish.ret.name())
}

/// Render one value exactly as the .ont surface does.
fn val_text(v: &Value) -> String {
    v.to_string()
}

/// Build the sampling prompt. Transparent examples ONLY — this is the single
/// choke point enforcing the opacity guarantee; audit changes here hardest.
/// Prefill strategy: the prompt ends with a literal `fn @`, so the model's
/// first generated token is already inside the function name. Mellum2 is a
/// code-completion model; completion-style prompting beats chat instructions.
pub fn build_prompt(wish: &Wish, feedback: &[String]) -> String {
    let mut p = String::new();
    p.push_str("Complete one Ontic sketch implementation.\n");
    p.push_str("\n=== SPECIFICATION (notation only — never copy into code) ===\n");
    p.push_str(&format!("{}\n", sig_text(wish)));
    if !wish.invariants.is_empty() {
        p.push_str("Invariants:\n");
        for inv in &wish.invariants {
            p.push_str(&format!(
                "| {}\n",
                crate::lower::expr_display(inv)
            ));
        }
    }
    if !wish.transparent.is_empty() {
        p.push_str("Evidence:\n");
        for ex in &wish.transparent {
            let ins: Vec<String> = ex.inputs.iter().map(val_text).collect();
            p.push_str(&format!("=> {} -> {}\n", ins.join(", "), val_text(&ex.output)));
        }
    }
    if !feedback.is_empty() {
        p.push_str("\nRejected attempts and reasons (avoid these mistakes):\n");
        for f in feedback {
            p.push_str(&format!("- {}\n", f));
        }
    }
    p.push_str("\n=== LANGUAGE RULES ===\n");
    p.push_str("Output exactly one function; nothing else.\n");
    p.push_str("Example of the format on a DIFFERENT task (note: two passes via two folds bound with let):\n");
    p.push_str("fn @sq_over_sum(%ns: List<Int>) -> Int { let %total = fold %v in %ns, %a from 0 { %a + %v }; let %sq = fold %v in %ns, %b from 0 { %b + %v * %v }; %sq / %total }\n");
    p.push_str("The | , => , ?? , +- tolerance marks above are SPECIFICATION notation. Do not emit them.\n");
    p.push_str("Iterate only via fold: fold %v in <list>, %acc from <init> { <body> }.\n");
    p.push_str("The body must have exactly the signature's return type.\n");
    p.push_str("== / != compare same-typed scalars only; never lists.\n");
    p.push_str("F64 arithmetic is IEEE: division by zero yields inf/NaN, never an error.\n");
    p.push_str("len(x) is the only list operation; there is no indexing or concatenation.\n");
    p.push_str("Choose your own short lowercase name.\n");
    p.push_str("%res exists only in specifications; NEVER reference it inside the implementation.\n");
    p.push_str("\n=== IMPLEMENTATION ===\nfn @");
    p
}

/// Build the JSON body for one /completion call (pure — unit tested).
/// Grammar is ALWAYS set and strict from the first generated token; the
/// prompt prefill (`fn @`) means token 0 is already inside the name, so no
/// prose or reasoning preamble is reachable.
pub fn body_for(prompt: &str, cfg: &ForgeConfig, sample_index: usize) -> String {
    json!({
        "prompt": prompt,
        "n_predict": MAX_TOKENS,
        "temperature": cfg.temperature,
        "grammar": sketch::GRAMMAR,
        "seed": cfg.seed.wrapping_add(sample_index as u64),
        "cache_prompt": true,
    })
    .to_string()
}

/// Extract the candidate text from a /completion response.
/// The prompt prefill ends with `fn @`, so generated content starts at the
/// function NAME. We deterministically reattach the prefix and slice to the
/// last `}` (models sometimes append commentary after the function). Pure
/// text surgery — acceptance decisions remain entirely in the sieve.
pub fn extract_candidate(raw: &str) -> String {
    let text = raw.trim();
    if let Some(i) = text.find("fn @") {
        // Model echoed the full form; slice from the real start.
        let end = text.rfind('}').map(|j| j + 1).unwrap_or(text.len());
        return text[i..end.max(i)].trim().to_string();
    }
    // Prefill continuation: slice trailing commentary after the final brace.
    let end = text.rfind('}').map(|j| j + 1).unwrap_or(text.len());
    format!("fn @{}", &text[..end])
}

fn extract_content(body: &str) -> Result<String, String> {
    let v: serde_json::Value =
        serde_json::from_str(body).map_err(|e| format!("bad JSON response: {}", e))?;
    let raw = v
        .get("content")
        .and_then(|c| c.as_str())
        .ok_or_else(|| "response missing `content`".to_string())?;
    Ok(extract_candidate(raw))
}

/// Sample `cfg.samples` candidates in parallel. Returns texts ordered by
/// sample index so downstream sieving is deterministic regardless of
/// network completion order.
pub fn sample(wish: &Wish, cfg: &ForgeConfig, feedback: &[String]) -> Result<Vec<String>, String> {
    let prompt = build_prompt(wish, feedback);
    let n = cfg.samples.max(1);
    let workers = worker_count().min(n);
    let done = AtomicUsize::new(0);

    let results: Vec<Vec<(usize, String)>> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..workers)
            .map(|w| {
                let prompt_ref = &prompt;
                let cfg_ref = &cfg;
                let done_ref = &done;
                s.spawn(move || -> Result<Vec<(usize, String)>, String> {
                    let mut client = HttpClient::connect(&cfg_ref.host, cfg_ref.port)?;
                    let host_header = format!("{}:{}", cfg_ref.host, cfg_ref.port);
                    let mut out = Vec::new();
                    let mut idx = w;
                    while idx < n {
                        let body = body_for(prompt_ref, cfg_ref, idx);
                        // Shared-server etiquette: transient failures (reset
                        // connections, 5xx) get fresh-connection retries with
                        // backoff; an exhausted sample is skipped, not fatal.
                        let mut attempt = 0;
                        loop {
                            attempt += 1;
                            match client.post_json(&host_header, "/completion", &body) {
                                Ok(resp) if resp.status == 200 => {
                                    out.push((idx, extract_content(&resp.body)?));
                                    break;
                                }
                                Ok(resp) if retryable_status(resp.status) && attempt < SAMPLE_TRIES => {
                                    eprintln!(
                                        "forge: sample {} got HTTP {}, retrying ({}/{})",
                                        idx, resp.status, attempt, SAMPLE_TRIES
                                    );
                                }
                                Ok(resp) => {
                                    return Err(format!("sample {}: HTTP {}: {}", idx, resp.status, resp.body))
                                }
                                Err(e) if attempt < SAMPLE_TRIES => {
                                    eprintln!(
                                        "forge: sample {} transport error ({}), reconnecting ({}/{})",
                                        idx, e, attempt, SAMPLE_TRIES
                                    );
                                    client = HttpClient::connect(&cfg_ref.host, cfg_ref.port)?;
                                }
                                Err(e) => {
                                    eprintln!("forge: sample {} abandoned: {}", idx, e);
                                    break;
                                }
                            }
                            std::thread::sleep(Duration::from_millis(300 * attempt as u64));
                        }
                        let seen = done_ref.fetch_add(1, Ordering::Relaxed) + 1;
                        eprintln!("forge: {}/{} sampled", seen, n);
                        idx += workers;
                    }
                    Ok(out)
                })
            })
            .collect();
        let mut merged: Vec<Vec<(usize, String)>> = Vec::new();
        for h in handles {
            merged.push(
                h.join()
                    .map_err(|_| "forge worker panicked".to_string())??,
            );
        }
        Ok::<Vec<Vec<(usize, String)>>, String>(merged)
    })?;

    let mut flat: Vec<(usize, String)> = results.into_iter().flatten().collect();
    flat.sort_by_key(|(i, _)| *i);
    let texts: Vec<String> = flat.into_iter().map(|(_, t)| t).collect();
    if texts.is_empty() {
        return Err("all samples failed (server unreachable or rejecting)".to_string());
    }
    Ok(texts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wish;

    const WISH_SRC: &str = "\
fn Ledger.total(%items: List<Int>) -> Int
  | %res >= 0
  => [1,2,3] -> 6
  => [] -> 0
  ?? [4,5] -> 9
";

    #[test]
    fn test_prompt_contains_only_transparent_evidence() {
        let w = wish::parse(WISH_SRC).unwrap();
        let p = build_prompt(&w, &[]);
        assert!(p.contains("[1,2,3] -> 6"));
        assert!(p.contains("[] -> 0"));
        assert!(!p.contains("-> 9"), "opaque example leaked into prompt");
        assert!(p.contains("%res >= 0"));
        assert!(p.contains("List<Int>"));
    }

    #[test]
    fn test_feedback_round_included_in_prompt() {
        let w = wish::parse(WISH_SRC).unwrap();
        let p = build_prompt(&w, &["S5-probe/invariant-violation: res < 0".into()]);
        assert!(p.contains("Rejected attempts"));
        assert!(p.contains("invariant-violation"));
    }

    #[test]
    fn test_body_always_carries_grammar_and_seed() {
        let cfg = ForgeConfig::default();
        let b = body_for("PROMPT", &cfg, 3);
        assert!(b.contains("\"grammar\""));
        assert!(b.contains(GRAMMAR_SNIPPET));
        assert!(b.contains("\"seed\""));
    }

    const GRAMMAR_SNIPPET: &str = "root";

    #[test]
    fn test_sample_seeds_differ_per_index() {
        let cfg = ForgeConfig::default();
        let b0 = body_for("p", &cfg, 0);
        let b1 = body_for("p", &cfg, 1);
        assert_ne!(b0, b1);
    }

    #[test]
    fn test_endpoint_parsing() {
        assert_eq!(parse_endpoint("10.0.0.2:9000"), ("10.0.0.2".into(), 9000));
        assert_eq!(parse_endpoint("junk"), ("127.0.0.1".into(), 8279));
    }

    #[test]
    fn test_candidate_extraction_reattaches_prefill() {
        let raw = "total(%items: List<Int>) -> Int { fold %x in %items, %acc from 0 { %acc + %x } }\nExplanation follows.";
        let got = extract_candidate(raw);
        assert!(got.starts_with("fn @total"));
        assert!(got.ends_with("}"));
        assert!(!got.contains("Explanation"));
    }

    #[test]
    fn test_extraction_handles_echoed_form() {
        let raw = "junk\nfn @f() -> Int { 1 }\ntail";
        assert_eq!(extract_candidate(raw), "fn @f() -> Int { 1 }");
    }

    #[test]
    fn test_prompt_ends_with_prefill_and_hides_opaque() {
        let w = wish::parse(WISH_SRC).unwrap();
        let p = build_prompt(&w, &[]);
        assert!(p.ends_with("\nfn @"));
        assert!(!p.contains("-> 9"), "opaque example leaked into prompt");
    }
}
