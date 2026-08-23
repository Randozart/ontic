# Ontic — Agent Guidelines

**2026-08-22:** Initial operating manual. Derived from the briev-lang
(brief-compiler-dogfood) operating contract; adapted to Ontic's division of
labour between deterministic verification and stochastic generation.

## Operating Contract

You are building a **stochastic specification compiler**: users write
specification (`.ont`), a local transformer proposes implementations, and a
fully deterministic sieve decides what is true. Zero tolerance for "probably
fine" — the sieve IS the product. A bug in the sieve means unverifiable code
ships as "verified".

Every decision passes three questions:

1. **Does this strengthen the sieve or weaken it?** Convenience that weakens
   a check (looser probe domains, skipped stages, larger tolerances) is a
   contract violation, not a tradeoff.
2. **Is this deterministic?** Anything that decides acceptance must be
   reproducible byte-for-byte given the same inputs and seed. The LLM sits
   behind exactly one wall: candidate *generation*. It never validates,
   never ranks, never decides.
3. **Does this keep Ontic self-contained?** New dependencies need
   justification against hand-rolling. HTTP, RNG, SHA-256 are hand-rolled;
   serde/serde_json are the only allowed external crates.

Patches are unacceptable. There is no "go fast and break things."

## Golden Rules

1. **THE WALL**: the transformer generates candidates; everything else —
   parsing, typechecking, evaluation, probing, overfit detection, ranking,
   emission — is deterministic Rust. Code that lets model output influence a
   verdict outside S1 (parse) is a critical failure.
2. **CONTRACT-FIRST**: invariants (`|`) are the source of truth. Never edit
   an invariant to make a candidate pass. If all candidates fail, the answer
   is more/better candidates or a proven-unsatisfiable report — never a
   weakened wish.
3. **OPAQUE STAYS OPAQUE**: held-out examples (`??`) and auto-hidden examples
   must never reach a forge prompt, log line, cache key comment, or test
   fixture visible to sampling. Leakage invalidates the overfit guarantee.
4. **PROBES ARE PART OF THE CONTRACT**: probe strength derives from type
   domains ∩ invariants. When improving probes, widen honestly (better
   domain coverage), never tune until a specific candidate passes.
5. **OVERFIT REJECTION IS BEHAVIORAL + STRUCTURAL**: S4 (held-out) and S5
   (probes) are behavioral; S6 (shape scan) is structural. Neither alone is
   sufficient. Never demote one to advisory.
6. **INTERPRETER IS THE ORACLE**: if interpreter and lowerer disagree, the
   interpreter wins. Fix the MLIR lowering, never the interpreter, to make
   emitted IR match observed behaviour.
7. **ADDITIVE ONLY**: existing sieve stages keep their order and semantics.
   New checks append; `_ =>` fallthroughs stay unchanged.
8. **ALWAYS FINISH**: no `todo!()`, no stubs, no deferred edge cases in
   committed code.
9. **TESTS OR IT DOESN'T EXIST**: every stage needs behavioral tests,
   including negative tests (a known-overfit candidate MUST be rejected).
10. **SELF-CONTAINED**: no network calls except to CONFIGURED forge
    endpoints (local llama-server or cloud sampler per `.env`/CLI). No
    telemetry. Vault is plain files. API keys live only in 0600 temp header
    files consumed by curl and are never logged, printed, or committed.
    NOTE: cloud-sampled candidate sets are not reproducible across runs;
    verdicts and vault keys remain deterministic. Prompt provenance is
    stored per solve so runs stay auditable.
11. **SPEED REQUIRES DECLARATION**: the fast path never exists without a
    visible contract word in the wish (`wrapping`, future `prop` proofs).
    Compiler mercy is forbidden: semantics must be identical between the
    oracle interpreter and emitted native code, tier for tier.
12. **HINTS ARE ADVICE, NEVER EVIDENCE**: `hint` lines shape forge prompts
    only. They never touch canonical text, vault keys, sieve verdicts, or
    any acceptance decision.
13. **EFFECTS LIVE IN RECIPES, NEVER WISHES**: file/console IO compiles to
    deterministic driver C at the recipe layer. Wishes stay pure so probes,
    composition, and native parity remain sound.
14. **FORMATS ARE TRUSTED WRITERS**: parse/emit machinery (CSV/PLY/OBJ/text)
    is verified-once stdlib; synthesis owns the transform BETWEEN them.
    File evidence desugars to values at parse time — the sieve never
    touches disk.
