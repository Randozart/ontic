# Vault Rewrite — Current State & Repair Plan (2026-08-31)

## State

- HEAD: `6543a22` "vault: rewrite as trust ledger — attestation, provenance, doctor, GC, export/import, path-keyed deletes".
- **Uncommitted changes**: `src/vault.rs` (+273/−242), `src/lint.rs` (+17/−22), `src/nous.rs` (+19/−17), `src/program.rs` (+11/−1), `src/main.rs` (+42/−28). The rewrite is **half-landed**: `vault.rs` itself is internally consistent, but all *callers* still use the old API.
- Build state: `cargo build` fails with **52 errors**, all in `src/main.rs`. `cargo test --lib` additionally fails on 4 errors in `src/nous.rs` and `src/program.rs` (test modules). Nothing runs.

## The new API (what vault.rs now exposes)

| Old | New |
|-----|-----|
| `Vault::open(&dir) -> Result<Vault, String>` | `Vault::open(&dir) -> Vault` (infallible) |
| `Vault::key_for(name)` | **gone** — keys are sha256(name) via `Vault::key(name)` (private) or `sha256(name)` free fn |
| `put(name, sketch, mlir, pre, post, examples)` | `put_meta(name, sketch, pre, post, examples) -> (String, Vec<Artifact>)` (returns key) |
| (n/a) | `put_proven(name, sketch, mlir, pre, post, examples, proof_json)` |
| `trust_of(name) -> (name, status, note)` | `trust(name) -> Option<TrustVerdict>` (key-based; `TrustVerdict{status: ProvenVerdict, note, stamp}`) |
| `remove(name)` | `delete(name) -> Result<(), String>` (deletes `.mlir`/`.manifest.json`/`.proof`) |
| `list()` (owned) / `get(key)` (owned) | `list()` (borrowed `&[Entry]`) / `get(key) -> Option<&Entry>` |
| `doctor() -> (Vec<String>, Vec<String>)` | **gone** |
| `find_by_path(name)` | **gone** |
| `lint_text(gens) -> (Vec, Vec)` | `lint::lint_file(&[Gen], Option<&Vault>) -> Vec<LintReport>` (adds provenance cross-check `PROV_UNATTAINED` when vault present; `LintReport{code, message, severity}` with `Severity` enum) |
| `Entry` | adds `proof: Option<String>` |
| `ProvenVerdict` | replaces `TrustStatus` (adds `Attested`) |

New helpers on Vault: `doctor(&[&Path]) -> (Vec<String>, Vec<String>)`, `gc_orphans(&[&Path]) -> (removed, retained)`, `export(name, out)`, `import(pkg) -> (keys, artifacts)`, `list_artifacts`, `provenance_of`, `delete`.

## Error inventory

### main.rs (52 errors)
- **L377-384** (`cmd_solve`): `Vault::open().expect`, `key_for`, `list()` ownership, `get()` ownership, `put(...)` 6-arg, `put_meta` 6-arg (line 384 passes `mlir` as the sketch arg — wrong arity for new 5-arg signature).
- **L790-792** (`cmd_run`): `Vault::open().expect`, `list()`/`get()` ownership.
- **L840-847** (`cmd_run`): `key_for` → use `sha256(name)`.
- **L1006-1012** (`cmd_bench`): same open/expect/list/get pattern.
- **L1270-1274** (`cmd_check`): same.
- **L1300** (`cmd_check`): `trust_of` → `trust(key)`.
- **L1366, 1399, 1419, 1522** (`cmd_forge`): same open/expect/list/get pattern.
- **L1506** (`cmd_forge`): `remove` → `delete` (and key, not name).
- **L1646** (`cmd_vault ls`): `list()` ownership.
- **L1672-1685** (`cmd_vault status`): `key_for`, `get`, `trust_of`.
- **L1721-1742** (`cmd_vault rm`): `key_for`, `get`, `remove`; note: CLI still says "remove by name" — new `delete` is key-based; `vault rm <prefix>` should resolve prefix→name→`delete`.
- **L1744-1776** (`cmd_vault doctor`): `doctor()` free-form tuple and `find_by_path` gone. Rebuild using `Vault::doctor(&[recipe paths])`; note it now requires the recipe paths used by the project.
- **L1791** (`cmd_vault gc`): `gc_orphans(&[&Path])` now takes recipe paths (not `()`).
- **L1829** (`cmd_vault export`): `list()` ownership.
- **L1857** (`cmd_vault import`): `import` returns `(Vec<String>, Vec<Artifact>)` — update destructuring.
- **L1934-1950** (`cmd_lint`): `lint_text` tuple → `lint_file(&file.gens, Some(&v))`; findings are `Vec<LintReport>`; severity via `f.severity == Severity::Err/Warn`.
- **L1971-2007** (`cmd_export`): `list()` ownership; `get()` borrowed — `get(key).cloned()` if owned needed.

