# Plan: Complete Vault API Rewrite — 2026-08-31

## Context

The uncommitted rewrite of `src/vault.rs` and `src/lint.rs` introduces a modular
vault design (4 sub-structs, enum trust, proof stamps, borrowed access) and a
`LintReport` struct. The build is broken (41 errors) because:
- 4 internal type bugs in `vault.rs`
- ~40 caller sites in `main.rs` not updated for the new API
- 14 lint call sites still pattern-match the old tuple shape

## Trust Model (from GUARDS.md)

```
PROVEN > GUARDED > RAW > NONE
```

- PROVEN: Z3 SMT machine-checked proof
- GUARDED: C shim wrapper checking preconditions at call time
- RAW: raw .so, zero overhead, no runtime checks
- NONE: no artifacts

## API Changes (Old → New)

| Aspect | Old (committed) | New (uncommitted) |
|--------|-----------------|-------------------|
| Structure | Flat `Vault` struct | 4 sub-structs: Manifest, Entries, Trust, Artifacts |
| Trust | `trust_of(key) -> &'static str` | `trust(key) -> Option<TrustVerdict>` (enum) |
| Proof | none | `ProofStamp` + `ProvenVerdict` per entry |
| Access | Owned (`Entry`, `Vec<Entry>`) | Borrowed (`&Entry`, `Vec<&Entry>`) |
| Key | `Vault::key_for(gen)` | Free `sha256::digest(canonical(gen))` |
| put | `put(gen, sketch, mlir)` + `put_meta(...)` | `put(key, sketch, mlir, gen_text, sig, pre, post)` |
| doctor | `doctor() -> Vec<(String, String)>` | gone |
| find_by_path | `find_by_path(path) -> Option<Entry>` | gone |
| open | `Result<Self, String>` | `Self` (never fails) |
| remove | `remove(key) -> Result<Vec<PathBuf>, String>` | `delete(key) -> Result<(), String>` |
| lint | `(&'static str, Vec<LintFinding>)` | `LintReport { findings: Vec<LintFinding> }` |

## Steps

### Step 1: Fix 4 internal type errors in vault.rs

1. **Line 110**: `entry.gen_text.clone()` → `entry.gen_text.as_deref().unwrap_or("").to_string()`
   - `gen_text` is `Option<String>`, can't `.clone()` directly in `json!` context
   - Fix: use `.as_deref().unwrap_or("")` or match

2. **Line 152**: `entry.proof.as_ref().map(...)` → `entry.proof.as_ref().and_then(|p| ...)`
   - `proof` is `Option<ProvenVerdict>`; `.as_ref()` gives `Option<&ProvenVerdict>`
   - The map closure needs to handle the reference correctly

3. **Line 160**: `entry.proof.as_ref().and_then(|p| p.stamp.as_ref()).map(|s| s.tier.as_str())`
   - `tier` is `String`, not `&'static str` — `.as_str()` returns `&str` with lifetime tied to `s`
   - Fix: return `Option<String>` or restructure to avoid the lifetime issue

4. **Line 198**: Same `proof.as_ref()` issue in `delete()`
   - The closure captures `entry` by reference but borrows conflict
   - Fix: extract the proof tier before the closure, or restructure

### Step 2: Re-add doctor() method

The `cmd_vault_doctor` in main.rs calls `v.doctor()` expecting `Vec<(String, String)>`.
The new design removed it. Re-add as a method on `Vault`:

```rust
/// Structural findings: orphaned artifacts, missing manifest entries.
pub fn doctor(&self) -> Vec<(String, String)> {
    // Check: .o/.so files without manifest entry → orphan
    // Check: manifest entries without .mlir file → missing
    // Check: trust.json entries without manifest entry → orphan trust
}
```

### Step 3: Add find_by_signature()

`cmd_vault_import` uses `find_by_path(path)` to check if a key already exists.
The new design removes it. Add:

```rust
/// Find entry by signature string (e.g. "add(i32, i32) -> i32").
pub fn find_by_signature(&self, sig: &str) -> Option<&Entry> {
    self.entries.find_by_signature(sig)
}
```

Or update `cmd_vault_import` to iterate `list()` and match.

### Step 4: Add Vault::key_for convenience

8 call sites in main.rs use `Vault::key_for(gen)`. Rather than updating all,
re-add a thin wrapper:

```rust
/// Content address of a gen — canonical text is the identity payload.
pub fn key_for(gen: &Gen) -> String {
    crate::sha256::sha256_hex(gen.canonical().as_bytes())
}
```

### Step 5: Update main.rs vault call sites

Tracing all call sites:

