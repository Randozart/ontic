//! Forge: stochastic candidate generation against the local transformer.
//!
//! THE WALL: this module *generates* candidates only. It never sees sieve
//! verdicts beyond the machine-readable rejection strings it is told to
//! retry on, and it never judges anything. Prompts contain ONLY transparent
//! evidence (AGENTS.md Golden Rule 3).

use crate::http::HttpClient;
use crate::sketch;
use crate::gen::{Value, Gen};
use serde_json::json;
use crate::sampler::{self, Usage};
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
/// Cloud generations get more room than llama prefill: composed bodies run long.
const MAX_TOKENS_CLOUD: usize = 1536;
/// Retryable server statuses — typically transient overload on shared hosts.
fn retryable_status(status: u16) -> bool {
    matches!(status, 500 | 502 | 503 | 504)
}

/// Which sampler transport produces candidates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Llama,
    OpenAICompat,
    GeminiNative,
    /// Type-directed random enumeration — the ablation baseline.
    Uniform,
}

impl Backend {
    pub fn label(&self) -> &'static str {
        match self {
            Backend::Llama => "llama",
            Backend::OpenAICompat => "openai",
            Backend::GeminiNative => "gemini",
            Backend::Uniform => "uniform",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ForgeConfig {
    /// Local llama endpoint host/port.
    pub host: String,
    pub port: u16,
    /// Cloud backend selection + credentials indirection.
    pub backend: Backend,
    pub endpoint: String,
    pub model: String,
    pub api_key_env: String,
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
            backend: Backend::Llama,
            endpoint: "https://generativelanguage.googleapis.com/v1beta".to_string(),
            model: "gemini-3.5-flash-lite".to_string(),
            api_key_env: "GEMINI_API_KEY".to_string(),
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

/// Render a gen parameter list for prompts.
fn sig_text(gen: &Gen) -> String {
    let params: Vec<String> = gen
        .params
        .iter()
        .map(|(n, t)| format!("%{}: {}", n, t.name()))
        .collect();
    format!("fn {}({}) -> {}", gen.path, params.join(", "), gen.ret.name())
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
pub fn build_prompt(gen: &Gen, feedback: &[String]) -> String {
    let mut p = String::new();
    p.push_str("Complete one Ontic sketch implementation.\n");
    p.push_str("\n=== SPECIFICATION (notation only — never copy into code) ===\n");
    p.push_str(&format!("{}\n", sig_text(gen)));
    if !gen.invariants.is_empty() {
        p.push_str("Invariants:\n");
        for inv in &gen.invariants {
            p.push_str(&format!(
                "| {}\n",
                crate::lower::expr_display(inv)
            ));
        }
    }
    if !gen.transparent.is_empty() {
        p.push_str("Evidence:\n");
        for ex in &gen.transparent {
            let ins: Vec<String> = ex.inputs.iter().map(val_text).collect();
            p.push_str(&format!("=> {} -> {}\n", ins.join(", "), val_text(&ex.output)));
        }
    }
    if !gen.hints.is_empty() {
        p.push_str("\n=== AUTHOR GUIDANCE ===\n");
        for h in &gen.hints {
            p.push_str(&format!("- {}\n", h));
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
pub fn sample(
    gen: &Gen,
    cfg: &ForgeConfig,
    feedback: &[String],
) -> Result<(Vec<String>, Usage), String> {
    // Uniform = local type-directed enumeration (ablation baseline).
    if cfg.backend == Backend::Uniform {
        let gens = crate::genrand::generate(gen, cfg.samples.max(1), cfg.seed);
        return Ok((gens.into_iter().map(|g| g.text).collect(), Usage::zero()));
    }
    // Cloud backends never touch the llama worker pool.
    if matches!(
        cfg.backend,
        Backend::OpenAICompat | Backend::GeminiNative
    ) {
        return sample_cloud(gen, cfg, feedback);
    }
    let prompt = build_prompt(gen, feedback);
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

    // Cloud backends take a separate sequential path (curl per sample).
    if cfg.backend != Backend::Llama {
        return sample_cloud(gen, cfg, feedback);
    }

    let mut flat: Vec<(usize, String)> = results.into_iter().flatten().collect();
    flat.sort_by_key(|(i, _)| *i);
    let texts: Vec<String> = flat.into_iter().map(|(_, t)| t).collect();
    if texts.is_empty() {
        return Err("all samples failed (server unreachable or rejecting)".to_string());
    }
    Ok((texts, Usage::zero()))
}

/// Cloud sampling path: one curl request per candidate index, retry/backoff
/// on transient failures, tokens accumulated across the batch.
fn sample_cloud(
    gen: &Gen,
    cfg: &ForgeConfig,
    feedback: &[String],
) -> Result<(Vec<String>, Usage), String> {
    let key = std::env::var(&cfg.api_key_env)
        .map_err(|_| format!("missing API key: set ${}", cfg.api_key_env))?;
    let style = match cfg.backend {
        Backend::OpenAICompat => crate::cloud::AuthStyle::Bearer,
        _ => crate::cloud::AuthStyle::XGoogApiKey,
    };
    let url = match cfg.backend {
        Backend::GeminiNative => sampler::gemini_url(&cfg.endpoint, &cfg.model),
        _ => format!("{}/chat/completions", cfg.endpoint.trim_end_matches('/')),
    };
    let llama_style_prompt = build_prompt(gen, feedback);
    let chat_prompt_text = sampler::chat_prompt(&llama_style_prompt);

    let mut texts: Vec<(usize, String)> = Vec::new();
    let mut usage_total = Usage::zero();
    for idx in 0..cfg.samples.max(1) {
        let body = match cfg.backend {
            Backend::OpenAICompat => {
                sampler::openai_body(&cfg.model, &chat_prompt_text, cfg.temperature, MAX_TOKENS_CLOUD)
            }
            _ => sampler::gemini_body(&chat_prompt_text, cfg.temperature, MAX_TOKENS_CLOUD),
        };
        let mut attempt = 0usize;
        loop {
            attempt += 1;
            match crate::cloud::post_json(
                &url,
                Some((&key, style)),
                &[],
                &body,
                120,
            ) {
                Ok(resp) if resp.status == 200 => {
                    let (raw, u) = match cfg.backend {
                        Backend::GeminiNative => sampler::gemini_parse(&resp.body)?,
                        _ => sampler::openai_parse(&resp.body)?,
                    };
                    usage_total += u;
                    let cand_text = forge_extract_helper(&raw);
                    if std::env::var("ONTIC_DEBUG").is_ok() {
                        eprintln!("DEBUG cand {}: {}", idx, cand_text);
                    }
                    texts.push((idx, cand_text));
                    break;
                }
                Ok(resp)
                    if matches!(resp.status, 429 | 500 | 502 | 503 | 504)
                        && attempt < SAMPLE_TRIES =>
                {
                    eprintln!(
                        "forge: sample {} got HTTP {}, retrying ({}/{})",
                        idx, resp.status, attempt, SAMPLE_TRIES
                    );
                }
                Ok(resp) => {
                    return Err(format!(
                        "sample {}: HTTP {}: {}",
                        idx,
                        resp.status,
                        resp.body
                    ))
                }
                Err(e) if attempt < SAMPLE_TRIES => {
                    eprintln!("forge: sample {} transport error ({}), retrying ({}/{})", idx, e, attempt, SAMPLE_TRIES);
                }
                Err(e) => {
                    eprintln!("forge: sample {} abandoned: {}", idx, e);
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(400 * attempt as u64));
        }
    }
    texts.sort_by_key(|(i, _)| *i);
    let out: Vec<String> = texts.into_iter().map(|(_, t)| t).collect();
    if out.is_empty() {
        return Err("all cloud samples failed".to_string());
    }
    Ok((out, usage_total))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gen;

    const GEN_SRC: &str = "\
fn Ledger.total(%items: List<Int>) -> Int
  | %res >= 0
  => [1,2,3] -> 6
  => [] -> 0
  ?? [4,5] -> 9
";

    #[test]
    fn test_prompt_contains_only_transparent_evidence() {
        let w = gen::parse(GEN_SRC).unwrap();
        let p = build_prompt(&w, &[]);
        assert!(p.contains("[1,2,3] -> 6"));
        assert!(p.contains("[] -> 0"));
        assert!(!p.contains("-> 9"), "opaque example leaked into prompt");
        assert!(p.contains("%res >= 0"));
        assert!(p.contains("List<Int>"));
    }

    #[test]
    fn test_feedback_round_included_in_prompt() {
        let w = gen::parse(GEN_SRC).unwrap();
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
        let w = gen::parse(GEN_SRC).unwrap();
        let p = build_prompt(&w, &[]);
        assert!(p.ends_with("\nfn @"));
        assert!(!p.contains("-> 9"), "opaque example leaked into prompt");
    }
}

/// Normalize raw provider output into a candidate text (thin alias kept so
/// the cloud path reads symmetrically with the llama path).
fn forge_extract_helper(raw: &str) -> String {
    extract_candidate(raw)
}
