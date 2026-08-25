# Self-Distillation Readiness Stage

**Date:** 2026-08-24
**Author:** Randy Smits-Schreuder Goedheijt + session agent
**Status:** Approved (session of 2026-08-24)
**Depends on:** verified-corpus collection; bounded loops; flywheel ledger.

## 1. Thesis

A future LoRA on the verified corpus either raises solve-rate at equal
verifiability or it does not — and Ontic must be able to say which, with
numbers. This stage builds the measurement before the training happens.

## 2. Work items

| # | Item | Gate |
|---|------|------|
| D1a | Eval suite: fresh never-solved specs across tiers (scalar / reduction / transform / until-solver / composed-over-vault-deps) in `examples/eval_suite/` | every spec passes `ontic check` |
| D1b | `ontic eval --suite DIR [--sampler-backend B] [--samples N] [--tag NAME] [--trained-on FILE]`: pass@N + ns/call per gen, persisted `.ontic/eval/<tag>.json`; contamination guard skips corpus keys | DG1: baseline-gemini recorded |
| D2a | `ontic sweep <topics-file> [--limit N]`: topic→spec→solve loop through all gates; canonical-key dedup against corpus | DG2: ≥150 total records, model-authored majority |
| D2b | Topics file ~55 diverse requests (stats/linalg/signal/activations/solvers) | topics reviewed, wide coverage |
| D3 | Rich 3DGS rerun: section-6 display equations verbatim into corpus text; decompose targeting projection/cov2d chain | DG3: result published either way |
| D4 | Handoff package: CORPUS.md recipe finalized (splits, unsloth-class config sketch, eval protocol before/after tags) | DG4: third-party trainable |

## 3. Non-goals

The LoRA training itself; optimizer passes; contracted `.hpp`; tuples.

## 4. Risks carried

Suite authored by the agent = agent biases (noted in docs). Decomposition
depth may remain conservative on D3 — that evidence promotes shape/tuple
work. Sweep diversity bounded by topic-list quality.
