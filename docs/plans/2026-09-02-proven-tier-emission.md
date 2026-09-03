# Proven Tier Emission — Plan (2026-09-02)

**Status: done (2026-09-03).** Commits `9f5fe58` (tier-aware emission +
trust stamps) and `19c4106` (solve wiring + equivalence gate). All 9
steps landed as planned; default build 183/183 green. `--features
proven` paths are compile-gated but not execution-verified on this
machine (z3 absent — the plan's accepted risk).

**Date:** 2026-09-02
**Timestamp:** 2026-09-02 22:20

Completes the proven tier started by `3085c8e` (z3 overflow-absence
proofs): flag-free codegen for provably-safe kernels, per production-master
P7 acceptance. Survey: `2026-09-02-next-work-survey.md`.

## Pre-flight findings (verified against code, 2026-09-02 evening)

- **z3 is NOT installed on the dev machine** (`cargo test --features proven`
  fails in z3-sys bindings: no `z3.h`/`libz3`). The gate stays at the
  DEFAULT build (177 tests); `--features proven` tests are a MANUAL check,
  run once z3 is installed. Feature-gated code must still be
  `cargo check --features proven` clean where toolchain allows.
- **`emit_shim_c` is invariant-driven, not overflow-driven**: guarded shims
  compile gen invariants into C guards; there are no overflow preconditions
  to skip. Proven kernels get the same structural guards automatically.
  D3's "shim skip" is therefore cosmetic: the only real difference is
  omitting the `@ontic_trap`/`@ontic_trapf` declarations from proven MLIR
  (unused declarations are legal; omit for cleanliness, keep in checked).
- **Vault landing site is one**: `cmd_solve` (`main.rs:1147`, `v.put`);
  forge runs through the same solve path. Commit 2 wires a single site.
- **`trust()` reads `Entry.proof` (in-memory)**, but the authoritative stamp
  lives in `{key}.stamp.json`. After F2 rewiring, `trust()` must read the
  stamp FILE (fall back to in-memory proof for legacy entries), otherwise
  `attested` is ignored. The `reason.contains("checked")` match dies.
- **`Entry` and `Manifest` share 8 identical fields**; factor a shared
  `EntryPayload` (serde surface) rather than duplicating `tier` twice.
- **`set_trust` call sites**: only `cmd_vault_import` (`main.rs:1685`).
  Its `status` comes from the `.nous` manifest's `trust` field — the
  import path keeps a legacy shim until NOUS stamps carry real stamps
  (out of scope here; noted for the F2 follow-up).
- **`vault ls`/`status` trust rendering** is at `main.rs:1443` (`trust()`)
  and the `Display for TrustVerdict` in vault.rs ("proven"/"raw") — the
  tier badge composes with the existing verdict text, not replacing it.

## Design decisions

### D1 — The contract word is the recorded proof (GR11 amendment)

GR11: "the fast path never exists without a visible contract word". We
treat the machine-recorded PROVEN verdict (ProofStamp in the manifest) as
that word: the emitter refuses to emit flag-free code absent a recorded
proof. No grammar change, no vault-key churn for existing gens. GR11 text
amended: the declaration is the recorded proof, not an author-claimed word.

### D2 — Shared proven-subset check

`prove.rs` already knows which shapes are provable (`unsupported_shape`,
`count_arith`, `contains_neg`). Factor these into `prove::subset_ok(gen,
cand) -> Option<String>` (Some = honest reason NOT provable) so
`lower.rs` selection cannot drift from `prove.rs` encoding. The emitter
uses the SAME gate: whole body within subset AND `ret`/params scalar-Int.

### D3 — Emission selection

- `lower::emit_fn` gains an optional tier param or a wrapper
  `emit_fn_tier(name, params, ret, body, calls, tier)`.
- Tier = Proven ⇒ arith sites emit `arith.addi/subi/muli : i64` directly
  (no i128, no trap, no scf.if). Neg: `arith.subi 0, x`.
- Tier = Checked (default) ⇒ today's `emit_checked_arith`, unchanged.
- Guarded `.so` shim: proven kernels get NO precondition trap shim for
  overflow (the point of the tier); structural guards (list lengths) stay.
  Honest scope: proven covers arithmetic traps only.
- **Equivalence gate**: before a proven emission lands, the proven module
  and the checked module are both executed on the same probe rows (differ-
  ential parity drivers, existing machinery in `pipeline.rs`); any row
  mismatch ⇒ fall back to checked emission (never a weaker sieve).

### D4 — Trust ledger wiring

- `ProofStamp` gains `attested: bool` (`#[serde(default)]` so legacy
  stamps parse). `vault.trust()` reads the `{key}.stamp.json` FILE
  (falling back to the in-memory `Entry.proof` for legacy entries) and
  maps `attested → Attested` — kills `reason.contains("checked")`.
- `set_trust` takes `&ProofStamp` instead of a string verdict (writes the
  stamp file + manifest). The one legacy caller (`cmd_vault_import`)
  builds a `ProofStamp` from the `.nous` manifest's trust field via a
  small helper (import provenance is not z3 attestation — it stamps
  `attested: false` with reason "nous import").
- `put_proven` becomes the landing path for solve/forge survivors with a
  recorded Proven verdict. A shared `EntryPayload` struct (the 8 common
  fields of `Entry`/`Manifest`) gains `tier: String`
  (`#[serde(default = "default_tier")]` → "checked") so old manifests
  parse unchanged; `Entry` and `Manifest` embed it (no field
  duplication).
- `vault ls` / `vault status` render the tier badge next to the existing
  proven/raw verdict text.

## Scope per commit

### Commit 0 — `chore(plans): commit proven-tier arc plans`
Both untracked plan docs (`next-work-survey`, this file) committed so the
arc's contract is tracked before implementation.

### Commit 1 — `feat(proven): tier-aware emission + trust stamps`
Land the vault half FIRST (self-contained, independently testable), then
the emitter half; both land in one commit.
1. `vault.rs`: shared `EntryPayload { …, tier }` (serde default
   "checked"); `Entry`/`Manifest` embed it. `ProofStamp.attested`
   (serde default false). `trust()` from stamp FILE (legacy fallback to
   in-memory proof). `set_trust(&ProofStamp)`. Update vault self-tests +
   migrate the `cmd_vault_import` caller via the nous-stamp helper.
2. `prove.rs` (feature-gated): `pub fn subset_ok(gen, cand)
   -> Option<String>` factoring `unsupported_shape`/`contains_neg`/
   scalar-Int param+ret checks — single source shared with `lower.rs`.
3. `lower.rs`: `emit_proven_arith` (plain `arith.addi/subi/muli : i64`;
   neg = `arith.subi 0, x`); tier param threaded through `emit_fn` (new
   wrapper `emit_fn_tier`, old `emit_fn` delegates with Checked); proven
   module omits `@ontic_trap` declarations (checked keeps them). No shim
   changes (guards are invariant-driven, unchanged per tier).
4. `interp.rs`: stale tier comments cleaned (doc-only).
5. Tests (default build): proven MLIR contains no `i128`/`ontic_trap`
   (pure emission test — no z3 needed); checked emission byte-stable;
   negative — division candidate ⇒ `subset_ok` Some ⇒ checked emission.

### Commit 2 — `feat(proven): solve wiring + equivalence gate`
1. `main.rs` `cmd_solve` landing site (single site — forge shares it;
   feature-gated): after GR6 parity passes, run `proof_for`; Proven ⇒
   lower BOTH tiers, run differential parity proven-vs-checked on the same
   transparent-example row (existing `differential_parity` machinery,
   twice); any mismatch ⇒ fall back to checked emission (never a weaker
   sieve). On match: `put_proven` with `tier: "proven"` +
   `ProofStamp{attested: true, reason: "z3-unsat", details: [proof
   summary]}`. Unproven ⇒ `put` with tier "checked" (today's path).
2. Equivalence-gate test (feature-gated): hand kernel proven end-to-end
   (z3 Unsat ⇒ flag-free emission ⇒ parity vs checked object green).
3. `vault ls/status` tier badge; `.hpp` metadata block records tier.

### Commit 3 — `docs: proven tier lands`
1. README overflow table: proven row real (no longer "future").
2. AGENTS.md GR11 amendment (recorded proof = declaration word).
3. GUARDS.md GR11 line corrected (guard tiering ≠ speed tiering).
4. CHANGES.md entry (timestamped, test counts).

## Risk register

| Risk | Mitigation |
|------|-----------|
| Emitter/prove subset drift | shared `subset_ok` fn, single source |
| Proven emission diverges on unprobed rows | z3 proof covers the WHOLE invariant-satisfying domain; equivalence gate on the transparent-example row as belt-and-braces; division family stays checked |
| Old manifests without `tier` | serde `#[serde(default)]` → "checked" |
| z3 absent on dev machine | default-build gate only; feature-gated tests are manual checks; `cargo check --features proven` where toolchain allows |
| `trust()` regression (reads in-memory proof) | stamp-file-first with legacy fallback; test both paths |
| `.nous` import path (`set_trust` caller) | nous-stamp helper builds `attested:false` stamps; import provenance ≠ z3 attestation |

## Acceptance

- `cargo test --lib` green (default build; 177+ new tests).
- `cargo test --lib --features proven` green ONCE Z3 IS INSTALLED:
  proven positive + negative (division), equivalence gate. Until then:
  `cargo check --features proven` clean.
- `ontic prove examples/<kernel>.ont --hand ...` reports Proven;
  `ontic solve` with the feature vaults it with `tier: proven`,
  `vault status` shows the badge, MLIR artifact has no i128/trap.
- Praetor clean; CHANGES.md entry; no vault-key churn for existing gens.

## Execution notes (from pre-flight trace)

- Emission hook point: `lower.rs:1924` (`emit_binop`'s
  `matches!(Add|Sub|Mul) → emit_checked_arith`) becomes a tier dispatch.
- `emit_fn` gains the tier via a thin `emit_fn_tier` wrapper; all existing
  callers keep `emit_fn` (default Checked) — zero churn.
- Equivalence gate reuses `main.rs::differential_parity` (L689) — run it
  on the proven composite and the checked composite over the same row.
- Trap declarations: `emit_fn` (L289) emits `@ontic_trap`/
  `@ontic_trapf` unconditionally; gate them on tier != Proven.
- Vault landing: `main.rs:1147` (`v.put`) is the only site; meta.json
  write follows it unchanged.
