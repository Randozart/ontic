//! Vault: content-addressed store of verified implementations.
//! Key = SHA-256 of the gen's canonical text (transparent evidence only —
//! opaque sets are sieve-internal and never enter keys). Entries are plain
//! files: `<key>.mlir` + `<key>.json` manifest.

use crate::sha256::sha256_hex;
use crate::gen::Gen;
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};

/// A stored verified function.
#[derive(Debug, Clone)]
pub struct Entry {
    pub key: String,
    pub name: String,
    pub signature: String,
    /// Declared tier at solve time (pre-manifest entries default checked).
    pub wrapping: bool,
    pub sketch_text: String,
    pub mlir: String,
}

/// Filesystem-backed vault rooted at a directory (default `.ontic/vault`).
pub struct Vault {
    dir: PathBuf,
}

impl Vault {
    /// Open (creating if needed) a vault directory.
    pub fn open<P: AsRef<Path>>(dir: P) -> Result<Self, String> {
        let dir = dir.as_ref().to_path_buf();
        fs::create_dir_all(&dir).map_err(|e| format!("vault create failed: {}", e))?;
        Ok(Vault { dir })
    }

    /// Content address of a gen — canonical text is the identity payload.
    pub fn key_for(gen: &Gen) -> String {
        sha256_hex(gen.canonical().as_bytes())
    }

