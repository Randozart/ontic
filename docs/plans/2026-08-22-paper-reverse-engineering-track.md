# Paper Reverse-Engineering Track — 3DGS First

**Date:** 2026-08-22
**Author:** Randy Smits-Schreuder Goedheijt
**Status:** Approved (session of 2026-08-23)
**Depends on:** M2 composition; P1/P2 float+broadcast layers.

## 1. Thesis

Papers are already specifications: equations → signatures + invariants;
figure/table values → transparent evidence; official repo outputs → opaque
held-out evidence. Ontic's pipeline turns transcription into verification,
and parity tables against official implementations become reproduction proof.

## 2. Stdlib doctrine

Trusted intrinsics (implemented once natively, never forge-synthesized) +
graduated vault functions form an ever-expanding standard library. Papers
assume a stdlib; Ontic grows one per paper it reproduces.

## 3. Phases

| Phase | Delivers |
|-------|----------|
| PR0 | Intrinsics: `index(l,i)` (OOB traps like div-zero), `range(n)` |
| PR1 | Linalg vault wishes: dot/matvec/matmul/transpose (flat lists + stride convention) — synthesized, then graduated |
| PR2 | Intrinsics: `sort` (trusted), seeded `rand` |
| PR3 | **V0**: 3DGS §4.1 EWA projection — world→camera transform, Jacobian J, Sigma' = J W Sigma W^T J^T, conic, 3-sigma radius. Evidence transcribed from paper figures/supplementary |
| PR4 | **V1**: single-tile alpha blending as a fold over pre-sorted splats |
| PR5 | **V2**: full-image render from frozen params; PPM IO (zero-dep); numeric parity vs official CUDA output |
| PR6 | **AD**: Enzyme LLVM plugin on our lowered IR; `ontic grad <file> --fn name`; FD-prop cross-checks analytic gradients both directions. V3: training-as-fold (fold over steps updating params), Adam step wish, toy scene |

## 4. Gradients policy

Real AD via Enzyme at the LLVM boundary (Ontic already emits LLVM IR).
FD props remain as independent cross-checks: forward wish + gradient wish are
separate specs whose consistency is *verified*, catching transcription errors
in either direction.

## 5. Out of scope until GPU tier

Density control mutation (clone/split/prune at scale), real-time rates,
multi-million splat scenes. CPU reference renders are valuable regardless —
they verify paper numerics exactly where GPU implementations are hardest to
trust.

## 6. Workflow checklist per paper

1. Transcribe equations → wishes (sig + invariants from paper statements)
2. Figure/table values → transparent examples
3. Official outputs → ?? opaque evidence
4. Forge + sieve → vault entries; graduate stable ones
5. Recipes compose the pipeline; run on real data
6. Parity table vs official implementation (brievc protocol)
