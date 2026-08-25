# `.nous` — Vault Export Package

**Date:** 2026-08-25
**Status:** approved, executing
**Name:** `.nous` — νοῦς, the intellect that grasps being (pairs with
*ontic*). Backronym: **N**ative **O**ntic **U**nified **S**tore.

## Goal

Export/import large slices of the vault as a single portable file.
`.ous` is one kernel; `.nous` is many kernels plus their dependency
graph, guarded twins, and the trust metadata needed to keep the
zero-false-accept story intact across machines.

## Design decisions (user-approved)

| Decision | Choice |
|----------|--------|
| Trust model | **Hybrid**: import lands `attested`; `--verify` re-runs the sieve locally and promotes to `verified` |
| Payload | LLVM obj **and** MLIR text; importer re-lowers when toolchain present |
| Scope v1 | Local files only: export / import / ls --json / dep closure / guarded twins. No signing, no network |
| Format | `NOUS1\n` magic + length-prefixed sections (same framing as `.ous`); zero deps |

## Spec-wrinkle this plan resolves

`.ous` ships the *candidate* sketch, not the *gen*. Canonical text
excludes opaque examples by design (GR3), so verify-on-import cannot
reconstruct the spec from existing manifests. Fix: solve stores raw
gen text in the manifest via the existing `extra_meta` plumbing
(`"gen_text"`). Old manifests without it export attest-only — honest
degradation, no format break.

## Format

```
NOUS1\n
[u64 LE len][TOC JSON]
[u64 LE len][OUS1 blob] × N      verbatim ous::pack_full output
[u64 LE len][EXTRA bytes] × K    .guarded.so / .guarded.c / .hpp
```

TOC JSON:

```json
{
  "format": "nous1",
  "generator": "ontic <version>",
  "created": "<iso8601>",
  "target": "<host triple or \"unknown\">",
  "entries": [
    {
      "key": "sha256…",
      "name": "matvec",
      "path": "Linalg.matvec",
      "signature": "fn Linalg.matvec(List<F64>, List<F64>) -> List<F64>",
      "deps": ["Stats.mean"],
      "quality": "full | edges_only",
      "guarded": true,
      "verifiable": true,
      "ous_index": 0,
      "extras": ["guarded_so", "guarded_c", "hpp"]
    }
  ],
  "extras": [
    { "key": "<parent key>", "kind": "guarded_so", "name": "libx-k.guarded.so" }
  ]
}
```

## Components

### 1. Vault additions (`src/vault.rs`)

- `cmd_solve` passes `"gen_text"` into `vault.put` extra_meta.
- Sidecar `.ontic/vault/trust.json`: `{key: "verified" | "attested"}`.
  Local-only, never shipped. Local solves write `verified`.
- Helpers: `trust_of(key)`, `set_trust(key, status)`.

### 2. Container (`src/nous.rs`, new)

- `NousEntry { entry: vault::Entry, obj: Vec<u8>, header: String,
  gen_text: Option<String>, quality: String,
  guarded: Vec<(String /*kind*/, Vec<u8>)> }`
- `pack(entries: &[NousEntry]) -> Result<Vec<u8>, String>`
- `unpack(data: &[u8]) -> Result<NousPackage, String>` — validates
  NOUS1 magic, per-entry OUS1 magic, TOC/key agreement.
- Round-trip + corruption negative tests.

### 3. Export CLI

```
ontic vault export [names…] [--all] --out pkg.nous
```

- Dep closure from `use` lines, topological order; missing dep = error
  naming it.
- Entry without `gen_text` → exported with `"verifiable": false`,
  warning printed.
- Summary table printed (names, keys, sizes).

### 4. Import CLI

```
ontic vault import pkg.nous [--verify] [--dry-run] [--force]
```

- Default: integrity-check TOC, land entries `attested`; key collisions
  skipped unless `--force`; guarded extras restored alongside.
- `--verify`: per entry, re-run sieve S1–S7 from shipped gen_text
  (hand candidate = shipped sketch). Deterministic ⇒ same verdict
  cross-machine. Pass → `verified`; fail → rejected with reasons,
  not landed. Sieve reuses `sieve::run_one` — no new verification code
  path (THE WALL preserved: same deterministic judge).
- `--dry-run`: print TOC summary, write nothing.
- Import never touches corpus/ONTIC_COLLECT feedstock.

### 5. Status surface

- `ontic vault ls` gains a TRUST column (or JSON field via `--json`).
- `ontic vault status <name>` optional convenience.

### 6. Docs

- `docs/NOUS.md`: format spec, trust-model rationale (why attested is
  second-class), worked example (export chain → import --verify),
  compatibility notes.
- README pointer, CHANGES.md entry.

## Verification

1. Unit: pack/unpack round-trip byte-equality; bad magic; truncation;
   TOC mismatch.
2. Unit: dep closure incl. missing-dep error; topo order.
3. Integration: export matvec chain → wipe vault dir → import →
   callable via FFI smoke test; `ls` shows attested.
4. Integration: import --verify promotes honest package to verified;
   tampered gen_text (example violating invariant) rejected pre-landing.
5. Full suite green; end-to-end commands run live.

## Commit legs

1. `feat(vault): nous container + gen_text/trust plumbing` — vault.rs,
   nous.rs + tests, solve-side plumbing.
2. `feat(cli): vault export/import with hybrid verify` — main.rs,
   docs/NOUS.md, CHANGES.md.

## Out of scope (v1)

- Detached signing / ed25519 (GR10 tension; revisit for distribution)
- Network fetch (`ontic get host/pkg.nous`)
- Cross-target obj bundles (single target triple per v1 package)