    fn mlir_path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{}.mlir", key))
    }

    fn manifest_path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{}.json", key))
    }

    /// Store a survivor. Overwrites on re-verification of the same key.
    /// `extra_meta` merges into the manifest (e.g. solve provenance).
    pub fn put_meta(
        &self,
        gen: &Gen,
        sketch_text: &str,
        mlir: &str,
        extra_meta: &serde_json::Value,
    ) -> Result<String, String> {
        let key = Self::key_for(gen);
        fs::write(self.mlir_path(&key), mlir)
            .map_err(|e| format!("mlir write failed: {}", e))?;
        let params: Vec<String> = gen
            .params
            .iter()
            .map(|(n, t)| format!("%{}: {}", n, t.name()))
            .collect();
        let canonical = gen.canonical();
        let manifest = json!({
            "name": gen.name,
            "path": gen.path,
            "signature": format!("fn {}({}) -> {}", gen.path, params.join(", "), gen.ret.name()),
            "wrapping": gen.wrapping,
            "canonical": canonical,
            "sketch": sketch_text,
            "ns_per_call_note": "see solve output; timing is machine-specific",
        });
        let merged = Self::merge_json(&manifest, extra_meta);
        fs::write(
            self.manifest_path(&key),
            serde_json::to_string_pretty(&merged).map_err(|e| e.to_string())?,
        )
        .map_err(|e| format!("manifest write failed: {}", e))?;
        Ok(key)
    }

    /// Store without provenance metadata.
    pub fn put(&self, gen: &Gen, sketch_text: &str, mlir: &str) -> Result<String, String> {
        self.put_meta(gen, sketch_text, mlir, &serde_json::Value::Null)
    }

    /// Find a solved entry by gen path (latest match wins).
    /// Dependencies are resolved by path because their full canonical text
    /// lives only in the manifest — stored at solve time.
    pub fn find_by_path(&self, path: &str) -> Option<Entry> {
        let mut best: Option<(String, Entry)> = None;
        let entries = self.list().ok()?;
        for e in entries {
            // Signature starts with "fn <path>(" — match exactly.
            if signature_path(&e.signature) == path {
                best = Some((e.key.clone(), e));
            }
        }
        best.map(|(_, e)| e)
    }

    /// Shallow-merge b over a (b wins).
    fn merge_json(a: &serde_json::Value, b: &serde_json::Value) -> serde_json::Value {
        match (a, b) {
            (serde_json::Value::Object(_), serde_json::Value::Null) => a.clone(),
            (serde_json::Value::Object(am), serde_json::Value::Object(bm)) => {
                let mut out = am.clone();
                for (k, v) in bm {
                    out.insert(k.clone(), v.clone());
                }
                serde_json::Value::Object(out)
            }
            _ => a.clone(),
        }
    }

    /// Fetch an entry by key.
    pub fn get(&self, key: &str) -> Option<Entry> {
        let mlir = fs::read_to_string(self.mlir_path(key)).ok()?;
        let man_raw = fs::read_to_string(self.manifest_path(key)).ok()?;
        let man: serde_json::Value = serde_json::from_str(&man_raw).ok()?;
        Some(Entry {
            key: key.to_string(),
            name: man.get("name")?.as_str()?.to_string(),
            signature: man.get("signature")?.as_str()?.to_string(),
            wrapping: man.get("wrapping").and_then(|b| b.as_bool()).unwrap_or(false),
            sketch_text: man.get("sketch")?.as_str()?.to_string(),
            mlir,
        })
    }

    /// List all entries, deterministically ordered by key.
    pub fn list(&self) -> Result<Vec<Entry>, String> {
        let mut keys: Vec<String> = fs::read_dir(&self.dir)
            .map_err(|e| format!("vault read failed: {}", e))?
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                name.strip_suffix(".json").map(|k| k.to_string())
            })
            .collect();
        keys.sort();
        Ok(keys.iter().filter_map(|k| self.get(k)).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gen;

    const GEN_SRC: &str = "fn f(%a: Int) -> Int\n  => 1 -> 2\n";

    #[test]
    fn test_key_is_sha256_of_canonical_and_stable() {
        let w = gen::parse(GEN_SRC).unwrap();
        let k1 = Vault::key_for(&w);
        let w2 = gen::parse(&w.canonical()).unwrap();
        assert_eq!(k1, Vault::key_for(&w2));
        assert_eq!(k1.len(), 64);
    }

    #[test]
    fn test_put_get_roundtrip() {
        let tmp = std::env::temp_dir().join(format!("ontic-vault-test-{}", std::process::id()));
        let v = Vault::open(&tmp).expect("opens");
        let w = gen::parse(GEN_SRC).unwrap();
        let key = v.put(&w, "fn @f(%a: Int) -> Int { %a * 2 }", "module { }").unwrap();
        let e = v.get(&key).expect("entry exists");
        assert_eq!(e.name, "f");
        assert_eq!(e.signature, "fn f(%a: Int) -> Int");
        assert!(e.mlir.contains("module"));
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn test_list_sorted_by_key() {
        let tmp = std::env::temp_dir().join(format!("ontic-vault-list-{}", std::process::id()));
        let v = Vault::open(&tmp).expect("opens");
        let w1 = gen::parse("fn f(%a: Int) -> Int\n  => 1 -> 2\n").unwrap();
        let w2 = gen::parse("fn f(%a: Int) -> Int\n  => 1 -> 3\n").unwrap();
        v.put(&w1, "s1", "m1").unwrap();
        v.put(&w2, "s2", "m2").unwrap();
        let all = v.list().unwrap();
        assert_eq!(all.len(), 2);
        assert!(all[0].key <= all[1].key);
        std::fs::remove_dir_all(&tmp).ok();
    }
}

/// Extract the gen path from a stored signature line (`fn Path.name(...)`).
fn signature_path(signature: &str) -> String {
    let inner = signature.strip_prefix("fn ").unwrap_or(signature);
    match inner.find('(') {
        Some(i) => inner[..i].trim().to_string(),
        None => inner.trim().to_string(),
    }
}

/// Append-only reuse ledger: `(dep_key, used_by_key)` -> hit count.
/// Lives BESIDE vault entries (never inside them — entries are immutable
/// artifacts, Golden Rule 15). Content is deterministic given the same
/// sequence of operations; cloud-sampled runs may order differently.
pub fn record_reuse(vault_dir: &str, dep_key: &str, used_by_key: &str) {
    let path = std::path::Path::new(vault_dir).join("reuse.json");
    let mut map: std::collections::HashMap<String, u64> = match std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
    {
        Some(m) => m,
        None => Default::default(),
    };
    *map.entry(format!("{}->{}", dep_key, used_by_key)).or_insert(0) += 1;
    if let Ok(body) = serde_json::to_string_pretty(&map) {
        let _ = std::fs::write(&path, body);
    }
}

/// Read reuse counts grouped by dependency key: dep_key -> total hits as a
/// dependency of anything.
pub fn reuse_counts(vault_dir: &str) -> std::collections::HashMap<String, u64> {
    let path = std::path::Path::new(vault_dir).join("reuse.json");
    let mut out: std::collections::HashMap<String, u64> = Default::default();
    if let Some(map) = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str::<std::collections::HashMap<String, u64>>(&s).ok())
    {
        for (pair, n) in map {
            if let Some(dep) = pair.split("->").next() {
                *out.entry(dep.to_string()).or_insert(0) += n;
            }
        }
    }
    out
}
