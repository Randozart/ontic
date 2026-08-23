# Ontic — Intermediate Report

**Date:** 2026-08-23
**Author:** Randy Smits-Schreuder Goedheijt
**Scope:** Sessions of 2026-08-22 → 2026-08-23 (23 commits, `7029fcb` → `ee54e67`)
**Status:** M1 complete; P1/P2 core complete; M2 core complete; D-track planned

---

## 1. Executive summary

Ontic is a **stochastic specification compiler**: the user writes a wish
(`.ont`) — signatures, invariants, evidence — a local transformer proposes
implementations, and a fully deterministic seven-stage sieve decides what is
true. Verified winners are emitted as MLIR, compiled to native objects,
benchmarked against C baselines, and cached in a content-addressed vault from
which larger programs compose.

In two working sessions Ontic went from empty directory to:

- **Solve-from-spec demonstrated live**: forge-only candidates converged on
  the identical vault key as an independently hand-solved solution.
- **Composition over synthesis**: a benchmark no available model could solve
  directly (`meansqdev`, requiring two dependent passes) closed via vault
  composition — decomposition beat model size.
- **Native performance pipeline with honest measurement**: every survivor is
  compiled and timed; parity tables record gaps with root causes, not excuses.

The load-bearing architectural claim — *trust scales with verifier strength,
not model strength* — has been reinforced by every capability gain: all of
them came from sieve/emitter/protocol work while the model stayed frozen.

## 2. Architecture as built

```
.ont wish ──► forge (Mellum2/Qwen via llama-server, GBNF-constrained,
                 prompt prefill) ──► candidate sketches
                        │                    │
                        │                    ▼
                        │            ┌── SIEVE S1–S7 ──────────────┐
                        │            │ parse · typecheck · visible  │
                        │            │ evidence · held-out evidence │
                        │            │ probes · overfit shape scan  │
                        │            │ bench                        │
                        │            └──────────────────────────────┘
                        │                    │ survivors only
                        │                    ▼
                        │        MLIR text → mlir-opt validate (mandatory)
                        │              → opt -O3 → llc → native objects
                        │              → differential interp↔native gates
                        ▼                    │
                   vault ◄───────────────────┘
          (SHA-256 of canonical spec)
                        │
                        ▼
             recipes: linear programs over verified parts
             (+ effects layer: write/dump/log driver glue)
```

**THE WALL (Golden Rule 1):** model output enters only as candidate *text*.
Everything past S1 — typechecking, evaluation, probing, overfit detection,
ranking, emission — is deterministic Rust. The model never validates, ranks,
or decides.

## 3. Milestones achieved

| Milestone | Commit | Evidence |
|---|---|---|
| M0 scaffold | `7029fcb` | Full sieve spine, hand-candidate gate: lookup-table rejected at S4 |
| Prefill forge strategy | `d9b9984` | Prose made unreachable; grammar starts inside function name |
| Step A: native pipeline | `9173a94` | mlir-opt validation mandatory; survivors timed as native objects |
| Overflow semantics | `fe6a8a5` | Three tiers: wrapping (declared+fast), checked (default+honest), proven (M3 queued); differential bit-parity + trap-parity gates |
| Recipes | `e843a74` | Linear programs execute natively over vaulted functions |
| Syntax highlighter | `caec1f1` | VS Code extension for `.ont`/`.sketch` |
| **M1: solve-from-spec** | `7c26e53` | Live convergence onto same vault key as hand solution |
| **P1 Layer A/B** | `2b5cb73`/`51ab33c` | F64 scalars + `List<F64>`, tolerance contracts (`± tol`), IEEE oracle semantics, f64 native parity |
| Numeric promotion | `2d0286e` | Int→F64 widening across checker/oracle/emitter (`sitofp`) |
| **P2 broadcasting + builtins** | `b2d6117`→`314ec1f` | `%xs * 3.0 + 1.0` lowers to guarded elementwise loops returning memref descriptors; `sum/max/min/sqrt/exp/log/abs`; broadcast native parity gate |
| Hints + effects | `83c0926`/`ee54e67` | Quarantined guidance channel; deterministic recipe IO |

Current test suite: **103 behavioral tests**, including negative gates
(overfit rejection at S4/S6, invariant-violation kills at S5, trap-parity,
bit-parity, broadcast parity). ~8.1k lines of dependency-light Rust
(serde/serde_json only; HTTP, RNG, SHA-256 hand-rolled).

## 4. Measured results

### 4.1 Solve-from-spec convergence (M1)

