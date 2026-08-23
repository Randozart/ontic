//! Minimal `.env` reader: KEY=VALUE lines, `#` comments ignored, values may
//! be quoted. NEVER overrides variables already present in the real
//! environment — .env is the lowest-precedence source.

use std::collections::HashMap;
use std::path::Path;

/// Parse .env text into a map (pure — unit tested).
pub fn parse(text: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (k, v) = match line.split_once('=') {
            Some(kv) => kv,
            None => continue,
        };
        let key = k.trim().to_string();
        if key.is_empty() {
            continue;
        }
        let mut val = v.trim().to_string();
        // Strip matching quotes.
        if (val.starts_with('"') && val.ends_with('"') && val.len() >= 2)
            || (val.starts_with('\'') && val.ends_with('\'') && val.len() >= 2)
        {
            val = val[1..val.len() - 1].to_string();
        }
        out.insert(key, val);
    }
    out
}

/// Load `.env` from the given directory into the process environment.
/// Returns the number of variables actually injected (existing env vars are
/// never overridden). Missing file is not an error.
pub fn load(dir: &Path) -> Result<usize, String> {
    let path = dir.join(".env");
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => return Ok(0),
    };
    let mut injected = 0usize;
    for (k, v) in parse(&text) {
        if std::env::var_os(&k).is_none() {
            std::env::set_var(&k, &v);
            injected += 1;
        }
    }
    Ok(injected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_comments_quotes_and_blanks() {
        let m = parse("# comment\nA=1\nB = \"two words\"\nC='three'\n\nbroken line\n=empty\nD=\n");
        assert_eq!(m.get("A").map(String::as_str), Some("1"));
        assert_eq!(m.get("B").map(String::as_str), Some("two words"));
        assert_eq!(m.get("C").map(String::as_str), Some("three"));
        assert!(m.get("D").map(String::as_str) == Some("") || !m.contains_key("D"));
        assert!(!m.contains_key("broken"));
    }

    #[test]
    fn test_load_never_overrides_existing_env() {
        let dir = std::env::temp_dir().join(format!(
            "ontic-envtest-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(".env"), "ONTIC_DOTENV_PROBE=fromfile\n").unwrap();
        std::env::set_var("ONTIC_DOTENV_PROBE", "fromenv");
        let n = load(&dir).expect("loads");
        assert_eq!(n, 0, "existing env must win");
        assert_eq!(
            std::env::var("ONTIC_DOTENV_PROBE").unwrap(),
            "fromenv"
        );
        std::env::remove_var("ONTIC_DOTENV_PROBE");
        std::fs::remove_dir_all(&dir).ok();
    }
}
