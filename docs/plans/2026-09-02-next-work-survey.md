# Ontic — Next Work Survey (2026-09-02)

**Date:** 2026-09-02
**Timestamp:** 2026-09-02 22:05

Post-admin survey of what work remains, ranked by value-to-effort. Baseline:
`3bdf50b` + admin commit `1835a7f`; 177/177 tests green.

## Findings

### F1 — Proven tier: proofs exist, the fast path doesn't (THE gap)

`3085c8e` (2026-08-26) delivered `src/prove.rs`: z3 overflow-absence proofs,
feature-gated (`proven = ["dep:z3"]`), with an honest subset boundary
(straight-line Int arithmetic; division family EXCLUDED by design — SMT
div/mod is Euclidean, oracle truncates; restricted coverage only weakens
toward conservative answers).

But the tier stops at annotation:

- `cmd_prove` only PRINTS "flag-free codegen eligible (GR11)" — it emits
  nothing. `src/lower.rs` has exactly one code path: `emit_checked_arith`
  (i128 widen → range check → trap). There is no proven-tier emitter.
- `interp.rs` has no tier: `Ctx { deps }` only; the doc-comment at L265
  ("the DEP'S OWN tier") and `Ctx::checked()` naming are stale relics from
  when the wrapping tier was removed (`07b67d6`).
- `README.md` L168 still says "A **future** `proven` tier plans Z3
  absence proofs" — it already exists.
- `AGENTS.md` GR11 still says fast paths require a "`wrapping`" word in the
  gen — `wrapping` no longer parses (removed 2026-08-25). Stale contract
  text.
- The production-master plan (P7) anticipated this: "Emitter: tier-aware
  arith selection: proven bodies skip widen-check-trap" + "Equivalence
  gate: every proven emission differentially tested vs checked-tier object
  on probe rows before acceptance" + "manifest stamps `"tier": "proven"`".
  None of that landed; P7 as written only covered the z3 encoding module.

This is the natural next leg: the proof machinery is in, the payoff
(flag-free code) is not.

### F2 — Attestation wiring (small, follows from F1)

- `ProofStamp { reason, details }` — `vault.trust()` still maps via
  `reason.contains("checked")` (the fragile string matching flagged in the
  09-01 findings). Fix: `attested: bool` field.
- `set_trust(key, verdict: &str)` takes a STRING verdict
  ("proven"/"attested"/"verified") and reconstructs the reason — same
  string-matching disease. Should take a `ProofStamp`.
- `put_proven` has zero callers. Forge (`cmd_forge`) and import
  (`cmd_import`, main.rs:1685) land via unattested/`set_trust` paths.
  NOUS.md documents imports landing **attested** — the plumbing to make
  that verdict come from a real stamp doesn't exist yet.
- Note: "attested" in NOUS.md (import provenance) and "attested" in
  `ProvenVerdict` (proof-verified) are two different concepts wearing one
  word. The 08-31 completion plan had `Proven/Guarded/Raw` variants; the
  landed enum collapsed to `Attested/Unattested`, which muddies that.

### F3 — Str native parity (honest boundary, medium) — DONE 2026-09-03

Str v1 (`ee0a7d0`) was fail-closed at S2: "opaque FFI ABI pending".
Now implemented per `2026-09-03-str-native-abi.md` (`c7b3fe0`…`a320e75`):
ABI is `(char* data, long len)` (len authoritative, NUL-safe), return is
an `S` struct. `str_len`/`str_eq` lower to MLIR; S2 unblocked; native
driver + byte-exact parity + probes + header/shim all in. 194/194 green.
(Original scope, for the record:)
- `lower.rs`: `(char* data, long len)` per Str param (plan P6 wording),
  `str_len` = len-field load, `str_eq` = length compare + byte loop,
  header/shim/hpp emission.
- `pipeline.rs`: driver arms for string args + `const char*` returns
  printed bounded.
- `main.rs` parity: strings compare exactly.
- Unblocks Str kernels from being vaulted with native artifacts —
  currently they can only solve in interpret mode.
- `lower.rs`: `(char* data, long len)` per Str param (plan P6 wording),
  `str_len` = len-field load, `str_eq` = length compare + byte loop,
  header/shim/hpp emission.