Ledger.total solved by Qwen3.8-27B under GBNF constraints: 8 samples,
5 deterministic kills, 1 survivor — vaulted under the **same SHA-256 key**
as the earlier hand-solved variant. Independent stochastic search converging
on the same verified contract is the thesis in miniature.

### 4.2 Harness lift on a weak model (prompt evolution)

Same model, same task class, across forge iterations:

| Stage | Valid-parse rate | Deepest stage reached |
|---|---|---|
| Initial prompt | ~0% | S1 |
| + static language rules | 25–50% | S2 semantic kills |
| + spec-notation firewall + few-shot shape demo | high | S3 near-misses (real arithmetic errors) |

All improvement came from harness engineering; the model was never changed.

### 4.3 Native performance parity (honest table)

1024-element i64 sum, median protocol:

| Implementation | ns/call |
|---|---|
| C reference (`clang -O2`, constant trip count) | **377** |
| Ontic wrapping tier + `opt -O3` | **519** |
| Ontic checked tier (bounds-check expansion) | ~576+ |

Gap root-caused into two named components: missing overflow-flag licensing
of reassociation (fixed for wrapping tier) and runtime trip counts blocking
full unroll (constant-shape specialization queued). No vectorization yet on
generic targets — optimization pass is the next measurable frontier.

### 4.4 Capability boundary found and closed architecturally

`Stats.meansqdev` (two dependent folds + empty guard): 48+ candidates across
multiple runs, **every one correctly rejected** at S3 with genuine
near-misses (sum-of-squares = 20, mean-of-squares = 10, NaN on empty).
No false accept ever occurred.

Resolution: **composition over vault symbols**. With `Stats.mean` solved and
vaulted, `meansqdev` calling it passed the full sieve, validated as a
composite module, benched natively with linked dependencies, and vaulted
(`d2b7192`). Decomposition beat model size — exactly as designed.

### 4.5 Differential gates that caught real bugs

- Fold variables absent from emitter type-env → nested float ops miswidened
- MLIR float literal `0e0` parses as op name → decimal-point formatter
- C23 reserved word `aligned` silently ate a generated struct member
- Harness timed its own buffer initialization (10× error, caught by review)
- GBNF/parser tokenization mismatch (`%name` atomicity) — killed real samples

Each is now a regression test or ISSUES.md entry.

## 5. Honest limitations

1. **Model ceiling remains binding for direct synthesis** of multi-pass
   algorithms. The answer so far is architectural (decomposition), but this
   must be re-measured per model (Mellum2-12B pending server stability).
2. **"Verified" currently means evidence + probes, not proof.** M3 (Z3
   absence proofs, `prop` laws) is required before kernel-grade claims are
   honest at full strength.
3. **Optimization delivery incomplete**: vectorization on generic targets,
   constant-shape specialization, and composed-function overhead
   (composed devsq benches at ~14µs @1024 elements vs single-pass
   equivalents) remain open.
4. **Infrastructure fragility**: shared llama-server endpoints failed twice
   mid-session; retry/backoff tolerates transient loss but batch completion
   needs stable endpoints.
5. **Forge feedback depth**: one repair round by design; harder tasks may
   need bounded multi-round CEGIS once counterexample quality improves (M3).

## 6. Roadmap state

| Track | Status | Next |
|---|---|---|
| M1 solve-from-spec | ✅ complete | — |
| M2 composition | ✅ core complete | forge dep-prompt block; `lib promote/ls`; sampler ablation |
| P1 floats | ✅ Layer A/B done | — |
| P2 broadcasting/builtins | ✅ core done | list-return bench support polish |
| H hints / E effects | ✅ landed | rms re-run when server stabilizes |
| Paper RE (3DGS V0–V3) | 📋 planned (`docs/plans/2026-08-23-paper-reverse-engineering-track.md`) | PR0 intrinsics → linalg wishes → EWA projection |
| D-track data transformation | 📋 planned (`2026-08-23-dtrack-data-transformation.md`) | trusted text/writer intrinsics → coords→PLY slice |
| M3 proofs | 📋 planned | Z3 fragment, prop laws, proven overflow tier |

## 7. Strategic assessment

The two-week-old question "does the thesis hold?" now has evidence rather
than argument: verification strength converted directly into capability
(promotion rules, tolerance contracts, composition), while the model tier
stayed constant. The remaining risks are quantified and each maps to a
queued mechanism rather than hope: model limits → decomposition; performance
gap → optimizer passes + specialization; verification depth → M3 proofs.

Ontic's near-term identity: **the weakest model able to fill constrained
holes suffices, provided specs are decomposed** — with a compounding vault
that turns every verified function into permanent infrastructure.
