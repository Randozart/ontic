//! `.ous` — single-file Ontic kernel bundle.
//!
//! Format: magic `OUS1\n`, then length-prefixed sections (u64 LE length +
//! bytes): MANIFEST(JSON), SKETCH(text), MLIR(text), LLVM-OBJ(binary),
//! HEADER(text). Hand-rolled reader/writer; zero deps.

use crate::vault::Entry;
use std::path::{Path, PathBuf};

const MAGIC: &[u8] = b"OUS1\n";

/// Read a u64 LE length prefix + payload from cursor position.
fn read_section(data: &[u8], pos: &mut usize) -> Option<Vec<u8>> {
    if *pos + 8 > data.len() {
        return None;
    }
    let len = u64::from_le_bytes(data[*pos..*pos + 8].try_into().ok()?) as usize;
    *pos += 8;
    if *pos + len > data.len() {
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

/// Pack a vault entry into a single `.ous` byte stream.
pub fn pack(entry: &Entry, obj_bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    // MANIFEST
    let manifest = serde_json::json!({
        "name": entry.name,
        "signature": entry.signature,
        "wrapping": entry.wrapping,
        "key": entry.key,
    });
    write_section(&mut out, manifest.to_string().as_bytes());
    // SKETCH
    write_section(&mut out, entry.sketch_text.as_bytes());
    // MLIR
    write_section(&mut out, entry.mlir.as_bytes());
    // OBJ
    write_section(&mut out, obj_bytes);
    // HEADER
    let header_path = PathBuf::from(format!("{}.h", entry.key));
    let _ = &header_path;
    // Header content comes from emit_header at call site; we store a marker
    // here and let callers pass it via `pack_with_header`.
    drop(header_path);
    out
}

/// Pack with explicit header text.
pub fn pack_full(entry: &Entry, obj_bytes: &[u8], header_text: &str) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    let manifest = serde_json::json!({
        "name": entry.name,
        "signature": entry.signature,
        "wrapping": entry.wrapping,
        "key": entry.key,
    });
    write_section(&mut out, manifest.to_string().as_bytes());
    write_section(&mut out, entry.sketch_text.as_bytes());
    write_section(&mut out, entry.mlir.as_bytes());
    write_section(&mut out, obj_bytes);
    write_section(&mut out, header_text.as_bytes());
    out
}

pub struct Unpacked {
    pub manifest: serde_json::Value,
    pub sketch_text: String,
    pub mlir: String,
    pub obj_bytes: Vec<u8>,
    pub header_text: String,
}

/// Unpack a `.ous` file into its sections.
pub fn unpack(data: &[u8]) -> Result<Unpacked, String> {
    if data.len() < MAGIC.len() || &data[..MAGIC.len()] != MAGIC {
        return Err("not an .ous file (bad magic)".to_string());
    }
    let mut pos = MAGIC.len();
    let manifest_raw = read_section(data, &mut pos)
        .ok_or("truncated: MANIFEST")?;
    let manifest: serde_json::Value = serde_json::from_slice(&manifest_raw)
        .map_err(|e| format!("manifest JSON: {}", e))?;
    let sketch = read_section(data, &mut pos).ok_or("truncated: SKETCH")?;
    let mlir = read_section(data, &mut pos).ok_or("truncated: MLIR")?;
    let obj = read_section(data, &mut pos).ok_or("truncated: OBJ")?;
    let header = read_section(data, &mut pos).ok_or("truncated: HEADER")?;
    Ok(Unpacked {
        manifest,
        sketch_text: String::from_utf8_lossy(&sketch).into_owned(),
        mlir: String::from_utf8_lossy(&mlir).into_owned(),
        obj_bytes: obj,
        header_text: String::from_utf8_lossy(&header).into_owned(),
    })
}

/// Read object file bytes for packing.
pub fn read_obj(path: &Path) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| format!("read {}: {}", path.display(), e))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entry() -> Entry {
        Entry {
            key: "abc123".to_string(),
            name: "mean".to_string(),
            signature: "fn Stats.mean(%xs: List<F64>) -> F64".to_string(),
            wrapping: true,
            sketch_text: "fn @mean(%xs: List<F64>) -> F64 { 0.0 }".to_string(),
            mlir: "module {\n  func.func @mean(%xs: memref<?xf64>) -> f64 {\n    return 0.0 : f64\n  }\n}".to_string(),
        }
    }

    #[test]
    fn test_pack_unpack_roundtrip() {
        let e = make_entry();
        let obj = vec![0xDE, 0xAD, 0xBE, 0xEF];
        let hdr = "// test header\ndouble mean(void*, void*, long, long, long);\n";
        let packed = pack_full(&e, &obj, hdr);

        assert!(packed.starts_with(MAGIC));

        let unpacked = unpack(&packed).unwrap();
        assert_eq!(unpacked.sketch_text, e.sketch_text);
        assert_eq!(unpacked.mlir, e.mlir);
        assert_eq!(unpacked.obj_bytes, obj);
        assert_eq!(unpacked.header_text, hdr);
        assert_eq!(unpacked.manifest["name"], "mean");
        assert_eq!(unpacked.manifest["wrapping"], true);
    }

    #[test]
    fn test_bad_magic_rejected() {
        assert!(unpack(b"NOTOUS").is_err());
        assert!(unpack(b"").is_err());
    }

    #[test]
    fn test_truncated_rejected() {
        let e = make_entry();
        let packed = pack_full(&e, &[1, 2, 3], "// h\n");
        // Cut off mid-payload.
        assert!(unpack(&packed[..packed.len() - 2]).is_err());
    }
}
