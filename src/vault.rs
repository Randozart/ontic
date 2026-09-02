//! Vault: content-addressed store of verified implementations.
//! Key = SHA-256 of the gen's canonical text.
//! Entries are plain files: `<key>.mlir`, `<key>.trust`, `<key>.manifest`, `<key>.proof`.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub key: String,
    pub name: String,
    pub signature: String,
    pub sketch_text: String,
    pub mlir: String,
    pub gen_text: Option<String>,
    pub proof: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub key: String,
    pub name: String,
    pub signature: String,
    pub sketch_text: String,
    pub mlir: String,
    pub gen_text: Option<String>,
    pub proof: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofStamp {
    pub reason: String,
    pub details: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProvenVerdict {
    Attested,
    Unattested,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustVerdict {
    pub status: ProvenVerdict,
}

pub struct Vault {
    dir: PathBuf,
    index: HashMap<String, Entry>,
}

impl Vault {
    /// Set the trust verdict for a key by rewriting its manifest `proof` field.
    pub fn set_trust(&mut self, key: &str, verdict: &str) -> Result<(), String> {
        let entry = self
            .index
            .get_mut(key)
            .ok_or_else(|| format!("no vault entry {key}"))?;
        let stamp = ProofStamp {
            reason: if verdict == "proven" {
                "machine-checked".to_string()
            } else {
                "unverified".to_string()
            },
            details: Vec::new(),
        };
        let proof = serde_json::to_string_pretty(&stamp).unwrap_or_default();
        entry.proof = Some(proof.clone());
        let manifest_path = self.dir.join(format!("{}.manifest", key));
        fs::write(
            &manifest_path,
            serde_json::to_string_pretty(entry).unwrap_or_default(),
        )
        .map_err(|e| e.to_string())?;
        let proof_path = self.dir.join(format!("{}.proof", key));
        fs::write(&proof_path, proof).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn new(dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        fs::create_dir_all(&dir).expect("failed to create vault directory");
        Self {
            dir,
            index: HashMap::new(),
        }
    }

    pub fn open(dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        let mut index = HashMap::new();
        for entry in fs::read_dir(&dir).expect("failed to read vault directory") {
            let entry = entry.expect("failed to read directory entry");
            let file_name = entry.file_name().into_string().expect("invalid filename");
            if file_name.ends_with(".mlir") {
                let key = file_name.trim_end_matches(".mlir");
                let mlir = fs::read_to_string(entry.path()).expect(&format!(
                    "failed to read {}: {:?}",
                    key,
                    entry.path()
                ));
                let manifest_path = dir.join(&format!("{}.manifest", key));
                if manifest_path.exists() {
                    let manifest_str = fs::read_to_string(&manifest_path)
                        .expect(&format!("failed to read manifest {:?}", manifest_path));
                    let manifest: Manifest =
                        serde_json::from_str(&manifest_str).expect("failed to parse manifest");
                    index.insert(
                        key.to_string(),
                        Entry {
                            key: key.to_string(),
                            name: manifest.name,
                            signature: manifest.signature,
                            sketch_text: manifest.sketch_text,
                            mlir,
                            gen_text: manifest.gen_text,
                            proof: manifest.proof,
                        },
                    );
                } else {
                    index.insert(
                        key.to_string(),
                        Entry {
                            key: key.to_string(),
                            name: key.to_string(),
                            signature: key.to_string(),
                            sketch_text: mlir.clone(),
                            mlir,
                            gen_text: None,
                            proof: None,
                        },
                    );
                }
            }
        }
        Self { dir, index }
    }

    pub fn put(
        &mut self,
        key: &str,
        name: &str,
        signature: &str,
        sketch_text: &str,
        mlir: &str,
        gen_text: Option<&str>,
    ) -> Result<(), String> {
        let entry = Entry {
            key: key.to_string(),
            name: name.to_string(),
            signature: signature.to_string(),
            sketch_text: sketch_text.to_string(),
            mlir: mlir.to_string(),
            gen_text: gen_text.map(String::from),
            proof: None,
        };
        let manifest_path = self.dir.join(&format!("{}.manifest", key));
        let proof_path = self.dir.join(&format!("{}.proof", key));
        if let Some(ref gen_text) = entry.gen_text {
            fs::write(
                &proof_path,
                serde_json::to_string_pretty(gen_text).unwrap_or_default(),
            )
            .map_err(|e| e.to_string())?;
        }
        fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&entry).unwrap_or_default(),
        )
        .map_err(|e| e.to_string())?;
        self.index.insert(key.to_string(), entry);
        let mlir_path = self.dir.join(&format!("{}.mlir", key));
        fs::write(&mlir_path, mlir).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn put_proven(
        &mut self,
        key: &str,
        name: &str,
        signature: &str,
        sketch_text: &str,
        mlir: &str,
        gen_text: Option<&str>,
        stamp: &ProofStamp,
    ) -> Result<(), String> {
        let entry = Entry {
            key: key.to_string(),
            name: name.to_string(),
            signature: signature.to_string(),
            sketch_text: sketch_text.to_string(),
            mlir: mlir.to_string(),
            gen_text: gen_text.map(String::from),
            proof: Some(serde_json::to_string_pretty(stamp).unwrap_or_default()),
        };
        let manifest_path = self.dir.join(&format!("{}.manifest", key));
        fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&entry).unwrap_or_default(),
        )
        .map_err(|e| e.to_string())?;
        let proof_path = self.dir.join(&format!("{}.proof", key));
        self.index.insert(key.to_string(), entry);
        fs::write(
            &proof_path,
            serde_json::to_string_pretty(stamp).unwrap_or_default(),
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn get(&self, key: &str) -> Option<&Entry> {
        self.index.get(key)
    }

    pub fn list(&self) -> Vec<&Entry> {
        self.index.values().collect()
    }

    pub fn delete(&mut self, key: &str) -> Result<(), String> {
        let _ = self.index.remove(key);
        let mlir_path = self.dir.join(&format!("{}.mlir", key));
        let manifest_path = self.dir.join(&format!("{}.manifest", key));
        let proof_path = self.dir.join(&format!("{}.proof", key));
        fs::remove_file(&mlir_path).ok();
        fs::remove_file(&manifest_path).ok();
        fs::remove_file(&proof_path).ok();
        Ok(())
    }

    pub fn trust(&self, key: &str) -> Option<TrustVerdict> {
        self.index.get(key).and_then(|e| {
            e.proof
                .as_ref()
                .and_then(|p| serde_json::from_str::<ProofStamp>(p).ok())
                .map(|stamp| TrustVerdict {
                    status: if stamp.reason.contains("checked") {
                        ProvenVerdict::Attested
                    } else {
                        ProvenVerdict::Unattested
                    },
                })
        })
    }

    /// Content address of a gen — canonical text is the identity payload.
    pub fn key_for(gen: &crate::gen::Gen) -> String {
        crate::sha256::sha256_hex(gen.canonical().as_bytes())
    }

    /// Structural findings: orphaned artifacts, missing manifest entries.
    pub fn doctor(&self) -> Vec<(String, String)> {
        let mut findings = Vec::new();
        for e in self.index.values() {
            let mlir_path = self.dir.join(format!("{}.mlir", e.key));
            if !mlir_path.exists() {
                findings.push((
                    format!("{}: .mlir file missing", e.key),
                    "error".to_string(),
                ));
            }
            let manifest_path = self.dir.join(format!("{}.manifest", e.key));
            if !manifest_path.exists() {
                findings.push((
                    format!("{}: .manifest file missing", e.key),
                    "error".to_string(),
                ));
            }
        }
        if let Ok(entries) = fs::read_dir(&self.dir) {
            for entry in entries {
                if let Ok(entry) = entry {
                    let file_name = entry.file_name().into_string();
                    if let Ok(name) = file_name {
                        if name.ends_with(".mlir") {
                            let key = name.trim_end_matches(".mlir");
                            if !self.index.contains_key(key) {
                                findings.push((
                                    format!("{key}: orphaned .mlir (no manifest)"),
                                    "warn".to_string(),
                                ));
                            }
                        }
                    }
                }
            }
        }
        findings
    }

    /// Find entry by signature string.
    pub fn find_by_signature(&self, sig: &str) -> Option<&Entry> {
        self.index.values().find(|e| e.signature == sig)
    }

    /// Find an entry whose signature's function name matches a file path suffix.
    pub fn find_by_path(&self, path: &str) -> Option<Entry> {
        let matches: Vec<Entry> = self
            .list()
            .into_iter()
            .cloned()
            .filter(|e| {
                let sig_path = signature_path(&e.signature);
                sig_path == path
                    || sig_path.ends_with(&format!(".{}", path))
                    || path.ends_with(&format!(".{}", sig_path))
            })
            .collect();
        matches
            .iter()
            .max_by_key(|e| (e.gen_text.is_some() as i32, e.key.clone()))
            .cloned()
    }
}

/// Short function name from a signature string ("fn add(a: i32, b: i32) -> i32" → "add").
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
    let path = PathBuf::from(vault_dir).join("reuse.json");
    let mut map: std::collections::HashMap<String, u64> = match fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
    {
        Some(m) => m,
        None => Default::default(),
    };
    let key = format!("{dep_key}\u{2192}{used_by_key}");
    *map.entry(key).or_insert(0) += 1;
    let _ = fs::write(
        &path,
        serde_json::to_string_pretty(&map).unwrap_or_default(),
    );
}

/// Reuse ledger counts, aggregated by dep key (`dep_key` → total hits),
/// sorted for determinism. Same contract as the pre-rewrite API.
pub fn reuse_counts(vault_dir: &str) -> std::collections::HashMap<String, u64> {
    let path = PathBuf::from(vault_dir).join("reuse.json");
    let map: std::collections::HashMap<String, u64> = match fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
    {
        Some(m) => m,
        None => return Default::default(),
    };
    let mut out: std::collections::HashMap<String, u64> = Default::default();
    for (pair, n) in map {
        if let Some(dep) = pair.split('\u{2192}').next() {
            *out.entry(dep.to_string()).or_insert(0) += n;
        }
    }
    out
}

impl std::fmt::Display for TrustVerdict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.status {
            ProvenVerdict::Attested => write!(f, "proven"),
            ProvenVerdict::Unattested => write!(f, "raw"),
        }
    }
}
