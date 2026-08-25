# Ontic — Research Report: Self-Distillation Readiness

**Date:** 2026-08-24
**Covers:** Distillation-readiness stage D1–D4
**Commits:** `cfb2b1e`
**Tests:** 147 behavioral, all green
**Plan reference:** `docs/plans/2026-08-24-distillation-readiness.md`

---

## 1. Gates

| Gate | Result |
|---|---|
| DG1 | ✅ `ontic eval` records pass@N + ns/call per tag; gemini baseline **11/12 (91.7%)** persisted; uniform floor 0/12 documented; offline execution proven; collection force-disabled in eval children so held-out solves never leak into training data |
| DG2 | ✅ corpus at **177 records** — 102 model-authored (43 solve + 59 spec via sweep), sweep pass-rate 54%, canonical-key dedup enforced |
| DG3 | ✅ tested honestly: richer corpus text produced a 3-node tree where **every node cites a vault core**, two of them pipeline-deposited (`blend_step → blend_term → alpha_eval`). Projection/Jacobian layers still not emitted unprompted — recorded as signal for shape/tuple work, not laundered |
| DG4 | ✅ CORPUS.md carries the full protocol: splits, exclusion flags, unsloth-class recipe sketch, before/after eval tags |

## 2. The baseline that makes training scientific

`.ontic/eval/baseline-gemini.json`: per-gen keys, pass flags, best native
timings. When a fine-tuned sampler exists, one command re-scores under a new
tag with `--trained-on` skipping contaminated gens — the flywheel's
before/after number becomes a diff of two JSON files.

The single failure is informative: `EvalSig.softmax2` drew candidates using
`append` or Int/F64-divergent if-branches across 6 samples. Difficulty-tailed
suites are supposed to have such cases.

## 3. Sweep findings

48 topics → 48 valid drafts (100% draft validity after lenient gate) → 26
solved (54%). Failures clustered on: population-statistics two-pass bodies,
jaccard min/max elementwise folds, and anything needing carried secondary
state beyond a single accumulator — consistent with the known tuple-shaped
gap. Every failure entered the corpus as preference pairs with machine
reasons attached.

One infra catch: sweep defaulted to the llama backend and failed with raw
socket errors until `--spec-backend gemini` was explicit — default-backend
surprise documented here so it isn't rediscovered.

## 4. D3 depth analysis

The decomposer now composes rather than restates: all three nodes declared
`use` of existing cores, and the chain reached two levels into
pipeline-deposited history (`blend_term` was itself born in a decompose run).
Still absent: any attempt at projection/Jacobian/cov2d despite verbatim
display equations in the corpus text. Hypothesis: those equations carry
matrix-shape threading that flat-list contracts don't naturally express
without tuples or explicit index-pack conventions — next-stage evidence.

## 5. State of the flywheel

papers → trees → vault (reuse edges: 41 pairs / 78 hits) → corpus
(177 records) → eval baselines → [external LoRA] → eval diff.
Every arrow now exists as a running command; only the LoRA itself remains
external by design.

## 6. Next queue

Tuples when multi-output pressure arrives · contracted `.hpp` · genrand
dep-calls · optimizer passes targeting until-fold + doll-allocation overhead.
