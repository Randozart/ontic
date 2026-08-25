# Bounded Loops Milestone — `until` on fold

**Date:** 2026-08-24
**Author:** Randy Smits-Schreuder Goedheijt + session agent
**Status:** Approved (session of 2026-08-24)
**Depends on:** consolidation complete; strategy report §5.1 soundness test.

## 1. Design

Data-dependent early exit inside a declared budget, expressed as an optional
pre-test clause on the existing fold construct:

```
fold k in range(MAX), acc from INIT { STEP } until DONE_EXPR
```

- **Termination stays decidable**: iteration count ≤ MAX (range-bound),
  machine-managed.
- **Pre-test semantics**: `until` is evaluated on the current `(k, acc)`
  BEFORE each iteration; zero iterations if INIT satisfies DONE; result is
  the surviving `acc` either way (documented: never-done ⇒ full budget runs).
- **No break-statements**: avoids effect-typing of diverging branches and
  `scf.for`'s lack of break. An `until` fold lowers to `scf.while` (MLIR) /
  compound-condition `br` loop (LLVM).
- **Oracle first**: interpreter implements semantics before any lowering;
  differential tests enforce parity (Golden Rule 6).

## 2. Work items

| # | Item | Gate |
|---|------|------|
| L1 | Grammar + AST: optional `until` suffix on fold; display/canonical roundtrip | parse tests |
| L2 | Checker: `until` infers Bool under `{k: Int, acc: init_ty}` | negative tests |
| L3 | Interpreter: pre-test loop, deterministic | unit tests incl. 0-iteration + full-budget |
| L4 | Differential harness: interp vs native on until-kernels | LG1: parity incl. edges |
| L5 | MLIR lowering: `scf.while` emission when `until` present | LG2: mlir-opt validates |
| L6 | Direct-LLVM lowering: compound `br` loop | covered by L4 |
| L7 | Forge proof: iterative-solver kernel (e.g. Newton sqrt / fixed-point) solved by gemini from spec alone | LG3: vaulted + Python-verified |
| L8 | Docs: ask_langref.txt teaches the clause; CHANGES + report | FG-L |

## 3. Non-goals

General `break` anywhere in expressions, while-loops without budgets,
recursion — permanent membrane residents per the soundness test.
