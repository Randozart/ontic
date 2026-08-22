# Python-Eclipse Track — Research Code at Vectorized Speed

**Date:** 2026-08-22
**Author:** Randy Smits-Schreuder Goedheijt
**Status:** Approved (session of 2026-08-22)
**Depends on:** `2026-08-22-step-a-mlir-toolchain-and-recipes.md`

## 1. Thesis

Python's ease is not the language — it is the arrangement: cheap orchestration
over pre-compiled kernels. Ontic attacks the arrangement itself:

> Specs in, fused verified kernels out. Linear recipes as glue.
> The vault becomes the lab's private, proven NumPy.

Structural wedge: NumPy elementwise chains allocate temporaries; fusion is
manual folklore (`numba`, `torch.compile`). Ontic lowers chains through MLIR
where fusion is automatic — one verified loop instead of N temporaries.

## 2. Honest scope lines

- NOT replacing matplotlib/pandas/notebooks. Recipes emit CSVs; plotting
  stays external.
- Full autodiff out of reach near-term; M3 `prop` system encodes
  finite-difference gradient checks early.
- "Eclipse" = years-scale. Measurable win first: fused numeric pipelines at
  vectorized speed, written as specs, verified against evidence.

## 3. Phases

| Phase | Delivers | Gate |
|-------|----------|------|
| P1 floats | `F64` end-to-end: IEEE interp (NaN-tolerant, overflow ≠ kill), float parsing (`1e-9`), inline tolerances `-> 6.28 ± 1e-9`, cmpf/addf/mulf/divf lowering | RMS-error wish solved + sieved within stated tolerance |
| P2 arrays+math | `List<F64>`; broadcasting ops (`%xs + %ys`, `%xs * 0.5`) across wish/sketch/interp/lowerer; `sum/max/min` sugar; `sqrt/exp/log/abs` builtins (each = interp+lower+differential-test triple) | dot-product, moving-average, softmax wishes survive sieve |
| W1 wedge | Signal/stats benchmark set vs NumPy single-thread: FIR/IIR taps, convolution window, running stats, z-score; brievc timing protocol; array sizes where overhead dominates | parity or win; results table committed |
| P3 ergonomics | CSV read binding (`%xs = data "f.csv" col 0`), vault-as-library layout, cold/warm UX report | spec → kernel → script → numbers demo |
| P4 laws | `prop`: commutativity/monotonicity/bounds; FD-gradient check prop; CEGIS-flavored feedback | sieve rejects example-passing candidate violating a stated law |
| P5 GPU branch | NVVM path off shared memref/affine groundwork | deferred until CPU wedge decisive |

Domain order locked this session: signal/stats → ML space (losses,
activations, metrics). ML reuses P1–P4 wholesale; exp/log/tanh arrive with
P2 builtin machinery.

Language decisions locked:

- Broadcasting ops (`%xs + %ys`), not explicit map/zip.
- Inline per-example tolerances (`± tol`), abs+rel epsilon comparison.
  Tolerance is contract: every float comparison cites its epsilon; never a
  global silent slack.

## 4. Standing honesty rules

1. Every new builtin ships interp+lower+differential tests before any wish
   uses it.
2. Benchmarks name enemy + regime (NumPy version, BLAS backend, sizes); wins
   claimed only within measured regimes.
3. Forge quality on float code is the known risk: grammar stays tight, CEGIS
   feedback teaches, Qwen3.8 (:8279) remains fallback brain.
4. Cold solve costs seconds–minutes; warm vault hits are instant. The UX
   promise is solve-once-run-forever — like numba caching, but with proofs.

## 5. First actions

1. Execute Step A/R (companion plan) to green.
2. P1 branch: floats end-to-end with tolerance contracts.
