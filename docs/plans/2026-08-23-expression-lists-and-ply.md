# Next Stretch — Expression Lists, ctypes sret Fix, D1 PLY Writer

**Date:** 2026-08-23
**Author:** Randy Smits-Schreuder Goedheijt
**Status:** Approved (session of 2026-08-23)
**Depends on:** All prior tracks (M0–M2, K-track, PR0, cloud samplers)

## 1. The three blockers between here and 3DGS EWA projection

| Blocker | Why it matters | Effort |
|---------|---------------|--------|
| Expression-list literals `[%a * %b, %c + %d]` | Without these, no kernel can return computed multi-element results (matvec, cross product, any multi-output transform). Sketch list literals currently accept NUMBER tokens only. | ~100 lines across sketch/check/interp/lower |
| ctypes sret binding for List-return kernels | Flat-MemRef return is a 40-byte struct via hidden sret pointer on x86-64 SysV. ctypes segfaults when restype is void* because calling convention doesn't match. Proper fix: declare restype as ctypes.Structure. | ~30 lines in pyous.py |
| Trusted PLY writer as vault intrinsic | Currently the Python script writes PLY; under the two-world doctrine this should be a trusted stdlib function verified against reference fixtures. | ~80 lines + fixture tests |

## 2. Work breakdown

| # | Item | Files | Gate |
|---|------|-------|------|
| EL1 | Grammar: list literal elements accept full expressions (not just number tokens). Parser builds Expr::ListExpr(Vec<Expr>) when any element is non-literal. Type inference: all elements must share type → List<T>. | sketch.rs, check.rs | parse+typecheck unit tests |
| EL2 | Interpreter: evaluate each element expression, construct Value::List/FloatList from results. | interp.rs | eval tests: `[%a * %b, %a + %b]` with known inputs |
| EL3 | Lowering: allocate result memref, scf.for loop evaluating each element and storing. Reuses broadcast-loop pattern. | lower.rs | mlir-opt validation + native parity test |
| CT1 | pyous.py: MemRefF64 Structure as restype for list-return kernels. Read result.aligned/result.size to extract numpy array. | examples/pyous.py | live call gate: translate_scale returns correct transformed coords |
| **D1** | Trusted PLY writer as Python-side trusted function (not sketch builtin — per two-world doctrine, IO lives outside the sieve). Write reference fixture test. | examples/write_ply.py | byte-parity vs hand-crafted expected PLY |

## 3. Execution order

EL1 → EL2 → EL3 (green = expression-list kernels work natively) → CT1
(Python can call them) → D1 (PLY writer) → D2-GATE rerun (full pipeline).

## 4. After this stretch

Once expression lists + sret binding land, the following become expressible:

| Kernel | Signature | Unlocked by |
|--------|-----------|-------------|
| matvec(2×2) | `(List<F64>, List<F64>) -> List<F64>` | EL1–EL3 |
| cross_product | `(List<F64>, List<F64>) -> List<F64>` | EL1–EL3 |
| normalize | `(List<F64>) -> List<F64>` | already works (broadcast) |
| EWA projection step | toward 3DGS V0 | EL1–EL3 + linalg library |

## 5. Honesty notes

- Expression lists increase grammar complexity — the GBNF mirror must be
  updated in lockstep (one grammar, two consumers rule).
- sret binding must be tested across compilers (clang/gcc/clang++); the
  Itanium ABI is stable but implementation-defined details matter.
- The "verified" vocabulary remains evidence+probes until M3 lands.