### nous.rs (test modules)
- **L118, 239**: `Entry { ... }` literals missing `proof` field → add `proof: None`.

### program.rs (test modules)
- **L454, 516**: `Vault::open(...).expect(...)` → drop `.expect(...)`.

### vault.rs self-tests
- Already updated to the new API (uses `put_meta`, `put_proven`, `trust`, `delete`, `doctor`, `gc_orphans`, `import`, `export`, `list_artifacts`, `provenance_of`). Should compile once lib builds.

## Repair plan (single coherent commit)

1. **`src/nous.rs` tests**: add `proof: None` to both `Entry` literals.
2. **`src/program.rs` tests**: drop `.expect("vault opens")` at L454, L516.
3. **`src/main.rs`**, top-down:
   a. Replace every `Vault::open(p).expect(...)` with `Vault::open(p)` (L377, 790, 1006, 1270, 1366, 1399, 1419, 1522).
   b. Introduce local keys where needed: `let key = ontic::sha256(name);` (replace `key_for` at L382, 840, 1672).
   c. Adapt `list()`/`get()` call sites to borrowed refs (use `&e` fields directly; clone if ownership required).
   d. `put` → `put_meta(name, &g.sketch, &g.pre, &g.post, &g.examples)` (L380); for forge's proven path, `put_proven(name, &cand.sketch, &mlir, pre, post, examples, &proof_json)` (L1395).
   e. `trust_of(name)` → `vault.trust(&key)` returning `Option<TrustVerdict>` (L1300, 1684). Render `trust.status` and `trust.note`.
   f. `remove` → `delete` (L1506). In `cmd_vault rm` (L1721-1742): resolve prefix to entries via `list()`, then call `delete(&e.name)` per match (name-based, as the CLI promises) — or change CLI to key. Prefer name-based to keep UX.
   g. `cmd_vault doctor` (L1744): call `v.doctor(&[recipe paths])`. Decide whether to require a `--recipes <path...>` flag or auto-glob `*.ont`; the old signature had no paths. Simplest: accept optional recipe paths from args, pass `&[]` if none (orphans check then compares against empty ⇒ everything orphaned — so require at least one path, or document the change).
   h. `cmd_vault gc` (L1791): same — `v.gc_orphans(&[paths])`; update the "orphans: none" message to use both `removed` and `retained`.
   i. `cmd_vault import` (L1857): destructure `(keys, _artifacts)`.
   j. `cmd_lint` (L1934-1950): `let findings = ontic::lint::lint_file(&file.gens, Some(&v));` and match on `f.severity` (`Severity::Err/Warn/Info`).
   k. `cmd_export` (L1971-2007): adapt to borrowed `list()`; clone `Entry` if `nous::write_package` needs owned.
4. **Docs**: `docs/PLANS.md` (vault rewrite row: mark Done), `docs/GUARDS.md` (if it references `trust_of`/`doctor` old API), README `vault rm/doctor` help text. CHANGES.md entry.
5. **Verify**: `cargo test --lib` green; `cargo build --release`; smoke `ontic check examples/ledger.ont`, `ontic vault ls`, `ontic vault doctor examples/*.ont`, `ontic lint examples/ledger.ont`.
6. **Commit** with a message like "vault: finish trust-ledger rewrite — port CLI + tests to new API".

## Additional findings (2nd pass)

- **`cmd_lint` (L1925-1952)** is already written against the *new* API — `lint_file(&file.gens, vault.as_ref())`, `Severity::Err/Warn/Info`, `f.rule`, `f.path`, `f.detail`. It is correct as-is; the earlier note's "L1934-1950" error group was a misread of error lines (those were `list()`-ownership sites elsewhere). Verify `LintReport` actually has `rule`/`path`/`detail` fields; if lint.rs only has `code`/`message`, either rename fields or adapt main.rs.
- **`cmd_vault import` (L1744-1776)** still calls `Vault::key_for(&w)` and `v.find_by_path(d)` — both gone. Replace with `ontic::sha256(&w.name)` and `v.get(sha256(d))`.
- **`cmd_vault export` (L1600)**: "attest-only" messaging already anticipates the new model — consistent, keep.
- **Attestation plumbing is UNEXERCISED**: no caller of `put_proven` exists in src/. The `proof`/`Attested` machinery is dead code until `cmd_forge` (or `cmd_prove`) starts writing `.proof` files. `prove` is behind `--features proven` (currently stub/feature-gated per main.rs L91).
- **`TrustVerdict` mapping** in `vault.rs` (`reason.contains("checked")`) is fragile string matching.

