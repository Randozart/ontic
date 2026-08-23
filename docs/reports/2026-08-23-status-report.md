# Ontic — Status Report

**Date:** 2026-08-23
**Sessions:** 2026-08-22 → 2026-08-23 (25+ commits)
**Tests:** 126 behavioral, all green
**Codebase:** ~10k lines Rust, 2 external deps (serde + system LLVM toolchain)

---

## What Ontic is

A DSL whose products are verified native libraries. You write a
specification (`.ont`) — signatures, invariants, example evidence. A local
transformer proposes implementations. A deterministic seven-stage sieve
proves them. The output: a shared library + C header consumable from Python,
C, C++, or anything with an FFI.

## Architecture (as built)

```
.ont spec ──► forge (3 sampler backends) ──► sieve S1–S7 ──► vault (.ous/.so/.h)
                                                                    │
                                              recipes / pyous / C FFI
                                                                    │
                                                         verified output (.ply, .csv, stdout)
```

THE WALL: model output enters only as candidate text. Everything past S1 is
deterministic Rust. The model never validates, ranks, or decides.

## Milestones completed

| Track | Status | Key deliverables |
|-------|--------|-----------------|
| M0 scaffold | ✅ | Full sieve spine, hand-candidate gates |
| M1 solve-from-spec | ✅ | Forge-only convergence onto same vault key as hand solution |
| Step A native pipeline | ✅ | mlir-opt validation mandatory; native benching; opt -O3 |
| Overflow semantics | ✅ | wrapping/checked/proven tiers; bit-parity + trap-parity gates |
| Recipes | ✅ | Linear programs over verified parts; effects layer |
| Hints | ✅ | Quarantined guidance channel (rule 12) |
| P1 floats | ✅ | F64 end-to-end; tolerance contracts; native f64 parity |
| P2 broadcasting | ✅ | Elementwise ops through full stack; math dialect builtins |
| Cloud samplers | ✅ | openai/gemini backends with schema-constrained output |
| Ablation control | ✅ | uniform 0% vs LLM 83–100% survival — quantified |
| Composition | ✅ | rms closed via mean-calling devsq (decomposition beats model size) |
| K-track FFI | ✅ | Headers, shared libs, .ous pack/unpack |
| PY-GATE | ✅ | po.gen() → callable; numpy zero-copy; cache-hit <10ms |
| D2-GATE | ✅ | coords.txt → verified kernel → valid PLY |
| Vocabulary | ✅ | wish→gen project-wide; ontic key authority subcommand |

## Key numbers

| Metric | Value |
|--------|-------|
| Tests | 126 behavioral (all green) |
| Parity: wrapping vs C -O2 | 519 vs 377 ns/call (gap root-caused, fix queued) |
| Ablation: ledger | uniform 0/8 vs Gemini 8/8 |
| Ablation: rms | uniform 0/6 vs Gemini 5/6 |
| Cold genesis (cloud) | ~14s |
| Warm cache-hit | <10ms |
| Vault entries | 8+ kernels with .so + .h artifacts |

## Honest limitations

1. **Model ceiling**: multi-pass algorithms still require decomposition via
   composition. Direct synthesis of complex transforms remains hard for
   available models.
2. **"Verified" = evidence + probes**, not proof. M3 (Z3) required before
   kernel-grade claims are honest at full strength.
3. **Performance gap**: no vectorization yet on generic targets; runtime
   trip counts block full unroll.
4. **List-return from Python**: ctypes sret binding gap blocks data-transform
   kernels from being called natively from Python (fix planned).
5. **MatVec not expressible**: requires expression-list literals in the
   sketch grammar (planned next stretch).
6. **Infrastructure fragility**: shared llama-server endpoints failed twice;
   cloud samplers mitigate but add cost + non-reproducibility.

## Roadmap

| Priority | Item | Status |
|----------|------|--------|
| NEXT | Expression-list literals + ctypes sret fix + PLY writer | 📋 planned (`docs/plans/2026-08-23-expression-lists-and-ply.md`) |
| Then | D-track continuation: linalg library expansion, EWA projection toward 3DGS | 📋 planned |
| Then | M3 prop laws + Z3 proven tier | 📋 scoped |
| Then | Optimizer passes (vectorize, constant-shape) | 📋 noted |
| Polish | .ous multi-kernel bundles, pyous README, more ablation benchmarks | ongoing |

## Design decisions locked

| Decision | Ruling |
|----------|--------|
| THE WALL | Model generates only; sieve decides everything |
| Overflow semantics | Three tiers: wrapping (declared), checked (default), proven (M3) |
| Two worlds | Synthesis owns pure transforms; trusted IO lives outside sieve |
| Effects | Recipe statements only, never in wishes |
| Hints | Forge prompts only, never evidence |
| Formats | Trusted writers; synthesis owns the transform between them |
| Vocabulary | wish → gen (project-wide); gen = action verb in APIs |
