<p align="center">
  <img src="ontic-logo.svg" width="128" height="128" alt="Ontic logo"/>
</p>

# Ontic

**A DSL whose products are verified native libraries.**

Write a specification (`.ont`) — signatures, invariants, example evidence.
A local transformer proposes implementations. A deterministic seven-stage
sieve proves them. The output: a shared library + C header that Python,
C, C++, Rust — anything with an FFI — consumes at native speed.

```python
import pyous as po
rms = po.gen(open("examples/rms.ont").read(), tier="wrapping")
rms([2.0, 8.0])   # -> 5.830951894845301  (native speed, sieve-verified)
```

## The Wall

The transformer generates candidates. Everything else — parsing, typechecking,
evaluation, probing, overfit detection, ranking, emission — is deterministic
Rust. The model never validates, never ranks, never decides.

## Sieve (S1–S7)

| Stage | Check | Kill |
|---|---|---|
| S1 parse | grammar mirror of server-side GBNF | malformed |
| S2 well-formed | sketch typechecker | type error |
| S3 transparent | interpreter on visible examples | wrong output |
| S4 held-out | interpreter on opaque examples | **overfit** |
| S5 probes | seeded randoms ∩ type domains, checked against invariants | violation / runtime error |
| S6 shape | constant-guard ratio + table-shape scan | memorization structure |
| S7 bench | timing among survivors | ranking only |

Opaque policy: explicit `??` wins; otherwise auto-hide floor(50%) of examples
when ≥ 4 exist. Forge prompts contain only the transparent set.

## Usage

```bash
cargo build --release
ontic check examples/ledger.ont          # validate gen, report probe strength
ontic solve examples/ledger.ont --forge 127.0.0.1:8287   # forge + sieve + vault MLIR
ontic solve examples/ledger.ont --hand cand.sketch       # sieve hand candidates only
ontic bench examples/ledger.ont --hand ...               # timings only
ontic run examples/demo.ont              # execute a recipe (program block)
ontic vault                              # list verified functions
```

## Recipes

One `.ont` file may hold several gens plus one linear `program` block:

```
fn Ledger.total(%items: List<Int>) -> Int
  | %res >= -1000000000
  => [1,2,3] -> 6

fn Twice(%n: Int) -> Int
  => 21 -> 42

program Demo
  use Ledger.total
  use Twice
start
  %xs = [1,2,3]
  %r  = Ledger.total(%xs)
  print(%r)
  %n  = Twice(21)
  print(%n)
end
```

Strictly linear glue over verified parts — all computation stays in sieved
gens. Dependencies must be vaulted before `run`; unsolved gens are named
with their fix command.

## Overflow tiers

- `wrapping` line in a gen → mod-2^64 semantics, bit-exact interpreter↔native,
  LLVM free to reassociate (the fast lane; declared, never implicit).
- Default tier is checked: overflow kills candidates in the sieve and traps
  natively — honest-slow until M3 Z3 proofs replace checks.

Environment: `ONTIC_FORGE` (host:port), `ONTIC_FORGE_WORKERS` (default 2),
`ONTIC_VAULT` (default `.ontic/vault`), `ONTIC_MLIR_BIN` (toolchain dir).

Forge requirements: local llama.cpp `/completion` (GBNF grammar + `fn @`
prefill) **or** a cloud sampler:

```bash
export GEMINI_API_KEY=...        # or put it in .env (gitignored)
ontic solve examples/rms.ont \
  --sampler-backend gemini \ 
  --model gemini-2.0-flash-lite --samples 16
```

`--sampler-backend openai` speaks chat/completions (OpenRouter, Groq,
DeepSeek, vLLM...). Cloud candidate sets are non-reproducible; every solve
records full prompt provenance in its vault manifest, and each run prints a
token report.

## Editor support

VS Code syntax highlighting for `.ont`/`.sketch` lives in
`syntax-highlighter/` — see its README for install.

## Design docs

See `docs/plans/2026-08-22-ontic-mvp.md` for the full architecture, honesty
requirements ("fastest measured among survivors", never "optimal"), and the
M0–M4 milestone plan. Operating rules for agents: `AGENTS.md`.