## Refined execution plan (one session, ~4-6 steps)

### Step 0 — Decide the 3 open questions (ask user or default)
1. `vault rm <prefix>`: **keep name-based UX** (resolve prefix→names via `list()`, call `delete` per name). Default: yes.
2. `vault doctor`: new `doctor(&[paths])` needs referenced kernels. **Default: glob `examples/*.ont` + `./**/*.ont` when no explicit `--recipes` flag**; document that orphans are relative to the recipe set. (Alternative: require the flag.)
3. `ProofStamp`: add `attested: bool` field, stop string-matching `reason`. Default: do it (small, in vault.rs).

### Step 1 — lib compiles (test modules + lint field names)
- `src/nous.rs` L118, L239: add `proof: None` to `Entry` literals.
- `src/program.rs` L454, L516: drop `.expect("vault opens")`.
- Check `lint.rs` `LintReport` field names vs main.rs usage (`rule`/`path`/`detail` vs `code`/`message`); reconcile (prefer fixing lint.rs field names to match the already-written main.rs call site, or vice versa — whichever is smaller).
- `cargo build --lib` green.

### Step 2 — main.rs port (52 errors, top-down)
Follow the inventory above. Key transformations:
- `Vault::open(p).expect(..)` → `Vault::open(p)` (8 sites).
- `Vault::key_for(n)` → `ontic::sha256(n)` (L382, 840, 1672, 1751).
- `v.find_by_path(d)` → `v.get(ontic::sha256(d))` (L1756).
- `put(name, sketch, mlir, pre, post, ex)` → `put_meta(name, sketch, pre, post, ex)` (L380); forge proven path → `put_proven(...)` (L1395) **only if** forge emits proof JSON — otherwise `put_meta`.
- `trust_of(n)` → `v.trust(&key)` → `Option<TrustVerdict>` (L1300, 1684); render `status` + `note`.
- `remove(n)` → `delete(n)` (L1506); `cmd_vault rm` resolves prefix→names (L1721-1742).
- `cmd_vault doctor` → `v.doctor(&[recipe_paths])` with glob default (L1744+).
- `cmd_vault gc` → `v.gc_orphans(&[recipe_paths])` (L1791); message shows removed + retained.
- `import` returns `(keys, artifacts)` (L1857).
- `list()`/`get()` borrowed: use `&e.field` directly; `.cloned()` where ownership needed (nous export).
- `cargo build` green.

### Step 3 — wire attestation (optional, follow-up)
- `cmd_forge`: when a survivor is attested by a benchmark/proof, call `put_proven` with a `ProofStamp` JSON (`attested: true`, reason, seed, timestamp). Until `prove` lands, forge uses `put_meta` (proof stays `None`, verdict = Verified).
- `vault.rs` `trust()`: read `stamp.attested` instead of `reason.contains("checked")`.

### Step 4 — tests + smoke
- `cargo test --lib` green (vault self-tests already on new API).
- Add: (a) `put_proven` → `trust()` returns `Attested` test; (b) `PROV_UNATTAINED` lint rule test (gen with `deps` on an unattested kernel); (c) `doctor` orphan detection with explicit recipe paths.
- Smoke: `ontic check examples/ledger.ont`, `ontic lint examples/ledger.ont`, `ontic vault ls`, `ontic vault status total`, `ontic vault doctor examples/*.ont`, `ontic export examples/ledger.ont --out /tmp/x.nous`, `ontic vault import /tmp/x.nous --verify`.

### Step 5 — docs + commit
- `docs/PLANS.md`: vault-rewrite row → Done.
- `docs/GUARDS.md` + README: update `vault rm/doctor/gc` descriptions (path-based doctor; name-based rm).
- CHANGES.md entry (timestamped per AGENTS.md).
- `cargo test --lib` green → commit "vault: finish trust-ledger rewrite — CLI + tests on new API".

## Open design questions (resolved by Step 0 defaults above)

- **`vault rm <prefix>`**: new `delete` is key-based; old UX removed by name prefix. Keep name-based UX by resolving through `list()` (recommended), or switch CLI to keys (uglier, leaks sha256 into UX).
- **`vault doctor` recipe paths**: new `doctor(paths)` needs the set of referenced kernels to find orphans. Options: (a) add `--recipes` flag; (b) glob `examples/*.ont` + cwd `*.ont`; (c) keep old no-arg `doctor()` as a wrapper that globs. Recommend (a) explicit + (b) fallback.
- **`ProvenVerdict::Attested`**: `trust()` maps any `ProofStamp` with `reason.contains("checked")` → Attested, else Verified. This is fragile string matching; consider `ProofStamp.attested: bool` field instead.
