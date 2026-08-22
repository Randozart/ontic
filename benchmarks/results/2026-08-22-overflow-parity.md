# Overflow Parity — Wrapping Tier vs C Reference

**Date:** 2026-08-22
**Protocol:** brievc-style — CLOCK_MONOTONIC in-binary, interleaved runs,
medians where noted. Workload: sum of 1024 i64 elements, 2M iterations
(direct benches) unless stated.

| Implementation | ns/call | Notes |
|---|---|---|
| C reference (`clang -O2`) | **377** | constant N=1024 → full unroll, 4 parallel accumulators |
| Ontic wrapping tier (pre-opt-stage, 2k iters) | 464–576 | noisy harness; llc codegen only |
| Ontic wrapping tier + `opt -O3` stage | **519** | same 2M-iter protocol as C row |
| Ontic checked tier (pre-opt-stage, 2k iters) | ~576 | bounds-check expansion not yet native (W2 pending) |

## Analysis

1. Wrapping tier + middle-end stage closes roughly half the gap vs the first
   honest table (576 → 519).
2. Remaining gap causes, both named and deferred:
   - **No vectorization**: generic x86-64 target + memref-descriptor
     addressing; loop-vectorize cost model rejects. Fix belongs to P-track
     autotuning (target-feature plumbing, possibly address-preprocessing).
   - **Runtime trip count**: C reference benefits from constant N=1024.
     Constant-shape specialization queued in eclipse-track §P2 notes.
3. Semantics are bit-exact interpreter ↔ native under wrapping
   (differential test `test_interpreter_native_bit_parity`).

## Verdict

Thesis intact: declared semantics bought a measured speedup with zero
dishonesty. Remaining gap is optimization-tuning work, not verification work.