- `pipeline.rs`: driver arms for string args + `const char*` returns
  printed bounded.
- `main.rs` parity: strings compare exactly.
- Unblocks Str kernels from being vaulted with native artifacts —
  currently they can only solve in interpret mode.

### F4 — Docs/contract truthfulness sweep (small)

- `README.md` L168: "future proven tier" → describe the real tier
  (annotate today; codegen after F1).
- `AGENTS.md` GR11: drop `wrapping`, state the actual declaration
  mechanism (proof verdict / future `prop` word).
- `docs/GUARDS.md` L158 maps GR11 to "raw/guarded twin artifacts" —
  conflates guard tiering with speed-tiering; needs a line.
- `interp.rs` stale tier comments (F1-adjacent, cheap to fix in the F1
  commit or a doc-only pass).

### F5 — Candidate next capabilities (bigger, no plan yet)

- **Division family in z3**: value-faithful trunc-toward-zero modeling of
  div/mod in `prove.rs` would widen proven coverage to the largest
  unprovable class today.
- **Proposed-constant proofs**: z3 can verify "there EXISTS a constant
  making this invariant hold" — a different proof class than
  overflow-absence.
- **Guarded `.so` for Str kernels** (follows F3).
- **`.pi`/`.officina` tooling state**: no action needed; gitignored.

## Recommended plan: Proven tier completion (F1+F2+F4, one coherent arc)

Plan doc: `docs/plans/2026-09-02-proven-tier-emission.md` (to be written
before implementation). Outline:

1. **Declaration word (GR11 compliance).** GR11 demands the fast path
   "never exists without a visible contract word in the gen". Options:
   (a) `prop` line in gen grammar (bigger: grammar + parser + canonical
   text = vault key churn for existing gens); (b) treat the z3 PROVEN
   verdict itself as the declaration — recorded in the ProofStamp, and the
   emitter refuses to emit flag-free code absent a recorded proof.
   Recommend (b): the machine's proof IS the contract word; no key churn;
   GR11's intent ("no mercy without evidence") is satisfied because
   emission is gated on the stamp, not on author claim. Amend GR11 text
   to say so.
2. **`lower.rs` proven emitter.** `emit_proven_arith`: plain
   `arith.addi/subi/muli : i64`, no i128/trap. Selection per candidate:
   only when the WHOLE body is within the proven subset AND a Proven
   verdict is recorded (same subset check as `prove.rs` — factor the
   `unsupported_shape`/`count_arith` logic into a shared fn so lower and
   prove cannot drift).
3. **Manifest tier stamp.** `Entry` gains `tier: "checked" | "proven"`
   (default checked). `solve`/`forge` under `--features proven`: run
   `proof_for` on the survivor; if Proven, lower with proven arith and
   stamp the manifest + `ProofStamp{attested:true, reason, details:[z3
   encoding summary]}` via `put_proven`.
4. **Equivalence gate (P7 acceptance, non-negotiable).** Every proven
   emission differentially tested against the checked-tier object on the
   same probe rows before acceptance — proven code that diverges on any
   row kills the candidate back to checked. (Interpreter is the oracle,
   GR6: both must match interp.)
5. **`vault.trust()` from `ProofStamp.attested`** (kill the
   `reason.contains` matching); `set_trust` takes a `ProofStamp`.
6. **Tests.** Negative: division candidate ⇒ Unproven ⇒ checked emission
   (no fabricated proven). Positive: straight-line Int kernel ⇒ Proven ⇒
   flag-free MLIR (assert no i128/trap) + differential parity vs checked.
   GRAMMAR untouched (option b).
7. **Docs.** README proven-tier row (real, not "future"); AGENTS.md GR11
   amendment; GUARDS.md line; CHANGES.md entry.

Effort estimate: L (the lower.rs arith path is well-factored around
`emit_checked_arith`; the equivalence gate is the main new machinery).
Suggest ~2–3 commits: (1) tier stamp + proven emitter + shared subset
check; (2) forge/solve wiring + equivalence gate + `put_proven` wiring;
(3) docs.

### Sequencing note

F3 (Str native ABI) is independent and parallelizable; do it after the
proven tier or in a separate session. F5 items are plan-doc candidates,
not this arc.
