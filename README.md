# Ontic

**Stochastic specification compiler.** You write the wish (`.ont`); Mellum2
proposes implementations; a fully deterministic sieve decides what is true.
Verified winners are emitted as MLIR and cached in a content-addressed vault.

```
fn Ledger.total(%items: List<Int>) -> Int
  | %res >= -1000000000        // invariant: bounds the probe oracle
  => [1,2,3] -> 6              // transparent — forge sees this
  ?? [4,5] -> 9                // opaque — held out; overfit killer
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
ontic check examples/ledger.ont          # validate wish, report probe strength
ontic solve examples/ledger.ont --forge 127.0.0.1:8287   # forge + sieve + vault MLIR
ontic solve examples/ledger.ont --hand cand.sketch       # sieve hand candidates only
ontic bench examples/ledger.ont --hand ...               # timings only
ontic run examples/demo.ont              # execute a recipe (program block)
ontic vault                              # list verified functions
```

## Recipes

One `.ont` file may hold several wishes plus one linear `program` block:

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
wishes. Dependencies must be vaulted before `run`; unsolved wishes are named
with their fix command.

## Overflow tiers

- `wrapping` line in a wish → mod-2^64 semantics, bit-exact interpreter↔native,
  LLVM free to reassociate (the fast lane; declared, never implicit).
- Default tier is checked: overflow kills candidates in the sieve and traps
  natively — honest-slow until M3 Z3 proofs replace checks.

Environment: `ONTIC_FORGE` (host:port), `ONTIC_FORGE_WORKERS` (default 2),
`ONTIC_VAULT` (default `.ontic/vault`), `ONTIC_MLIR_BIN` (toolchain dir).

Forge requirements: any llama.cpp `/completion` endpoint; the GBNF grammar and
the `fn @` prompt prefill are sent per request. Designed against VITRIOL's
llama-server running Mellum2-12B-A2.5B.

## Editor support

VS Code syntax highlighting for `.ont`/`.sketch` lives in
`syntax-highlighter/` — see its README for install.

## Design docs

See `docs/plans/2026-08-22-ontic-mvp.md` for the full architecture, honesty
requirements ("fastest measured among survivors", never "optimal"), and the
M0–M4 milestone plan. Operating rules for agents: `AGENTS.md`.
