# List Construction — Typed Empty Lists + Concat + pyous sret Fix

**Date:** 2026-08-23 (late)
**Author:** Randy Smits-Schreuder Goedheijt
**Status:** Approved (session of 2026-08-23)
**Depends on:** FloatListLit, broadcasting, native pipeline.

## 1. Problem

Models naturally write fold-map-append patterns for list-returning kernels:

```
fold %v in %pts, %acc from [] { %acc ++ [%v * %s + %off] }
```

Three things block this pattern:

1. `[]` defaults to List<Int>, not List<F64>
2. No `++` concatenation operator exists
3. pyous.py uses c_void_p restype for list-return kernels (segfaults)

## 2. Fix plan

| # | Item | Files | Lines |
|---|------|-------|-------|
| LC1 | Empty ListLit defaults to ListF64 (not ListInt) | check.rs | ~3 |
| LC2 | BinOp::Concat (`++`) through grammar/check/interp/lower/display | sketch/check/interp/lower/sieve/overfit | ~120 |
| CT1 | pyous.py: MemRefF64 struct as restype for list-return kernels | examples/pyous.py | ~20 |

## 3. Design decisions

- `[]` defaults to List\<F64\>: research numerics is the primary use case;
  no Int-list gen currently uses `[]` in practice.
- `++` binds at additive precedence (same tier as `+`/`-`), so
  `%acc ++ [%v]` parses correctly inside fold bodies.
- Concat lowering: alloc combined memref, scf.for copy left side,
  scf.for copy right side at offset = len(left).
- pyous sret: ctypes.Structure restype auto-handles the hidden sret
  pointer on x86-64 SysV for structs >16 bytes.

## 4. Gate

Forge-solve transform.ont WITHOUT --hand: the model's natural fold-map
pattern must survive the full sieve and vault with artifacts.
