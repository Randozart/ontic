//! `.nous` — multi-kernel vault export package.
//!
//! Format: magic `NOUS1\n`, then length-prefixed sections (u64 LE length +
//! bytes), same framing discipline as `.ous`:
//!
//! ```text
//! NOUS1\n
//! [u64][TOC JSON]          package index: version, target, entries, extras
//! [u64][OUS1 blob] × N     verbatim ous::pack_full output per kernel
//! [u64][EXTRA bytes] × K   guarded shims / headers, TOC-referenced by index
//! ```
//!
//! Trust model lives OUTSIDE the container (packages carry no verdicts):
//! imports land `attested`; `--verify` re-runs the deterministic sieve
//! locally and promotes to `verified`. The container only guarantees
//! integrity of what was packed.

use crate::vault;
use std::path::Path;

const MAGIC: &[u8] = b"NOUS1\n";

/// One kernel plus everything needed to reconstruct it elsewhere.
#[derive(Debug, Clone)]
pub struct NousEntry {
    /// Vault entry (manifest facts + sketch + mlir + gen_text).
    pub entry: vault::Entry,
    /// Raw manifest JSON exactly as packed — imports restore it verbatim
    /// so provenance survives the round trip.
    pub manifest: serde_json::Value,
    /// Compiled LLVM object bytes (arch-specific; importer may re-lower).
    pub obj: Vec<u8>,
    /// C header text.
    pub header: String,
    /// Probe-plan quality at solve time: "full" | "edges_only".
    pub quality: String,
    /// Extra artifacts: (kind, bytes). Kinds: "guarded_so", "guarded_c",
    /// "hpp". Order preserved; TOC references by position.
    pub extras: Vec<(String, Vec<u8>)>,
}

/// An unpacked package: TOC plus decoded payloads.
#[derive(Debug, Clone)]
pub struct NousPackage {
    pub generator: String,
    pub created_unix: u64,
    pub target: String,
    pub entries: Vec<NousEntry>,
}

/// Read a u64 LE length prefix + payload.
fn read_section(data: &[u8], pos: &mut usize) -> Option<Vec<u8>> {
    if *pos + 8 > data.len() {
        return None;
    }
    let len = u64::from_le_bytes(data[*pos..*pos + 8].try_into().ok()?) as usize;
    *pos += 8;
    if len > data.len() || *pos + len > data.len() {
        return None;
    }
    let out = data[*pos..*pos + len].to_vec();
    *pos += len;
    Some(out)
}

fn write_section(out: &mut Vec<u8>, data: &[u8]) {
    out.extend_from_slice(&(data.len() as u64).to_le_bytes());
    out.extend_from_slice(data);
}

/// Best-effort host target tag for the TOC (advisory only).
fn host_target() -> String {
    format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS)
}

/// Pack kernels into a single `.nous` byte stream.
pub fn pack(entries: &[NousEntry]) -> Result<Vec<u8>, String> {
    if entries.is_empty() {
        return Err("refusing to pack an empty package".to_string());
    }
    // TOC
    let mut toc_entries = Vec::new();
    for e in entries {
        let kinds: Vec<&str> = e.extras.iter().map(|(k, _)| k.as_str()).collect();
        toc_entries.push(serde_json::json!({
            "key": e.entry.key,
            "name": e.entry.name,
            "signature": e.entry.signature,
            "quality": e.quality,
            "verifiable": e.entry.gen_text.is_some(),
            "guarded": e.extras.iter().any(|(k, _)| k == "guarded_so"),
            "extras": kinds,
        }));
    }
    let toc = serde_json::json!({
        "format": "nous1",
        "generator": format!("ontic {}", env!("CARGO_PKG_VERSION")),
        "created_unix": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        "target": host_target(),
        "entries": toc_entries,
    });
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    write_section(
        &mut out,
        serde_json::to_string_pretty(&toc)
            .map_err(|e| e.to_string())?
            .as_bytes(),
    );
    // Payloads: one .ous blob per entry, then extras flat in order.
    for e in entries {
        let blob = crate::ous::pack_full(&e.entry, &e.obj, &e.header);
        write_section(&mut out, &blob);
    }
    for (_, bytes) in entries.iter().flat_map(|e| e.extras.iter()) {
        write_section(&mut out, bytes);
    }
    Ok(out)
}

