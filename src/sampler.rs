//! Cloud sampler backends: OpenAI-compatible chat and native Gemini
//! structured output. Builders/parsers are pure functions pinned by tests.

use serde_json::json;

/// Which transport produces candidates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Llama,
    OpenAICompat,
    GeminiNative,
}

impl Kind {
    /// Parse a backend name; unknown names fall back to local llama.
    pub fn parse(s: &str) -> Kind {
        match s.to_ascii_lowercase().as_str() {
            "openai" | "openai-compat" => Kind::OpenAICompat,
            "gemini" | "gemini-native" => Kind::GeminiNative,
            _ => Kind::Llama,
        }
    }
}

/// Token usage reported by providers (0 when absent).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Usage {
    pub prompt: u64,
    pub completion: u64,
}

impl Usage {
    pub fn zero() -> Self {
        Usage { prompt: 0, completion: 0 }
    }
    pub fn total(&self) -> u64 {
        self.prompt + self.completion
    }
}

impl std::ops::AddAssign for Usage {
    fn add_assign(&mut self, rhs: Self) {
        self.prompt += rhs.prompt;
        self.completion += rhs.completion;
    }
}

const TYPE_ENUM: [&str; 5] = ["Int", "F64", "Bool", "List<Int>", "List<F64>"];

/// Strip the llama-style trailing prefill (`fn @`) for chat backends.
pub fn chat_prompt(llama_prompt: &str) -> String {
    llama_prompt.trim_end_matches("\nfn @").to_string()
}

/// OpenAI-compatible /chat/completions request body.
pub fn openai_body(model: &str, prompt: &str, temp: f64, max_tokens: usize) -> String {
    json!({
        "model": model,
        "messages": [
            {"role": "user", "content": prompt}
        ],
        "temperature": temp,
        "max_tokens": max_tokens
    })
    .to_string()
}

/// Parse an OpenAI-compatible response into (content, usage).
pub fn openai_parse(body: &str) -> Result<(String, Usage), String> {
    let v: serde_json::Value =
        serde_json::from_str(body).map_err(|e| format!("bad JSON: {}", e))?;
    let content = v
        .pointer("/choices/0/message/content")
        .and_then(|c| c.as_str())
        .ok_or_else(|| {
            format!(
                "no choices[0].message.content in `{}`",
                &body[..body.len().min(200)]
            )
        })?
        .to_string();
    let usage = Usage {
        prompt: v.pointer("/usage/prompt_tokens").and_then(|x| x.as_u64()).unwrap_or(0),
        completion: v
            .pointer("/usage/completion_tokens")
            .and_then(|x| x.as_u64())
            .unwrap_or(0),
    };
    Ok((content, usage))
}

/// Gemini responseSchema: typed object so parameter-type typos are impossible
/// on this backend. `body` stays free text — sieve-judged as ever.
fn schema() -> serde_json::Value {
    json!({
        "type": "OBJECT",
        "properties": {
            "name": {"type": "STRING"},
            "params": {
                "type": "ARRAY",
                "items": {
                    "type": "OBJECT",
                    "properties": {
                        "n": {"type": "STRING"},
                        "t": {"type": "STRING", "enum": TYPE_ENUM}
                    },
                    "required": ["n", "t"]
                }
            },
            "ret": {"type": "STRING", "enum": TYPE_ENUM},
            "body": {"type": "STRING"}
        },
        "required": ["name", "params", "ret", "body"]
    })
}

/// Native Gemini generateContent request body.
pub fn gemini_body(prompt: &str, temp: f64, max_tokens: usize) -> String {
    json!({
        "contents": [
            {"role": "user", "parts": [{"text": prompt}]}
        ],
        "generationConfig": {
            "temperature": temp,
            "maxOutputTokens": max_tokens,
            "responseMimeType": "application/json",
            "responseSchema": schema(),
            "candidateCount": 1
        }
    })
    .to_string()
}

/// Full endpoint URL from base + model.
pub fn gemini_url(base: &str, model: &str) -> String {
    format!("{}/models/{}:generateContent", base.trim_end_matches('/'), model)
}

/// Reassemble sketch source from schema fields (pure).
pub fn reassemble(name: &str, params: &[(String, String)], ret: &str, body: &str) -> String {
    let ps: Vec<String> =
        params.iter().map(|(n, t)| format!("%{}: {}", n, t)).collect();
    format!(
        "fn @{}({}) -> {} {{ {} }}",
        name,
        ps.join(", "),
        ret,
        body.trim()
    )
}

