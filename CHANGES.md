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

### 2026-08-22 21:40 — Step A: toolchain integration live
- Files: `src/pipeline.rs` (new), `src/lower.rs`, `src/main.rs`, `src/lib.rs`, both plan docs.
- A0/A1: quoted SSA names rejected by mlir-opt → bare `%name`; Ubuntu 18.1.3
  rejects custom `memref.dim` assembly entirely → generic op syntax emission.
  Validation now MANDATORY before vaulting (toolchain present ⇒ no unvalidated IR).
- A2/A4: S7 upgraded — survivors timed as NATIVE OBJECTS (mlir-opt conversion
  chain → mlir-translate → llc → clang-linked C harness, median of 9 runs,
  self-timed CLOCK_MONOTONIC). MemRef ABI verified flat-5 expansion
  (allocated, aligned, offset, size, stride).
- A4 first parity table: Ontic 576 ns/call vs C -O2 377 (1024-elem sum).
  Gap root-caused: no overflow flags on addi blocks LLVM reassociation/
  unrolling. Decision pending: nsw flags vs declared wrapping ops.
- Tool discovery: ONTIC_MLIR_BIN env > /usr/lib/llvm-18/bin > PATH.
