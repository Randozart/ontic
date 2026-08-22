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

### 2026-08-22 22:30 — Overflow semantics: three tiers landed (W0–W3, W1b)
- Files: `docs/plans/2026-08-22-overflow-semantics.md`, AGENTS.md rule 11,
  wish.rs (`wrapping` line + canonical), interp.rs (Ctx threading,
  wrapping_add/sub/mul/neg), sieve.rs (tier-aware eval/probes/bench),
  pipeline.rs (opt -O3 stage, eval_native differential driver), new
  examples/ledger-wrapping.ont, benchmarks/results/2026-08-22-overflow-parity.md.
- Wrapping tier: declared in wish, bit-exact interp↔native (differential test
  gates it), plain ops → LLVM reassociation legal.
- Checked default unchanged: overflow kills candidates at S3/S5.
- opt -O3 middle-end added to native pipeline (llc alone is codegen-only).
- 74 tests green.

### 2026-08-22 23:15 — Checked-tier native traps (W2 complete)
- Files: `src/lower.rs` (emit_checked_arith: i128 widen-check-narrow +
  `ontic_trap` extern), `src/pipeline.rs` (trap definition in harnesses,
  scratch_dir uniquifier for parallel tests).
- Honesty gate test: native traps exactly where the oracle kills
  (`test_checked_tier_native_trap_matches_interpreter`); clean inputs agree.
- Wrapping tier untouched (plain ops). All three tiers now live:
  wrapping = declared+fast, checked = default+honest-slow, proven = M3.

### 2026-08-22 23:50 — Recipes: linear programs over verified parts (R complete)
- Files: `src/recipe.rs` (multi-wish .ont parser + program typecheck),
  `src/program.rs` (C-driver assembly + native execution), `src/main.rs`
  (`ontic run`, multi-wish files, `--wish` selector), examples/demo.ont.
- Strictly linear glue per plan: BindLit / BindCall / print; literals allowed
  as call args; deps must be vaulted (unsolved deps name their fix command).
- E2E gate: test_recipe_end_to_end_native — hand-solve both wishes into an
  isolated vault, assemble driver, execute → ["6", "42"].
- Live demo: ontic run examples/demo.ont prints 6 / 42 from native objects.
- 82 tests green.

### 2026-08-22 23:59 — Program-block keyword: `wish` → `use`
- Files: `src/recipe.rs` (parser + fixtures), `src/program.rs` (e2e fixture),
  `examples/demo.ont`, README/AGENTS snippets.
- Dependency declarations inside program blocks now read `use Path.name`.
- "Wish" remains the vocabulary for function specs everywhere else;
  CLI flag `--wish` unchanged (multi-wish file selector, different concept).

### 2026-08-22 23:59 — VS Code syntax highlighter
- Files: `syntax-highlighter/` (package.json, language-configuration.json,
  syntaxes/ontic.tmLanguage.json, README).
- TextMate grammar for .ont/.sketch: declarations, tiers, program blocks,
  evidence arrows (=> transparent / ?? opaque), invariant pipes, %vars,
  @symbols, builtins, types. Mirrors briev-lang extension structure.

### 2026-08-22 (session 2) — M1 gate complete: solve-from-spec live
- Forge solved Ledger.total from its spec alone via Qwen3.8-27B (:8279;
  Mellum2 :8287 still down — pipeline is model-agnostic). 8 samples, 5
  deterministic S1/S2 kills, survivor mlir-opt validated and vaulted under
  the same canonical key as the earlier hand-solved version.
- forge.rs: prompt now carries the language's static rules (fold-only
  iteration, scalar-only equality, signature-typed bodies) — spec-of-language
  guidance, not task hints; kill rate dropped immediately.
- recipe.rs: pre-fn prefix lines (e.g. wrapping) attach to the following
  wish chunk instead of erroring.
