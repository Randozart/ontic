# Ontic — Intermediate Report #2

**Date:** 2026-08-23 (evening session)
**Covers:** Post-first-report through current state
**Commits:** `7f110c3` → HEAD (~20 additional commits)
**Tests:** 126 behavioral, all green
**Codebase:** ~10k lines Rust, 2 external deps

---

## 1. What happened this stretch

Three major arcs landed since the first intermediate report:

### Arc 1 — Cloud samplers + provenance
Implemented three sampler backends behind one dispatch: local llama.cpp,
OpenAI-compatible chat completions, and native Gemini with responseSchema
structured output. API keys managed via `.env` with 0600-permission header
files consumed by curl (never argv). Every solve records full prompt
provenance in its vault manifest.

**Live gate passed**: Gemini flash-lite solved Ledger.total from spec alone,
vaulted under the same canonical key as the hand-solved version — three
independent samplers converging on identical verified contracts.

### Arc 2 — Vocabulary evolution + ablation control
Project-wide rename wish→gen. Added `ontic key` subcommand as sole key
authority. Implemented type-directed random enumeration sampler and the
`ontic ablate` control experiment.

**Results**: uniform 0/8 vs Gemini 8/8 on ledger; uniform 0/6 vs Gemini 5/6
on rms. The transformer earns its slot decisively on both trivial and hard
tasks; type-directed enumeration never produces correct semantics.

### Arc 3 — Expression lists + D-track opening
Added expression-list literals (`[%a * %b, %c + %d]`) through all layers.
Opened the paper-RE track: transform kernel solved+vaulted, coords.txt→PLY
pipeline demonstrated from Python, dot kernel vaulted with guarded total
semantics. Identified two language gaps (list construction, constraint-aware
probing) documented in ISSUES.md.

## 2. Bugs found by live candidates (all fixed)

| Bug | Found by | Fix |
|-----|----------|-----|
| Pure-float comparisons hit int_of | rms candidates | Float path for ALL binops when any operand is F64 |
| Int==F64 rejected by checker | transform candidates | Eq arm widened for numeric promotion |
| arith.cmpf predicate names | mixed-compare candidate | oeq/une/olt/ole/ogt/oge for float path |
| Schema params carried % sigil | Gemini candidates | clean_ident() normalization |
| --samples/--seed dropped | cloud solve run | forge_config plumbing restored |
| Range returned loop counter not alloc | dot emission | Return alloc instead of acc |
| parse_concat infinite recursion | test suite | Reverted WIP entirely |

## 3. Honest gaps remaining

| Gap | Impact | Queued fix |
|-----|--------|-----------|
| Forge can't solve list-return kernels | Data-transform gens need hand candidates | Empty typed lists + cons/append builtins |
| List-return ctypes segfaults | Blocks Python consumption of list-return kernels | ctypes Structure restype (sret-aware) |
| MatVec not expressible | Needs expression-list construction from computed scalars | Same as above |
| Probe generator ignores input constraints | Mismatched-length probes kill valid kernels | Constraint-aware probing (M3) |
| No formal proofs | "Verified" = evidence + probes until M3 lands | prop laws + Z3 |

## 4. Architecture validation summary

Every claim now has quantitative evidence:

| Claim | Evidence |
|-------|----------|
| Trust scales with verifier strength | All capability gains came from sieve/emitter work while model stayed frozen |
| Composition beats model size | rms closed via vault composition when direct synthesis failed |
| Transformer earns its slot | Ablation: 0% survival for enumeration vs 83–100% for LLM |
| Differential gates catch real bugs | Five bugs found pre-ship across sessions |
| Honesty infrastructure works | Wrong human evidence rejected same as wrong code; capability boundaries recorded not hidden |

## 5. What's next

| Priority | Item | Dependency |
|----------|------|-----------|
| 1 | List construction builtins (cons/append) + typed empty lists | None — unlocks forge-solvable list-return kernels |
| 2 | ctypes sret binding for list-return kernels | None — unlocks Python consumption |
| 3 | D1 trusted PLY writer → D2 full gate without C shim | Depends on 1–2 for full automation |
| 4 | M3 prop laws + Z3 proven tier | Independent but large |
| 5 | Optimizer passes (vectorize, constant-shape specialization) | Independent |
