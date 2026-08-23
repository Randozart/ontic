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

### 2026-08-22 00:45 — P1 Layer A: F64 scalars end-to-end
- sketch.rs: FloatLit lexer/parser (1.5, 1e-9 forms), Ty::F64, GBNF float rule.
- wish.rs: Value::Float, Example.tol with `-> x ± tol` syntax (F64-only,
  abs+rel epsilon at sieve), parse_type F64.
- check.rs: F64 arith/ordering typing; public infer_type for lowerer.
- interp.rs: IEEE fast path — inf/NaN propagate, only integer div/mod-by-zero errors.
- sieve.rs: evidence_holds tolerance comparison; kill reasons cite ±tol verbatim.
- lower.rs: tyenv-threaded emitter — addf/subf/mulf/divf/remf/cmpf, typed
  scf.if/scf.for yields, return type from signature; floats never enter i128 traps.
- pipeline.rs: CK param kinds, f64 differential parity gate.
- probes.rs: F64 edge/random domains. forge prompt documents IEEE semantics.
- 88 tests green incl. native f64 bit-parity + no-trap-expansion guard.

### 2026-08-22 01:30 — P1 Layer B: List<F64> end-to-end
- Ty::ListF64 across sketch/wish/check/probes/lower/pipeline/program.
- Interpreter: fold binds F64 elements from FloatList; Len accepts both lists.
- Lowerer: tyenv now mutable and extended at Let/Fold bindings (fixes nested
  float ops inside folds misclassifying as Int); list_memref() picks
  memref<?xf64>; mlir_float() formatter guarantees decimal-point mantissas.
- Evidence: element-wise tolerance comparison for FloatList outputs.
- Pipeline: CK::ListF64 harness kind (double arrays); bench harness typed by CK.
- Gate: test_float_list_fold_native_parity — interpreter ≡ native through
  memref<?xf64>. 89 tests green.

### 2026-08-22 02:20 — Numeric promotion + prompt evolution
- Language decision: Int→F64 widening in mixed arith/comparisons
  (research-language convention). check.rs promotes, interp.rs IEEE path,
  lower.rs emits arith.sitofp on the Int side.
- check.rs: Len accepts List<F64>; fold binds F64 elements from List<F64>.
- forge.rs: spec-notation firewall in prompt (SPECIFICATION vs IMPLEMENTATION
  sections, explicit "do not emit | => ?? ±"), few-shot format example
  replacing abstract placeholders (small models parrot meta-syntax),
  %res prohibition, colder retry temperature (T=0.4 repair mode).
- examples/rms.ont opened as live benchmark: two-stage fold (deviations from
  mean) + empty guard. Current status: all candidates honestly killed at S3
  with genuine near-misses (sumsq=20, mean-sq=10, NaN-on-empty). OPEN target.

### 2026-08-22 03:05 — P2a: unary builtins (sum/max/min/sqrt/exp/log/abs)
- sketch.rs: Builtin op + 8 grammar keywords (unop1 GBNF rule).
- check.rs: builtin typing — reductions over lists, numeric transforms F64
  with implicit Int promotion.
- interp.rs: oracle semantics; max/min of empty lists are honest errors
  (probes expose them like div-by-zero).
- lower.rs: emit_builtin — Len via generic dim, reductions as synthesized
  folds (sentinel init: i64::MIN/MAX, ±inf), numeric transforms via math
  dialect after sitofp promotion. expr_ty extended for Builtin arms.
- 91 tests green.

### 2026-08-22 03:40 — P2b: broadcasting semantics in the oracle
- check.rs: list-op-scalar / zip-list typing; Int widens into F64 lists;
  List<Int> op F64 promotes to List<F64>.
- interp.rs: eval_broadcast — elementwise arith, scalar application,
  zip with honest length-mismatch errors.
- 95 tests green (broadcast + builtin semantics pinned).

### 2026-08-22 04:10 — P2c: broadcast lowering + list-returning functions
- lower.rs: functions return memref<?xT>; emit_broadcast — length guard
  (trap on mismatch, matching oracle zip error), result alloc, bare
  scf.for elementwise loop, sitofp widening into F64 results.
