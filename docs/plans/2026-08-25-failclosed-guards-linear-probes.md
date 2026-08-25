# Fail-Closed Guards + Linear Probe Solver

**Date:** 2026-08-25
**Status:** executing
**Legs:** C (fail-closed guards, S) → A-hybrid (linear probe solver, M)

## Leg C — fail-closed guard shims

### Problem

`emit_shim_c` substitutes `"true"` for invariants outside the translatable
subset (`guard_pred_text` fallback), producing `if(!(true))` — a guard that
never fires. A `.guarded.so` silently missing declared preconditions breaks
the twin-artifact contract (docs/GUARDS.md): the guarded artifact's entire
purpose is runtime enforcement.

### Fix

1. `lower.rs emit_shim_c`: collect untranslated conjuncts; if any exist,
   return `Err(String)` naming each predicate via `expr_display`.
   Delete `guard_pred_text`; call `contract_text` directly.
2. `main.rs cmd_solve` guarded path: on Err, print loud warning with the
   offending predicates; raw `.so` still vaults (guarded remains non-fatal,
   raw always lands).
3. Tests: existing shim tests keep passing (translatable subset unchanged);
   new negative test — invariant referencing unsupported construct yields
   Err from `emit_shim_c`.
4. docs/GUARDS.md: document fail-closed policy.

### Acceptance

- `cargo test --lib` green including new negative test
- F32.scale re-solve still produces guarded.so (regression)
- Commit leg

## Leg A-hybrid — hand-rolled linear constraint solver for probes

### Problem

Relational invariants (`len(%m) == %n * %n`) exhaust rejection sampling:
9/23 algorithm classes degrade to EdgesOnly (≤9 rows); kmeans hits 0 rows =
Unsatisfiable. Report: docs/reports/2026-08-24-capability-boundary.md Gap 4.

### Design (zero dependencies)

New module `src/probes_solver.rs`:

- **Unknowns**: scalar Int params + one length slot per list param.
  Bool params map to {0,1}. Float params and list ELEMENTS are not solved —
  element values come from the existing sampler, filtered by
  `first_violation` afterwards.
- **Constraint extraction**: walk conjuncts of each input-side invariant.
  Supported expression grammar: Int literals, unknowns, Add/Sub/Mul where
  Mul is univariate-in-one-unknown or constant-scaled, Div by constant,
  Mod by constant, comparisons Eq/Ne/Lt/Le/Gt/Ge, And/Or of comparisons.
  Anything else (float-typed relation, multivariate product, builtin other
  than len/index-free constructs) → constraint unsupported → solver returns
  None for the row.
- **Solving**: interval propagation over i64 domains clamped to type bounds
  (INT_LO..INT_HI, lengths 0..LIST_LEN_MAX), then deterministic enumeration:
  narrow domains to fixpoint; enumerate assignments edge-first (min/max
  alternating) in sorted unknown order. No RNG. Cap enumeration attempts;
  exceed → None (honest fallback).
- **Integration** (`probes::generate`): when a random-row budget exhausts
  (today: degrade to EdgesOnly), first consult the solver. On solution,
  materialize full rows (solved scalars+lengths; list bodies sampled and
  filtered through the existing `first_violation`). Quality = Full.
  Solver failure → existing EdgesOnly behavior. ADDITIVE ONLY: no existing
  stage reordered; every emitted row still passes `first_violation`
  (interpreter oracle). THE WALL intact — solver only proposes rows.

### Tests

1. QR-style: `%n: Int, %m: List<F64> | len(%m) == %n * %n` → Full plan,
   rows include n=0 edge and several n values.
2. Cholesky-style multi-constraint: two lists with coupled lengths.
3. kmeans-lite: relational constraints across ≥3 params.
4. Negative: float-relation invariant → EdgesOnly preserved.
5. Negative: unsatisfiable linear system (`len(%a) > 5 && len(%a) < 3`)
   → Unsatisfiable still raised when no edges pass.
6. Determinism: same seed → byte-identical plan twice.

### Acceptance

- `ontic check` on a QR-style spec reports Full quality
- Full suite green; commit leg

## Out of scope (this doc)

- z3/SMT backend (revisit if nonlinear demand appears)
- Float-constraint solving
- Element-level relational constraints inside lists
