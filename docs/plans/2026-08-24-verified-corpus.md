# Verified-Corpus Collection Stage (LoRA feedstock)

**Date:** 2026-08-24
**Author:** Randy Smits-Schreuder Goedheijt + session agent
**Status:** Approved (session of 2026-08-24)
**Depends on:** sieve reports with structured rejections; decompose pipeline;
provenance-in-manifest (cloud-samplers arc).

## 1. Principle

Only sieve-approved code becomes supervision. Ontic's training corpus is
clean by construction — no human judgement about code quality enters the
loop, the deterministic gates already decided.

## 2. Standing rule (contamination guard, GR3 companion)

Every record carries `gen_key`. A sampler fine-tuned on this corpus must
NEVER be used to solve gens whose keys appear in its training data — that
would burn S4's overfit-detection guarantee for those gens. Exporters expose
`--exclude-key` so eval splits are enforceable at training time.

## 3. Work items

| # | Item | Gate |
|---|------|------|
| C1a | `src/corpus.rs`: versioned Record {kind: solve\|spec, gen_key, backend, model, prompt, winner, rejects[{text,stage,reason}]}; append-only `.ontic/corpus/train.jsonl` | serde roundtrip test |
| C1b | `Rejection` carries candidate text; `run_one` attaches it on every kill path | rejection-text survives sieve::run (test) |
| C1c | `cmd_solve` appends a record post-sieve when `ONTIC_COLLECT=1` (from `.env`) | negative test: env unset ⇒ no file |
| C1d | `cmd_decompose` appends spec-kind records incl. diffs + repair transcripts | — |
| C2a | `ontic corpus backfill`: mine vault manifests; reconstructed prompts flagged `reconstructed:true` | count == artifact-bearing entries |
| C2b | `ontic corpus stats`; `export --format chat\|dpo --exclude-key K` | export honors exclusions |
| C2c | `docs/CORPUS.md`: schema, contamination rule, recipe notes | FG-C: docs merged |

## 4. Non-goals

The LoRA run itself (external tooling/GPU), eval harness, base-model choice.
Collection is model-agnostic by design.
