# The Verified Corpus

Ontic can record every solve and spec-synthesis run as a training record.
Because THE WALL means only sieve-approved code survives, the corpus is
**clean by construction**: no human judgement about code quality enters
supervision.

## Enabling collection

Set once in `.env` (or real environment):

```
ONTIC_COLLECT=1
```

Collection is off by default. When enabled, records append to
`.ontic/corpus/train.jsonl` — one JSON object per line, never rewritten.

## Record schema (v1)

| Field | Meaning |
|---|---|
| `schema` | `1` |
| `kind` | `solve` (kernel candidates) or `spec` (paper→tree) |
| `gen_key` | canonical SHA-256 of the gen, or 16-hex prompt hash for spec records |
| `backend`, `model` | sampler identity (`backfill`/`reconstructed` for mined entries) |
| `prompt` | the full forge/decompose prompt as sent |
| `winner` | best survivor's candidate text, or concatenated file blocks |
| `rejects[]` | killed candidates: `{text, stage, kind, reason}` — machine-generated critique |
| `reconstructed` | true for backfill entries whose prompt is not historical bytes |

## Contamination rule (standing)

A sampler fine-tuned on this corpus must NEVER solve gens whose `gen_key`
appears in its training data — that would burn S4's overfit detection for
those gens. Use export exclusions to enforce splits:

```bash
ontic corpus stats                                  # counts by kind/backend
ontic corpus backfill                               # mine vault (idempotent)
ontic corpus export --format chat --out sft.jsonl \\
    --exclude-key <key-prefix>[,<key-prefix>…]
ontic corpus export --format dpo  --out dpo.jsonl   # chosen/rejected pairs
```

- `chat` format: system/user/assistant messages ready for unsloth-class LoRA
  tooling.
- `dpo` format: `chosen` (vaulted winner) vs `rejected` (a killed candidate
  with its sieve reason) — preference pairs where the critique is machine
  truth.
- Backfill records are flagged `reconstructed:true`; down-weight them at
  training time if you care about exact historical prompts.

## Eval protocol (before/after a fine-tune)

```bash
# 1. Baseline with the stock sampler:
ontic eval --suite examples/eval_suite --tag baseline-gemini \
    --sampler-backend gemini --samples 6
# 2. Export splits (exclude any gens the sampler must never train on):
ontic corpus export --format chat --out sft.jsonl --exclude-key <keys>
ontic corpus export --format dpo  --out dpo.jsonl
# 3. Train externally (unsloth-class LoRA on the chat records).
# 4. Re-eval under your sampler tag and diff .ontic/eval/*.json:
ontic eval --suite examples/eval_suite --tag lora-v1 \
    --trained-on .ontic/corpus/train.jsonl
```

`--trained-on` skips contaminated gens so S4 overfit detection stays
meaningful for whatever remains. The suite spans difficulty tiers: scalar,
reductions, transforms, until-solvers, composed chains over vault deps.

## What the corpus teaches

1. **SFT pairs**: spec contract → verified implementation. This is the core
   mapping; every vault entry contributes one.
2. **Preference pairs**: rejected candidates with structured sieve reasons —
   the machine explains *why* code dies, which is precisely the signal a
   stronger proposer must learn.
3. **Spec-authoring records**: paper text → approved spec trees, including
   repair transcripts. Trains the decomposer behind the same gates.
