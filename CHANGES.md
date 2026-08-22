# CHANGES

## Changes Made on 2026-08-22

### 2026-08-22 (session 1) — M0 scaffold + M1 forge live

- Created `Projects/ontic/`; git init; author Randy Smits-Schreuder Goedheijt.
- Wrote `docs/plans/2026-08-22-ontic-mvp.md` (architecture, milestones M0–M4,
  honesty requirements) before implementation.
- Wrote `AGENTS.md` operating manual derived from briev-lang's contract;
  THE WALL rule: LLM generates only, sieve decides everything.
- Implemented full deterministic stack:
  - `wish.rs` — `.ont` parser; auto-split (hide floor(50%) when ≥4 examples,
    no explicit `??`); canonical text excludes opaque evidence by design.
  - `sketch.rs` — candidate AST, lexer (`@name`, `%var` atomic tokens),
    Pratt parser, GBNF `GRAMMAR` string mirroring it exactly.
  - `check.rs` — S2 typechecker + wish invariant validation.
  - `interp.rs` — oracle evaluator; checked arithmetic; short-circuit && ||.
  - `probes.rs` — xorshift-seeded probe generation + canonical edges.
  - `overfit.rs` — S6 static scanner (constant-guard ratio, table shape).
  - `sieve.rs` — S1–S7 pipeline with structured `Rejection{stage,kind,reason}`.
  - `lower.rs` — sketch → MLIR text emitter (arith/scf/memref) + expr printer.
  - `http.rs` — std-only HTTP/1.1 keep-alive client with bounded timeouts.
  - `forge.rs` — llama-server client; K workers; retry/backoff; prefill strategy.
  - `vault.rs` — SHA-256 content-addressed store of verified impls.
  - `main.rs` — CLI: check/solve/bench/vault.
- 69 behavioral tests green (`cargo test --lib`), including negative tests:
  lookup-table candidate killed at S4, invariant violator killed at S5,
  guard-ratio table killed at S6.
- Live end-to-end against VITRIOL llama-server (Mellum2-12B on :8287):
  candidates generated under GBNF and semantically sieved (S2/S3 kills on real
  model output observed). Full solve-to-vault gate pending server restart.
