//! Vault: content-addressed store of verified implementations.
//! Key = SHA-256 of the wish's canonical text (transparent evidence only —
//! opaque sets are sieve-internal and never enter keys). Entries are plain
//! files: `<key>.mlir` + `<key>.json` manifest.

use crate::sha256::sha256_hex;
use crate::wish::Wish;
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};

/// A stored verified function.
#[derive(Debug, Clone)]
pub struct Entry {
    pub key: String,
    pub name: String,
    pub signature: String,
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

    /// Content address of a wish — canonical text is the identity payload.
    pub fn key_for(wish: &Wish) -> String {
        sha256_hex(wish.canonical().as_bytes())
    }

    fn mlir_path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{}.mlir", key))
    }

    fn manifest_path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{}.json", key))
    }

    /// Store a survivor. Overwrites on re-verification of the same key.
    pub fn put(&self, wish: &Wish, sketch_text: &str, mlir: &str) -> Result<String, String> {
        let key = Self::key_for(wish);
        fs::write(self.mlir_path(&key), mlir)
            .map_err(|e| format!("mlir write failed: {}", e))?;
        let params: Vec<String> = wish
            .params
            .iter()
            .map(|(n, t)| format!("%{}: {}", n, t.name()))
            .collect();
        let manifest = json!({
            "name": wish.name,
            "path": wish.path,
            "signature": format!("fn {}({}) -> {}", wish.path, params.join(", "), wish.ret.name()),
            "sketch": sketch_text,
            "ns_per_call_note": "see solve output; timing is machine-specific",
        });
        fs::write(
            self.manifest_path(&key),
            serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?,
        )
        .map_err(|e| format!("manifest write failed: {}", e))?;
        Ok(key)
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
    use crate::wish;

    const WISH_SRC: &str = "fn f(%a: Int) -> Int\n  => 1 -> 2\n";

    #[test]
    fn test_key_is_sha256_of_canonical_and_stable() {
        let w = wish::parse(WISH_SRC).unwrap();
        let k1 = Vault::key_for(&w);
        let w2 = wish::parse(&w.canonical()).unwrap();
        assert_eq!(k1, Vault::key_for(&w2));
        assert_eq!(k1.len(), 64);
    }

    #[test]
    fn test_put_get_roundtrip() {
        let tmp = std::env::temp_dir().join(format!("ontic-vault-test-{}", std::process::id()));
        let v = Vault::open(&tmp).expect("opens");
        let w = wish::parse(WISH_SRC).unwrap();
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
        let w1 = wish::parse("fn f(%a: Int) -> Int\n  => 1 -> 2\n").unwrap();
        let w2 = wish::parse("fn f(%a: Int) -> Int\n  => 1 -> 3\n").unwrap();
        v.put(&w1, "s1", "m1").unwrap();
        v.put(&w2, "s2", "m2").unwrap();
        let all = v.list().unwrap();
        assert_eq!(all.len(), 2);
        assert!(all[0].key <= all[1].key);
        std::fs::remove_dir_all(&tmp).ok();
    }
}
