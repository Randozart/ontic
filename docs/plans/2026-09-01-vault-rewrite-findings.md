# Vault Rewrite — Findings Snapshot (2026-09-01 15:20)

**Date:** 2026-09-01
**Timestamp:** 2026-09-01 15:20

Companion to `2026-08-31-vault-rewrite-repair-plan.md`. This snapshot records
the repo state at session start, so the repair work can be tracked against a
known baseline.

## Baseline state

- HEAD: `6543a22` "vault: rewrite as trust ledger — attestation, provenance,
  doctor, GC, export/import, path-keyed deletes" (clean at HEAD).
- Uncommitted working-tree changes: `src/vault.rs` (+273/−242), `src/lint.rs`
  (+17/−22), `src/nous.rs` (+19/−17), `src/program.rs` (+11/−1),
  `src/main.rs` (+42/−28).
- `cargo build`: **52 errors, all in `src/main.rs`** (stale old-API call sites).
- `cargo test --lib`: additionally 4 errors in `src/nous.rs` (L118, L239 —
  `Entry` literals missing `proof`) and `src/program.rs` (L454, L516 — stale
  `.expect(...)` on now-infallible `Vault::open`).
- Nothing runs: CLI, tests, smoke all red.

## API delta (old → new)

| Old | New |
|-----|-----|
| `Vault::open(&dir) -> Result<Vault, String>` | `Vault::open(&dir) -> Vault` (infallible) |
| `Vault::key_for(name)` | gone — key = `sha256(name)` |
| `put(name, sketch, mlir, pre, post, examples)` | `put_meta(name, sketch, pre, post, examples) -> (String, Vec<Artifact>)` |
| (n/a) | `put_proven(name, sketch, mlir, pre, post, examples, proof_json)` |
| `trust_of(name) -> (name, status, note)` | `trust(name) -> Option<TrustVerdict>` (`ProvenVerdict` enum incl. `Attested`) |
| `remove(name)` | `delete(name) -> Result<(), String>` |
| `list()` owned / `get(key)` owned | borrowed `&[Entry]` / `Option<&Entry>`; `Entry` gains `proof: Option<String>` |
| `doctor()` tuple | `doctor(&[&Path]) -> (Vec<String>, Vec<String>)` (needs referenced-kernel set) |
| `find_by_path(name)` | gone — `get(sha256(name))` |
| `lint_text(gens)` | `lint::lint_file(&[Gen], Option<&Vault>) -> Vec<LintReport>` (adds `PROV_UNATTAINED`) |
| `TrustStatus` | `ProvenVerdict` |

New helpers: `gc_orphans(&[&Path])`, `export(name, out)`, `import(pkg) ->
(keys, artifacts)`, `list_artifacts`, `provenance_of`.

## Key findings

1. **`main.rs`** needs a full port of ~30 call sites (open/expect ×8,
   `key_for` ×4, `put`, `trust_of` ×2, `remove`/`delete`, `doctor`/`gc` now
   path-taking, `import` tuple, borrowed `list/get`).
2. **Attestation plumbing is dead code** — no caller of `put_proven` exists in
   src/; `prove` is feature-gated. `cmd_forge` currently lands survivors via
   `put_meta` until `prove` lands.
3. **`TrustVerdict` mapping fragility**: `vault.rs::trust()` maps
   `reason.contains("checked")` → `Attested`. Fix: add
   `ProofStamp.attested: bool`.
4. **`cmd_lint` in main.rs is already on the new API** — verify
   `LintReport` field names match (`rule`/`path`/`detail` vs `code`/`message`).
5. **Docs**: README/`docs/` don't reference old API names; only CLI help text
   for `vault rm/doctor/gc` + a CHANGES.md entry need updating.

## Design defaults (resolved)

- `vault rm <prefix>`: keep **name-based UX** (resolve prefix → names via
  `list()`, `delete` per name).
- `vault doctor`: accept explicit recipe paths; fallback globs
  `examples/*.ont` + cwd `*.ont`. Orphans are relative to that recipe set.
- `ProofStamp`: add `attested: bool`; stop string-matching `reason`.

## Execution order

1. lib compiles (nous.rs, program.rs, lint field reconciliation).
2. main.rs port (top-down per error inventory in repair-plan doc).
3. vault.rs `attested` flag.
4. tests (put_proven→Attested, PROV_UNATTAINED, doctor orphans) + smoke.
5. CHANGES.md entry + CLI help text + commit.