/// Parse and validate a `.nous` stream. Structural checks only — trust
/// decisions belong to the import command, never the container.
pub fn unpack(data: &[u8]) -> Result<NousPackage, String> {
    if data.len() < MAGIC.len() || &data[..MAGIC.len()] != MAGIC {
        return Err("not a nous package (bad magic)".to_string());
    }
    let mut pos = MAGIC.len();
    let toc_raw = read_section(data, &mut pos).ok_or("truncated TOC")?;
    let toc: serde_json::Value =
        serde_json::from_str(std::str::from_utf8(&toc_raw).map_err(|e| e.to_string())?)
            .map_err(|e| format!("TOC parse failed: {}", e))?;
    if toc["format"].as_str() != Some("nous1") {
        return Err(format!(
            "unsupported package format `{}`",
            toc["format"].as_str().unwrap_or("?")
        ));
    }
    let toc_entries = toc["entries"]
        .as_array()
        .ok_or("TOC missing entries array")?;
    let n_extras: usize = toc_entries
        .iter()
        .map(|e| e["extras"].as_array().map(|a| a.len()).unwrap_or(0))
        .sum();

    // Pass 1: all kernel payloads (order matches TOC entries).
    let mut blobs = Vec::new();
    for _ in toc_entries {
        let blob = read_section(data, &mut pos).ok_or("truncated entry payload")?;
        blobs.push(blob);
    }
    // Pass 2: extras, flat in TOC declaration order.
    let mut extra_bytes = Vec::new();
    for _ in 0..n_extras {
        let b = read_section(data, &mut pos).ok_or("truncated extra payload")?;
        extra_bytes.push(b);
    }

    let mut entries = Vec::new();
    let mut extra_iter = extra_bytes.into_iter();
    for (te, blob) in toc_entries.iter().zip(blobs.into_iter()) {
        let un = crate::ous::unpack(&blob)?;
        // TOC/key agreement: the manifest inside the .ous must match the
        // index that claims it.
        if un.manifest["key"].as_str() != te["key"].as_str() {
            return Err(format!(
                "TOC key mismatch for `{}`",
                te["name"].as_str().unwrap_or("?")
            ));
        }
        let mut extras = Vec::new();
        for kind in te["extras"].as_array().unwrap_or(&vec![]) {
            let kind = kind.as_str().ok_or("non-string extra kind")?.to_string();
            let bytes = extra_iter.next().ok_or("missing extra payload")?;
            extras.push((kind, bytes));
        }
        entries.push(NousEntry {
            manifest: un.manifest.clone(),
            entry: vault::Entry {
                key: un.manifest["key"]
                    .as_str()
                    .ok_or("manifest missing key")?
                    .to_string(),
                name: un.manifest["name"]
                    .as_str()
                    .ok_or("manifest missing name")?
                    .to_string(),
                signature: un.manifest["signature"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                sketch_text: un.sketch_text.clone(),
                gen_text: un.manifest["gen_text"].as_str().map(String::from),
                mlir: un.mlir.clone(),
                proof: un.manifest["proof"].as_str().map(String::from),
                // Tier rides the shipped manifest; old packages parse as
                // checked (serde default) — the safe direction.
                tier: un
                    .manifest
                    ["tier"]
                    .as_str()
                    .map(String::from)
                    .unwrap_or_else(|| "checked".to_string()),
            },
            obj: un.obj_bytes.clone(),
            header: un.header_text.clone(),
            quality: te["quality"].as_str().unwrap_or("unknown").to_string(),
            extras,
        });
    }
    // Trailing garbage check: every claimed section must be consumed.
    let expected_end = pos;
    let _ = n_extras;
    if expected_end != data.len() {
        return Err(format!(
            "package has {} trailing bytes after {} declared sections",
            data.len() - expected_end,
            entries.len()
        ));
    }
    let generator = toc["generator"].as_str().unwrap_or("?").to_string();
    Ok(NousPackage {
        generator,
        created_unix: toc["created_unix"].as_u64().unwrap_or(0),
        target: toc["target"].as_str().unwrap_or("?").to_string(),
        entries,
    })
}

/// Write a package to disk atomically enough for v1: temp file + rename.
pub fn write_to(path: &Path, data: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("nous.tmp");
    std::fs::write(&tmp, data).map_err(|e| format!("temp write failed: {}", e))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("rename failed: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_entry(key: &str, name: &str, verifiable: bool) -> NousEntry {
        NousEntry {
            manifest: serde_json::json!({"key": key, "name": name}),
            entry: vault::Entry {
                key: key.to_string(),
                name: name.to_string(),
                signature: format!("fn T.{name}(%x: Int) -> Int"),
                sketch_text: format!("fn @{name}(%x: Int) -> Int {{ %x }}"),
                gen_text: if verifiable {
                    Some(format!("fn T.{name}(%x: Int) -> Int\n  => 1 -> 1\n"))
                } else {
                    None
                },
                mlir: format!("module {{ func.func @{name}() -> i64 }}"),
                proof: None,
                tier: "checked".to_string(),
            },
            obj: vec![0xDE, 0xAD],
            header: format!("long {name}(long x);"),
            quality: "full".to_string(),
            extras: vec![("guarded_so".to_string(), vec![0x01, 0x02, 0x03])],
        }
    }

    #[test]
    fn test_roundtrip_preserves_everything() {
        let entries = vec![
            sample_entry(&"a".repeat(64), "alpha", true),
            sample_entry(&"b".repeat(64), "beta", false),
        ];
        let packed = pack(&entries).unwrap();
        let pkg = unpack(&packed).unwrap();
        assert_eq!(pkg.entries.len(), 2);
        assert_eq!(pkg.entries[0].entry.key, "a".repeat(64));
        assert_eq!(pkg.entries[0].obj, vec![0xDE, 0xAD]);
        assert_eq!(pkg.entries[0].extras[0].0, "guarded_so");
        assert_eq!(pkg.entries[0].extras[0].1, vec![0x01, 0x02, 0x03]);
        assert!(pkg.entries[0].entry.gen_text.is_some());
        assert!(pkg.entries[1].entry.gen_text.is_none());
        assert_eq!(
            pkg.generator,
            format!("ontic {}", env!("CARGO_PKG_VERSION"))
        );
    }

    #[test]
    fn test_roundtrip_deterministic_payload() {
        // Two packs of identical input must decode identically (the TOC
        // carries a creation timestamp; decoded payloads must not).
        let mk = || vec![sample_entry(&"c".repeat(64), "gamma", true)];
        let a = unpack(&pack(&mk()).unwrap()).unwrap();
        let b = unpack(&pack(&mk()).unwrap()).unwrap();
        assert_eq!(a.entries.len(), b.entries.len());
        for (ea, eb) in a.entries.iter().zip(b.entries.iter()) {
            assert_eq!(ea.entry.key, eb.entry.key);
            assert_eq!(ea.entry.mlir, eb.entry.mlir);
            assert_eq!(ea.obj, eb.obj);
            assert_eq!(ea.header, eb.header);
            assert_eq!(ea.extras, eb.extras);
        }
    }

    #[test]
    fn test_bad_magic_rejected() {
        assert!(unpack(b"NOT_A_PACKAGE").is_err());
        assert!(unpack(b"").is_err());
    }

    #[test]
    fn test_truncation_rejected() {
        let packed = pack(&[sample_entry(&"d".repeat(64), "delta", true)]).unwrap();
        for cut in [5usize, 9, 20, packed.len() - 3] {
            assert!(unpack(&packed[..cut]).is_err(), "cut at {cut} accepted");
        }
    }

    #[test]
    fn test_empty_package_refused_at_pack_time() {
        assert!(pack(&[]).is_err());
    }

    #[test]
    fn test_trailing_garbage_rejected() {
        let mut packed = pack(&[sample_entry(&"e".repeat(64), "eps", true)]).unwrap();
        packed.push(0x00);
        assert!(unpack(&packed).is_err());
    }
}