15. **KERNELS ARE ARTIFACTS**: humans edit wishes and recipes; vault entries
    are immutable build outputs. Never hand-edit generated MLIR/objects/
    headers — change the wish and re-solve instead.

## Architecture Pillars

- **One grammar, two consumers.** The GBNF string (`sketch::GRAMMAR`)
  constrains server-side sampling; the Rust parser mirrors it exactly. Any
  grammar change updates both in the same commit, with a parse-parity test.
- **The interpreter defines semantics.** Sketch AST semantics are whatever
  `interp.rs` computes. Lowering to MLIR is a projection of those semantics,
  checked by differential tests (random programs interpreted AND executed via
  mlir-cpu-runner when available).
- **Vault entries are contracts, not code trust.** Callers of a vault symbol
  rely on its `[pre][post]`, never on its body. Bodies are never re-verified;
  wishes are re-solved when their canonical text changes (SHA-256 key).
- **Machine-readable failures.** Every sieve kill produces a structured
  reason (`SieveRejection { stage, detail }`) consumed by the forge feedback
  round and by `ontic check`. Human prose is a rendering, not the format.

## Sieve Rules

| Stage | Owner | Rule |
|-------|-------|------|
| S1 parse | `sieve` | malformed ⇒ kill, reason includes offset |
| S2 well-formed | `check` | type errors ⇒ kill |
| S3 transparent | `interp` | any visible example fails ⇒ kill |
| S4 held-out | `interp` | any opaque fail ⇒ kill, tagged OVERFIT |
| S5 probes | `probes` | seeded, reproducible; violation ⇒ kill, counterexample recorded |
| S6 shape | `overfit` | threshold rejections must cite the metric value |
| S7 bench | `interp` | timing only among survivors; ties broken by smaller AST |

Thresholds (guard ratio, table-shape limits, probe counts) live in one config
struct, defaulted once, overridable per CLI flag — never scattered literals.

## Forge Rules

- One TCP connection per worker, kept alive across samples.
- Grammar parameter always set; raw unconstrained sampling is forbidden.
- Prompt contains: signature, invariants, transparent examples, vault dep
  signatures. Nothing else. No chain-of-thought solicitation.
- Exactly one feedback round: distilled rejection reasons appended, K
  resamples. No open-ended retry loops.
- Endpoint/port from CLI/env (VITRIOL default 8279). Missing server is a
  clean error, never a hang.

## Working Rules

- **Flat control flow** — max 2 nesting levels; guard clauses; named helpers.
- **Intent comments** before every function (what + why, one line minimum).
- **Params ≤ 6** — bundle into context structs (`Wish`, `SiegeConfig`,
  `ForgeRequest`).
- **Complexity ≤ 15** cyclomatic/cognitive; split stages rather than branch.
- **HashMap iteration determinism** — sort keys before anything that reaches
  output bytes (vault manifests, emitted MLIR).
- **Continuous commits** after each logical step with green tests.
- **Timestamped records**: CHANGES.md entry + ISSUES.md post-mortem for every
  failed approach. Format:
  `**Date:** YYYY-MM-DD` / `**Timestamp:** YYYY-MM-DD HH:MM`.
- **Plans before plan-driven work**: `docs/plans/YYYY-MM-DD-<topic>.md`.

## Commands

```bash
cargo build                 # build
cargo test --lib            # full behavioral suite (pre-commit gate)
cargo run -- check examples/ledger.ont    # validate a wish, report probe strength
cargo run -- solve examples/ledger.ont    # sieve pipeline (hand candidates / forge)
cargo run -- bench examples/ledger.ont    # rank survivors with timings
cargo run -- run examples/demo.ont        # execute a recipe over the vault
cargo run -- vault ls                     # list verified functions
```

Forge flags: `--forge host:port --samples K --seed N` (defaults: env
`ONTIC_FORGE`, 32, 0x5EED).

## Reference Index

| Resource | Location |
|----------|----------|
| MVP design + rationale | `docs/plans/2026-08-22-ontic-mvp.md` |
| Spec format + examples | `examples/*.ont`, README.md |
| Sieve thresholds | `src/sieve.rs` (`SiegeConfig`) |
| Grammar (GBNF + parser) | `src/sketch.rs` |
| Change log | `CHANGES.md` |
| Failed approaches | `ISSUES.md` |

## For OpenCode

1. Read this file and the MVP plan before changes.
2. Respect THE WALL (Golden Rule 1) above all else.
3. `cargo test --lib` green before every commit.
4. Log bugs + root causes in BUGS.md (create on first entry).
5. Praetor-clean code on changed files.