- pipeline.rs: RetSpec (I64/F64/ListF64); differential drivers print list
  descriptors (count + first 4 elems).
- C23 gotcha: struct member named `aligned` is a reserved specifier under
  clang 18 defaults — renamed to `data` (ISSUES.md entry).
- Gate: test_broadcast_native_parity (%xs*3.0+1.0 through native memref).
- 96 tests green.

### 2026-08-23 00:00 — Plans: M2 composition + paper-RE track
- docs/plans/2026-08-22-m2-composition-and-ablation.md (vault-calls, lib
  graduation, sampler ablation)
- docs/plans/2026-08-22-paper-reverse-engineering-track.md (3DGS V0-V3,
  PR0-PR6, Enzyme AD policy, stdlib doctrine)

### 2026-08-23 01:00 — M2 step 1: call syntax + wish deps (parser layer)
- sketch.rs: Expr::Call(path,args); lexer emits CallPath for dotted paths
  followed by '(' with keyword/builtin guard (collision bug found by tests);
  GBNF callx/cpath/callargs rules.
- wish.rs: `use Path` dependency lines; canonical() includes them.
- check.rs: check_with() types calls against DepSigs with numeric widening
  into F64 params; bare infer rejects calls honestly.
- Consumers carry total arms until runtime/emission wiring lands.
- Lexer lesson recorded: duplicate speculative blocks from scripted edits —
  always grep for the pattern after scripted insertion.

### 2026-08-23 01:40 — M2 step 3: vault resolution + sieve composition
- sieve.rs: run/run_one take DepMap; S2 uses check_with when deps resolved;
  ictx carries the dep table.
- vault.rs: manifests store `wrapping` + `canonical`; find_by_path resolves
  deps without their source files.
- main.rs: resolve_deps() builds the flat closure from vault before sieving;
  both forge and retry rounds thread it.
- G3 CORE GATE: composed candidate calling Stats.mean survives the full
  sieve (test_vault_call_composition_survives_sieve); undeclared calls die
  at S2 with actionable reasons.
- 98 tests green.

### 2026-08-23 02:10 — M2 step F: composite emission + native linking
- lower.rs: emit_fn takes CallMap; emit_call emits func.call with widened
  numeric args and param-type suffixes; compose_modules merges modules;
  ontic_trap declared always (broadcast guard uses it in every tier);
  expr_ty types builtin reductions by element type.
- main.rs: ResolvedDeps{map,mlirs,calls}; native bench compiles ONE
  composite module (deps+candidate); validation gate uses composite too.
- E2E GATE: devsq-composed.ont solved, validated, native-benched, vaulted —
  first fully composed verified function (mean called from devsq).

### 2026-08-23 02:40 — H: author hints (HG1)
- wish.rs: `hint "text"` lines -> hints Vec; excluded from canonical()
  (rule 12: advice never evidence).
- forge.rs prompt gains AUTHOR GUIDANCE block; main.rs check prints hints.
- Tests: order preservation, canonical stability under hint edits,
  unquoted rejection. 102 tests green.

### 2026-08-23 03:20 — E: recipe effects (write/dump/log)
- recipe.rs: Write/Dump/Log statements; log templates interpolate %vars;
  typechecking validates targets.
- program.rs: deterministic C codegen — CSV rows, {"name": value} JSON,
  printf logs. Effects run at driver runtime only; sieve untouched.
- EG2 gate: e2e writes exact CSV/JSON/log content from a solved function.
- 103 tests green.

### 2026-08-23 04:00 — Intermediate report
- docs/reports/2026-08-23-intermediate-report.md: milestones, measured
  results (convergence, lift curve, parity tables, capability boundary +
  architectural closure), honest limitations, roadmap state.

### 2026-08-23 04:10 — Plan: cloud sampler backends + provenance
- docs/plans/2026-08-23-cloud-samplers.md; .env gitignored.

### 2026-08-23 04:40 — Cloud samplers: dotenv + curl transport + openai/gemini backends
- src/dotenv.rs: .env reader, never overrides real env.
- src/cloud.rs: curl HTTPS transport; API key via 0600 header file
  (`-H @file`), deleted on drop — never in argv/logs.
