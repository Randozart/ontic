# map — List Transform Construct

**Date:** 2026-08-23 (late)
**Author:** Randy Smits-Schreuder Goedheijt
**Status:** Approved
**Depends on:** PR0, P1/P2 float+broadcast layers.

## 1. Problem

Models write correct fold-map-append patterns for list-returning kernels but
three things block them: polymorphic empty lists, mixed concat promotion,
and circular fold accumulator inference. Root cause: fold produces scalars;
there is no construct that produces lists.

## 2. Solution

Add `map` as a second iteration construct alongside `fold`:

```
map(%v in %pts) { %v * %s + %off }
```

| | fold | map |
|--|------|-----|
| Input | List\<T\> | List\<T\> |
| Output | U (scalar) | List\<U\> (same length) |
| Accumulator | Yes (circular inference risk) | No |
| Use case | Reductions | Transforms |

## 3. Work breakdown

| # | Item | Gate |
|---|------|------|
| M1-M2 | Grammar + AST + GBNF mirror | parses |
| M3 | Typechecker: list must be List<T>, body infers elem type U, returns List<U> | unit tests |
| M4 | Interpreter: iterate elements, eval body per elem, collect into FloatList/List | eval tests |
| M5 | Lowering: alloc same-dim memref, scf.for loop, eval+store per iteration; element type from body's expr_ty | mlir-opt validation |
| M6 | Consumer arms: display/overfit/sieve/genrand/expr_ty | compile clean |
| MG | Live gate: forge-solve transform.ont without --hand | survivors vaulted |

## 4. Design decisions

- Syntax mirrors fold exactly: `map(%v in %list) { body }`.
- Body type = result element type (no accumulator to constrain).
- Empty input → empty output (no special case).
- Int elements widen via sitofp when body produces F64.
