# Expressive Folds Stage (elementwise min/max + multi-accumulator)

**Date:** 2026-08-24
**Author:** Randy + agent
**Status:** Approved (standing queue; evidence from distillation sweep failures)

## 1. Evidence base

Sweep failures clustered on: jaccard min/max elementwise folds (no
elementwise min/max primitive), carried-secondary-state series (single
accumulator insufficient). Both are small grammar/builtin additions vs the
full-tuple project; tuples remain queued for genuine multi-output pressure.

## 2. Design

| Addition | Syntax | Semantics |
|---|---|---|
| Elementwise min/max | `min_el(a, b)` / `max_el(a, b)` (builtin2) | numeric promotion Int/F64 identical to `+`; lists broadcast per existing binop rules? **v1: scalars only** |
| Multi-accumulator fold | `fold v in L, acc from I, aux from J { (STEP_A, STEP_J) }` (aux repeatable) | body is a restricted TUPLE expression yielding one component per accumulator; result = final ACC; until composes |
| Restricted tuple expr | `(e1, e2, ...)` | ONLY valid as multi-acc fold bodies (checker enforces); avoids public Ty::Tuple ripple |

Termination unchanged (list-bounded). Oracle-first order maintained.

## 3. Work items

| # | Item | Gate |
|---|------|------|
| F1 | `min_el`/`max_el`: lex, parse(builtin2), checker(promote), interp, MLIR (`arith.select` on cmp), display | unit tests |
| F2 | Multi-acc fold: AST `aux: Vec<(String, Expr)>`, parser, checker(scoped types), interp(step over vec), display roundtrip | unit tests incl. 2-aux |
| F3 | MLIR: scf.for multi `iter_args` + multi `scf.yield`; until-path scf.while likewise | differential: hand-kernel interp==native |
| F4 | Direct LLVM: reject multi-acc/until combos cleanly (existing convention) | negative test |
| F5 | Forge proof: re-sweep previously FAILED topics (jaccard_cont, cov2, zscore, ema) — expect flips | FG-F: ≥half flip to solved |
| F6 | Docs: langref + forge prompt teach both; CHANGES + report | FG-F docs |

## 4. Non-goals

Tuple types, break-statements, while-loops, direct-LLVM parity for new forms
(MLIR pipeline authoritative), optimizer.