| Line | Old API | New API |
|------|---------|---------|
| 150 | `Vault::open(path).ok()` | `Vault::open(path)` (no Result) |
| 546 | `Vault::key_for(&w)` | `Vault::key_for(&w)` (re-added) |
| 547-553 | `v.put(&w, ...)` + `v.put_meta(...)` | `v.put(key, sketch, mlir, gen_text, sig, pre, post)` |
| 841 | `Vault::key_for(w)` | `Vault::key_for(w)` (re-added) |
| 843 | `v.trust_of(&w_key)` | `v.trust(&w_key).map(\|t\| match t { ... })` |
| 972 | `Vault::key_for(w)` | `Vault::key_for(w)` (re-added) |
| 1196 | `Vault::open(vd).ok()` | `Vault::open(vd)` |
| 1337 | `Vault::open(vd).ok()` | `Vault::open(vd)` |
| 1379 | `Vault::open(vd).ok()` | `Vault::open(vd)` |
| 1567 | `crate::vault::key_for(&w)` | `crate::vault::Vault::key_for(&w)` |
| 1746 | `Vault::key_for(&w)` | `Vault::key_for(&w)` (re-added) |
| 1747 | `v.find_by_path(path)` | `v.find_by_signature(sig)` or iterate |
| 1935 | `Vault::open(path).ok()` | `Vault::open(path)` |
| 1981 | `Vault::open(vd).ok()` | `Vault::open(vd)` |
| 1988 | `v.doctor()` | `v.doctor()` (re-added) |
| 2037 | `Vault::open(vd).ok()` | `Vault::open(vd)` |
| 2061 | `Vault::open(vd).ok()` | `Vault::open(vd)` |
| 2168 | `Vault::key_for(g)` | `Vault::key_for(g)` (re-added) |
| 2180 | `v.remove(key)` | `v.delete(key)` |
| 2254 | `Vault::open(vd).ok()` | `Vault::open(vd)` |
| 2300 | `v.remove(&e.key)` | `v.delete(&e.key)` |
| 2368 | `Vault::key_for(&g)` | `Vault::key_for(&g)` (re-added) |
| 3128 | `Vault::key_for(&g)` | `Vault::key_for(&g)` (re-added) |
| 3312 | `Vault::key_for(&g)` | `Vault::key_for(&g)` (re-added) |
| 3526 | `Vault::key_for(&g)` | `Vault::key_for(&g)` (re-added) |

Additional changes:
- `v.list()` returns `Vec<&Entry>` not `Vec<Entry>` — adjust `for e in v.list()` loops
- `v.get(key)` returns `Option<&Entry>` not `Option<Entry>` — adjust
- `v.trust_of(key)` → `v.trust(key)` returning `Option<TrustVerdict>` enum
- `v.trust_map()` → iterate `list()` + call `trust(key)` per entry
- `match v.open(path)` → `let v = Vault::open(path)` (no Result)
- `v.put(gen, sketch, mlir)` → `v.put(key, sketch, mlir, gen_text, sig, pre, post)`
- `v.put_meta(...)` → removed (folded into put)

### Step 6: Fix lint.rs consumers (14 sites)

`lint_text` now returns `LintReport` instead of `(&'static str, Vec<LintFinding>)`.
Update pattern matches:

Old:
```rust
let (sev, findings) = lint::lint_text(src);
match sev { "ok" => ..., "warn" => ..., "error" => ... }
```

New:
```rust
let report = lint::lint_text(src);
let sev = if report.findings.iter().any(|f| f.severity == Severity::Err) { "error" }
    else if !report.findings.is_empty() { "warn" }
    else { "ok" };
```

Or add a `LintReport::severity() -> &'static str` helper method.

### Step 7: cargo test --lib green

Run full test suite. Fix any test that references old API.

### Step 8: Commit + CHANGES.md entry

```
Vault API modernisation: modular sub-structs (manifest/entries/trust/artifacts),
enum trust verdicts (Proven/Guarded/Raw), proof stamps per entry, borrowed
access, LintReport struct. Callers updated.
```

## Open Decisions

1. **doctor()**: Re-add as method (simplest) or inline into cmd_vault_doctor?
   → Re-add as method. Less churn in main.rs.

2. **find_by_path → find_by_signature**: The old `find_by_path` matched on
   the path string in the manifest. The new design stores `signature` in
   entries. Add `find_by_signature()` or update callers to iterate.
   → Add `find_by_signature()`. Cleaner.

3. **TrustVerdict display**: The enum needs a `Display` impl for printing.
   Or match on variants in main.rs.
   → Add `impl fmt::Display for TrustVerdict`.

4. **lint LintReport::severity()**: Add a helper method to derive severity
   from findings.
   → Add `pub fn severity(&self) -> &'static str`.