/// Parse native Gemini response: extract schema JSON from parts[0], then
/// reassemble sketch text. Returns (sketch_text, usage).
pub fn gemini_parse(body: &str) -> Result<(String, Usage), String> {
    let v: serde_json::Value =
        serde_json::from_str(body).map_err(|e| format!("bad JSON: {}", e))?;
    let text = v
        .pointer("/candidates/0/content/parts/0/text")
        .and_then(|t| t.as_str())
        .ok_or_else(|| {
            format!(
                "no candidates[0] text: {}",
                &body[..body.len().min(200)]
            )
        })?
        .to_string();
    let usage = Usage {
        prompt: v
            .pointer("/usageMetadata/promptTokenCount")
            .and_then(|x| x.as_u64())
            .unwrap_or(0),
        completion: v
            .pointer("/usageMetadata/candidatesTokenCount")
            .and_then(|x| x.as_u64())
            .unwrap_or(0),
    };
    let obj: serde_json::Value = serde_json::from_str(text.trim())
        .map_err(|e| format!("schema payload not JSON: {}", e))?;
    // Normalization (pure text surgery): models often include the % sigil in
    // parameter names and dots in function names; both break the lexer.
    let clean_ident = |s: &str| -> String {
        s.trim()
            .trim_start_matches('%')
            .replace(['.', '-'], "_")
    };
    let name = clean_ident(obj.get("name").and_then(|x| x.as_str()).unwrap_or("f"));
    let ret = obj.get("ret").and_then(|x| x.as_str()).unwrap_or("Int");
    let body_src = obj.get("body").and_then(|x| x.as_str()).unwrap_or("");
    let mut params: Vec<(String, String)> = Vec::new();
    if let Some(arr) = obj.get("params").and_then(|p| p.as_array()) {
        for p in arr {
            let n = clean_ident(p.get("n").and_then(|x| x.as_str()).unwrap_or("a"));
            let t = p.get("t").and_then(|x| x.as_str()).unwrap_or("Int");
            params.push((n, t.to_string()));
        }
    }
    Ok((reassemble(&name, &params, ret, body_src), usage))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kind_parse_defaults_to_llama() {
        assert_eq!(Kind::parse("gemini"), Kind::GeminiNative);
        assert_eq!(Kind::parse("OpenAI"), Kind::OpenAICompat);
        assert_eq!(Kind::parse("whatever"), Kind::Llama);
    }

    #[test]
    fn test_chat_prompt_strips_prefill() {
        let p = build_prompt_like();
        assert_eq!(chat_prompt(&p), p.trim_end_matches("\nfn @"));
    }

    fn build_prompt_like() -> String {
        "spec\nrules\n\nfn @".to_string()
    }

    #[test]
    fn test_openai_body_shape_and_parse_roundtrip() {
        let b = openai_body("m1", "PROMPT", 0.4, 256);
        assert!(b.contains("\"model\":\"m1\""));
        assert!(b.contains("PROMPT"));
        let resp = r#"{"choices":[{"message":{"content":"fn @x(%a: Int) -> Int { %a }"}}],"usage":{"prompt_tokens":10,"completion_tokens":5}}"#;
        let (c, u) = openai_parse(resp).unwrap();
        assert!(c.starts_with("fn @x"));
        assert_eq!((u.prompt, u.completion), (10, 5));
    }

    #[test]
    fn test_gemini_body_carries_schema() {
        let b = gemini_body("PROMPT", 0.7, 512);
        assert!(b.contains("\"responseSchema\""));
        assert!(b.contains("List<Int>"), "type enum missing");
        assert!(b.contains("\"candidateCount\":1"));
    }

    #[test]
    fn test_gemini_parse_reassembles_sketch() {
        // Fixture built programmatically — no hand-counted brackets.
        let inner = json!({
            "name": "mean",
            "params": [{"n": "xs", "t": "List<F64>"}],
            "ret": "F64",
            "body": "sum(%xs) / len(%xs)"
        });
        let payload = json!({
            "candidates": [
                {"content": {"parts": [{"text": inner.to_string()}]}}
            ],
            "usageMetadata": {"promptTokenCount": 12, "candidatesTokenCount": 34}
        })
        .to_string();
        let (sketch, u) = gemini_parse(&payload).unwrap();
        assert_eq!(
            sketch,
            "fn @mean(%xs: List<F64>) -> F64 { sum(%xs) / len(%xs) }"
        );
        assert_eq!((u.prompt, u.completion), (12, 34));
    }

    #[test]
    fn test_reassemble_multi_param() {
        let s = reassemble(
            "f",
            &[("a".into(), "Int".into()), ("b".into(), "F64".into())],
            "Bool",
            "%a > %b",
        );
        assert_eq!(s, "fn @f(%a: Int, %b: F64) -> Bool { %a > %b }");
    }

    #[test]
    fn test_gemini_url_joining() {
        assert_eq!(
            gemini_url("https://x/v1beta/", "m"),
            "https://x/v1beta/models/m:generateContent"
        );
    }

    #[test]
    fn test_usage_adds() {
        let mut u = Usage::zero();
        u += Usage { prompt: 3, completion: 4 };
        u += Usage { prompt: 1, completion: 2 };
        assert_eq!(u.total(), 10);
    }
}
