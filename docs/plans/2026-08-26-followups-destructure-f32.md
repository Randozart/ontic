# Follow-ups: Tuple Destructuring + F32 Drivers

**Date:** 2026-08-26
**Status:** planned
**Origin:** overnight slate leg 5 known follow-ups (CHANGES.md 2026-08-26)

## A. Dep-call destructuring — `let (a, b) = Dep.fn(...);` (M)

Tuple kernels can vault but candidate bodies cannot consume their
components, so composition chains cannot use multi-output cores.

### Design

Additive new AST node (GR7 — existing `Let` untouched):

```
Expr::LetTup(Vec<String>, Box<Expr>, Box<Expr>)   // names, rhs, body
```

Grammar (both consumers in one commit):
```
letx2  ::= "let" ws "(" ws pid (ws "," ws pid)* ws ")" ws "=" ws e ws ";" ws e
```
Parse-parity test required.

### Per-layer changes

| Layer | Change |
|-------|--------|
| sketch.rs | parse `let (a, b) = …;`; GRAMMAR line |
| check.rs | infer LetTup: RHS must infer to `Ty::Tuple(cs)`; arity match; bind each name to its component type; body infers under extended env |
| interp.rs | eval LetTup: eval RHS → `Value::Tuple(vs)`; arity check; bind components |
| lower.rs emit_call | **bug fix**: `Ty::F32 => "f64"` in both `param_tys` and `ret_ty` — F32 dep calls mis-lower today; correct to `f32` |
| lower.rs emit_expr | LetTup arm: emit call with N results (`call @Dep.f(...) : (t1, t2)`), bind N ssas in env |
| lower.rs expr_display | display arm |
| sieve/overfit | catch-all arms if matches are exhaustive |

Scope guards:
- Destructuring only at let-position; no nested patterns.
- RHS restricted to Call expressions whose target ret is a tuple
  (checker-enforced) — general tuple-typed RHS is future work.
- Gen-level specs unchanged: gens stay single-output.

### Tests

1. Checker: arity mismatch killed at S2 with clear reason.
2. Checker: non-tuple RHS destructured → S2 kill.
3. Interp unit: components bound correctly.
4. MLIR text: emitted call carries `(t1, t2)` result types.
5. E2E gate: two-gen chain where producer returns `(List<F64>, List<F64>)`
   (QR-style), consumer destructures and vaults; native composite runs.
6. F32 dep-call regression from the bug fix (consumer calls an F32 core).

## B. F32 differential driver arms (S)

`eval_c_source` honest-rejects `CK::F32 | CK::ListF32`, so F32 kernels
have bench but no bit-parity differential evidence.

### Changes

| Piece | Change |
|-------|--------|
| pipeline.rs eval_c_source | decl/call arms for `float` scalars and `float[]` flat-memref lists; RetSpec::F32 scalar printf `%.9g` exact-roundtrip |
| pipeline.rs RetSpec | add `F32` variant |
| tests | F32 bit-parity test mirroring `test_interpreter_native_bit_parity` (skip-clean without toolchain) |

Bit-parity standard unchanged: interpreter oracle vs native must agree
exactly within the declared tolerance semantics.

## Commit legs

1. `feat(compose): tuple destructure + F32 call lowering fix` — part A
   incl. bug fix and e2e gate.
2. `feat(pipeline): F32 differential drivers` — part B.

## Out of scope

- Nested patterns / pattern matching beyond flat name tuples
- General (non-call) tuple-typed RHS
- Gen-level tuple params (still rejected)
