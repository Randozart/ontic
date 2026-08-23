# Cloud Sampler Backends + Prompt Provenance

**Date:** 2026-08-23
**Author:** Randy Smits-Schreuder Goedheijt
**Status:** Approved (session of 2026-08-23)
**Depends on:** forge (M1), vault manifests.

## 1. Motivation

Shared local llama-server endpoints failed twice mid-batch. Cloud samplers
(Gemini flash-lite class, ~fractions of a cent per solve) unblock long forge
runs and enable the model-comparison matrix (Mellum vs Qwen vs Gemini on
identical wishes).

## 2. Backends

| Kind | Transport | Constraints | Notes |
|------|-----------|-------------|-------|
| llama *(default)* | keep-alive client | GBNF + prefill | unchanged |
| openai | curl | chat/completions; no prefix-continuation guarantee | covers Gemini-compat, OpenRouter, Groq, DeepSeek, vLLM |
| gemini | curl | native generateContent + responseSchema `{name, params[{n,t}], ret, body}`; type enum kills param-type typos by construction; trailing model turn carries prefill | x-goog-api-key |

Schema constrains SHAPE only; the `body` expression stays fully stochastic
and wholly sieve-judged.

## 3. Security posture

- `.env` hand-rolled reader: KEY=VALUE, comments ignored, never overrides
  real env vars; gitignored.
- API key lives ONLY in a 0600 temp header file passed as `-H @file`
  (never argv — never visible in `ps`); deleted after use.
- Keys never logged; errors print status + response body only.
- AGENTS rule 10 amended: "no network calls except CONFIGURED forge
  endpoints".

## 4. Provenance + cost

- Vault manifest gains `last_solve`: full prompt text, sampler kind/model/
  temperature/seed-base/K, prompt SHA-256, cumulative token counts.
- Per-run stdout token report (prompt + completion across retries).
- Determinism honesty: verdicts/vault keys stay deterministic; cloud
  candidate sets are NOT reproducible — provenance records what ran.

## 5. Work breakdown

| # | Item | Test |
|---|------|------|
| C1a | `.env` reader + config precedence (CLI > env > .env > defaults) | parse/comment/no-override units |
| C1b | curl transport w/ 0600 header-file pattern | unit + error-path hygiene |
| C2a | openai backend (body builder + response/usage parse) | pure units |
| C2b | gemini backend (responseSchema body, reassembly roundtrip) | pure units |
| C2c | dispatch behind forge::sample; retry reuse; token accumulation | routing tests |
| C3a | CLI/config flags: --sampler-backend/--endpoint/--model/--api-key-env | parse units |
| C3b | provenance into vault manifest + token report | manifest roundtrip |
| C3c | docs: rule 10 amendment, README cloud section | — |
| C4 | live gates: ledger via Gemini flash-lite; rms attempt w/ hints | G-cloud |

## 6. Config surface

```
--sampler-backend llama|openai|gemini   (default llama)
--endpoint URL                          (cloud base url)
--model NAME                            (default gemini-2.0-flash-lite)
--api-key-env NAME                      (default GEMINI_API_KEY)
.env: ONTIC_API_KEY / GEMINI_API_KEY / ONTIC_MODEL / ONTIC_SAMPLER
```

Precedence: CLI flag > environment variable > .env > default.
