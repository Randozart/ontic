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

/// Default endpoint matches VITRIOL's memory-mode shim port.
pub const DEFAULT_FORGE: &str = "127.0.0.1:8279";
const MAX_TOKENS: usize = 512;
/// Parallel TCP workers — each keeps its own keep-alive connection.
const WORKERS: usize = 8;

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
pub fn build_prompt(wish: &Wish, feedback: &[String]) -> String {
    let mut p = String::new();
    p.push_str("Write one Ontic sketch implementation.\n");
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
        p.push_str("Rejected attempts and reasons (avoid these mistakes):\n");
        for f in feedback {
            p.push_str(&format!("- {}\n", f));
        }
    }
    p.push_str("Reply with only the sketch function.");
    p
}

/// Build the JSON body for one /completion call (pure — unit tested).
/// Grammar is ALWAYS set; unconstrained sampling is forbidden (AGENTS.md).
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

/// Extract the generated text from a llama-server /completion response.
fn extract_content(body: &str) -> Result<String, String> {
    let v: serde_json::Value =
        serde_json::from_str(body).map_err(|e| format!("bad JSON response: {}", e))?;
    v.get("content")
        .and_then(|c| c.as_str())
        .map(|s| s.trim().to_string())
        .ok_or_else(|| "response missing `content`".to_string())
}

/// Sample `cfg.samples` candidates in parallel. Returns texts ordered by
/// sample index so downstream sieving is deterministic regardless of
/// network completion order.
pub fn sample(wish: &Wish, cfg: &ForgeConfig, feedback: &[String]) -> Result<Vec<String>, String> {
    let prompt = build_prompt(wish, feedback);
    let n = cfg.samples.max(1);
    let workers = WORKERS.min(n);

    let results: Vec<Vec<(usize, String)>> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..workers)
            .map(|w| {
                let prompt_ref = &prompt;
                let cfg_ref = &cfg;
                s.spawn(move || -> Result<Vec<(usize, String)>, String> {
                    let mut client = HttpClient::connect(&cfg_ref.host, cfg_ref.port)?;
                    let host_header = format!("{}:{}", cfg_ref.host, cfg_ref.port);
                    let mut out = Vec::new();
                    let mut idx = w;
                    while idx < n {
                        let body = body_for(prompt_ref, cfg_ref, idx);
                        let resp = client.post_json(&host_header, "/completion", &body)?;
                        if resp.status != 200 {
                            return Err(format!("HTTP {}: {}", resp.status, resp.body));
                        }
                        out.push((idx, extract_content(&resp.body)?));
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
    Ok(flat.into_iter().map(|(_, t)| t).collect())
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
}