- src/sampler.rs: Kind enum; openai chat/completions body+parse;
  gemini generateContent with responseSchema {name,params[{n,t}],ret,body}
  (type enum kills param-type typos by construction) + reassembly;
  Usage accumulation. Fixture built programmatically after hand-counted
  bracket fixture failed twice (ISSUES entry).
- 107 tests green.

### 2026-08-23 05:00 — C1-C3: cloud sampler dispatch, flags, provenance
- forge.rs: ForgeConfig gains backend/endpoint/model/api_key_env; cloud
  sampling path (sequential curl, retry/backoff, 429-aware) returning Usage.
- main.rs: .env loaded at startup; new flags --sampler-backend/--endpoint/
  --model/--api-key-env; per-round token reports; provenance meta (prompt
  text + sha + sampler params) stored in vault manifests via put_meta().
- vault.rs: put_meta(extra) shallow-merged into manifest; put() kept.
- Docs: AGENTS rule 10 amended (configured endpoints, key hygiene,
  reproducibility caveat); README cloud usage section.
- 115 tests green.

### 2026-08-23 05:40 — C4 live gates + oracle fixes found by cloud candidates
- G-cloud ✅: ledger-wrapping solved via Gemini flash-lite (3/6 survivors,
  token report 1998/422), vaulted under same key as local solves.
- HG2 ✅: rms.ont SOLVED by cloud forge with hints — 5-6/8 survivors
  first round, vaulted (new key: hints excluded from identity as designed).
- Oracle gaps exposed by live candidates, fixed:
  - pure-float comparisons crashed in int_of → float path for all binops
    when any operand is F64 (interp)
  - Int==F64 promotion missing in checker Eq arm
  - arith.cmpf needs ordered-float predicate names (oeq/une/olt...)
  - gemini schema params carried % sigil + dotted names → clean_ident()
    normalization; MAX_TOKENS_CLOUD=1536 (composed bodies truncate at 512)
  - --samples/--seed flags were dropped by forge_config rewrite (found via
    live run); fcfg hoisted out of candidates branch
- Default cloud model updated per provider guidance: gemini-3.5-flash-lite.
- ONTIC_DEBUG=1 dumps raw reassembled candidates.

### 2026-08-23 05:20 — Vocabulary: wish -> gen (project-wide)
- Mechanical sweep across src/docs/examples; module gen.rs; Gen type;
  --gen selector; historical docs untouched; AGENTS vocabulary note.
- NEW: `ontic key <file.ont>` — canonical SHA-256 authority for external
  tools (pyous shells out; no second implementation of canonical()).
- Verified: ontic key output matches vault keys exactly.

### 2026-08-23 06:00 — PY-GATE: pyous bridge + math-dialect lowering fix
- examples/pyous.py: gen() spec→callable; key authority via `ontic key`
  subprocess; vault cache-hit requires artifacts.lib; ONTIC_AUTO_SOLVE=1
  opt-in solve-on-miss; numpy zero-copy Flat-5 args; __sieve_meta__.
- pipeline.rs: --convert-math-to-llvm pass added (Ubuntu mlir-translate does
  not auto-register math dialect — same class as memref.dim issue).
- main.rs: --samples/--seed plumbing restored (regression found live);
  fcfg hoisted; gemini default model gemini-3.5-flash-lite per provider.
- Live PY-GATE: cold genesis 14.4s via Gemini (12 candidates, 11 honest
  kills incl. 5× NaN-on-empty probes), 6 survivors, native bench 924ns,
  warm cache-hit 6ms, rms([2,8]) exact from Python.

### 2026-08-23 06:40 — Sampler ablation control experiment (G4)
- genrand.rs: type-directed random sketch generator (well-typed by
  construction; references params; depth-bounded folds/broadcasts/reductions).
- forge.rs: Backend::Uniform dispatches to local enumeration.
- main.rs: `ontic ablate <file>` runs uniform + configured sampler arms,
  prints per-stage survival table.
- Results:
  ledger-wrapping: uniform 0/8 survivors vs gemini 8/8.
  rms (hints):     uniform 0/6 (S2/S3 kills) vs gemini 5/6.
- Verdict: the transformer earns its slot decisively on both trivial and
  hard tasks; type-directed enumeration never produces correct semantics.
