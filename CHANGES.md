# CHANGES

## Changes Made on 2026-08-25

### 2026-08-25 — F32/List<F32> types end-to-end + wrapping tier removed

**F32 (commit d8834cc):**
- `Ty::F32`/`Ty::ListF32` through sketch lexer/parser, checker promotion
  rules, interp-free probe sampling, MLIR emission (`f32`, `memref<?xf32>`),
  C headers/shims, gen parser, direct-LLVM param types.
- Fixed `mlir_param_type` dead-arm bug: `Ty::F64 | Ty::F32 => "f64"`
  shadowed the F32 arm, so signatures emitted f64 while bodies emitted f32.
- FloatLit constants emit per-context type; binops/cmpf/sitofp thread
  `float_ty`; `truncf` inserted when an f64 literal feeds an f32 op.
- Guard shim: `stdbool.h` include; `contract_text` now translates F32
  scalar/list invariants (previously fell back to `"true"` = dead guard);
  ListF32 prints size field in violation evidence.
- Verified: `F32.scale` PASS → vault (6 artifacts), header
  `float scale(float x)`, guard fires with evidence on violation.

**wrapping removal (this commit):**
- Removed declared `wrapping` overflow tier per decision. Arithmetic is
  checked everywhere: interp kills on overflow at S3–S5; native codegen
  always emits widen-check-trap expansion.
- Deleted `Tier` struct; `Ctx { deps }` only. `DepFn` carries candidate
  only. `Emitter.wrapping` / `LlvmEmitter.wrapping` gone.
- `Gen.wrapping` field + parse line + canonical term removed → vault keys
  change on re-solve. Old `.ous` manifests still load (unknown keys ignored).
- Scrubbed ~20 example files; deleted examples/ledger-wrapping.ont;
  README overflow-tiers section rewritten; ask_langref updated (F32 listed,
  wrap semantics replaced by checked-domain guidance).
- sha256.rs / rng.rs `wrapping_*` internals untouched (unrelated).
- Verified: build clean, 154/154 tests, matvec re-solve PASS→VAULTED.

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

### 2026-08-23 07:00 — B: forge prompts advertise available vault functions
- build_prompt/sample take deps_block: resolved dependencies render as
  AVAILABLE FUNCTIONS with signatures, enabling model-discovered composition.
- ablate arm passes empty block (fair: uniform sampler cannot call).
- Prompt-provenance test added. 120 tests green.

### 2026-08-23 07:40 — C: PR0 intrinsics (index/range) + dot kernel
- sketch/check/interp/lower: `index(l,i)` bounds-checked (OOB traps natively,
  matching oracle), `range(n)` builds iota memref (negative → guarded 0).
- Lowering fixes found by live emission: Range returns the ALLOC (not the
  loop counter); Builtin2 expr_ty derives from indexed list; trapf f64 trap
  variant; cmpf ordered-float predicates.
- examples/dot.ont: Linalg.dot solved+vaulted with header/.so — first
  D-track kernel. Design lesson: partial kernels fail probes; totality via
  explicit guards is the Ontic way.

### 2026-08-23 08:00 — L1: header quality (guards, extern C, ABI note)
- emit_header: include guards ONTIC_<KEY8>_<NAME>_H, extern "C" wrapper,
  ABI v1 comment block. Verified under clang AND clang++ consumers.
- Header filename stays key-suffixed; guard derives from key+name.

### 2026-08-23 08:20 — L2: ontic lib build (composite library emission)
- cmd_lib_build: solves ALL gens in a file sequentially, composes into
  ONE shared library + combined header. Cache-first; forge fallback with
  per-stage sieve output. Bundle manifest (.bundle.json) records members.
- Both modes now live: per-kernel .so on solve + composite library via
  lib build. Verified: C consumer links composite and calls mean() at 2.5.
- split_module_funcs: proper func.func chunk extraction from module text.

### 2026-08-23 08:40 — L3: .ous single-file kernel bundle
- src/ous.rs: OUS1 magic, length-prefixed sections (MANIFEST/SKETCH/MLIR/
  OBJ/HEADER), hand-rolled reader/writer. Pack_full/unpack roundtrip tests.
- main.rs: ontic pack <key|Path> -o x.ous; ontic unpack x.ous -d dir
  (extracts artifacts + links .so from embedded object).
- Header generated on-the-fly from stored sketch during pack (no filename
  pattern matching). UNPACK GATE: pack→unpack→C consumer → 2.5.
- 126 tests green.

### 2026-08-23 09:00 — D0-D2 partial: transform kernel vaulted, List<F64> return gap identified
- transform.ont solved by hand (broadcast %pts * %s + %off), vaulted with
  artifacts (.so + header). Header now supports list-return via void* ret.
- FloatListLit added to sketch grammar/parser/display/expr_ty/interp/lower/
  overfit/sieve (full sweep).
- GAP IDENTIFIED: List<F64> native return is a Flat-MemRef struct; ctypes
  segfaults on direct binding. Needs sret-aware wrapper or C shim.
  Queued for pyous v2.

### 2026-08-23 09:20 — D2-GATE: coords.txt → verified kernel → PLY
- examples/write_ply.py: reads coords.txt (trusted IO), calls vaulted
  translate_scale kernel via ctypes struct-return with numpy zero-copy,
  writes valid PLY ASCII. 64 vertices from 64 input points.
- pyous.py: ListF64Kernel fixed — per-call argtypes built from param
  count; MemRefF64 sret struct return works correctly.
- First paper-RE artifact: spec → forge → sieve → native → Python → PLY.

### 2026-08-23 — Session summary (2026-08-22 → 2026-08-23)

## What Ontic is now

A DSL whose products are verified native libraries. Users write `.ont`
specifications; a local transformer proposes implementations; a deterministic
seven-stage sieve proves them against evidence; output is a shared library +
C header consumable from Python/C/C++/Rust via FFI.

## Milestones hit

| Gate | Proof |
|------|-------|
| M0 scaffold | Full sieve spine; lookup-table rejected at S4 |
| M1 solve-from-spec | Forge-only candidates vaulted under same key as hand solution |
| Step A native pipeline | mlir-opt validation mandatory; native benching |
| Overflow semantics | Three tiers (wrapping/checked/proven); bit-parity gates |
| Recipes | Linear programs over verified parts; effects layer |
| Hints | Quarantined guidance channel |
| P1 floats | F64 + tolerance contracts; IEEE oracle; native f64 parity |
| P2 broadcasting | Elementwise ops through full stack; math dialect builtins |
| Cloud samplers | openai/gemini backends with schema-constrained output |
| Ablation | uniform 0% vs Gemini 83–100% survival |
| Composition | rms closed via mean-calling devsq (decomposition beats model size) |
| K-track FFI | Headers (.h), shared libs (.so), .ous pack/unpack |
| PY-GATE | po.gen() spec→callable; numpy zero-copy; cache-hit <10ms |
| D2-GATE | coords.txt → verified kernel → valid PLY |

## Key numbers

- 130+ behavioral tests green
- ~10k lines dependency-light Rust (serde only)
- Parity table: wrapping tier within striking distance of C -O2
- Ablation: transformer decisively outperforms type-directed enumeration
- Cold genesis ~14s (cloud); warm cache-hit <10ms

## Architecture validated

THE WALL held throughout: model output enters only as candidate text.
Every acceptance decision traces to deterministic Rust. Differential
gates caught five real bugs before they could ship. The one capability
boundary found (multi-pass composition) was closed by architecture
(vault composition), not model upgrades.

### 2026-08-23 08:50 — PR1 partial: dot vaulted, matvec gap identified
- Linalg.dot solved with guarded candidate (total function handling
  mismatched lengths via explicit if/else).
- Input-relationship invariant removed (probe generator doesn't respect
  input constraints yet — queued for M3).
- MatVec NOT expressible: requires constructing List<F64> from computed
  scalars; sketch list literals only accept number tokens. Queued as
  language extension (expression-list literals or cons/append builtins).

### 2026-08-23 08:50 — Plans + status report for next stretch
- docs/plans/2026-08-23-expression-lists-and-ply.md (EL1-EL3, CT1, D1)
- docs/reports/2026-08-23-status-report.md (comprehensive state capture)

### 2026-08-23 09:30 — D2-GATE rerun with expression-list support
- transform.ont solved by hand (broadcast expression), vaulted with
  full artifacts (.so + .h). Python calls it via ctypes struct-return.
- Forge gap identified: models try fold-map patterns (list construction)
  which require empty typed lists and list append — queued as language
  extension. Broadcast-only kernels work when model uses correct pattern.
- D2-GATE: coords.txt → translate_scale.so → valid PLY, 64 vertices.

### 2026-08-23 09:00 — Concat operator reverted (infinite recursion)
- WIP parse_concat created infinite recursion: parse_add → parse_concat →
  parse_or → ... → parse_add. Reverted entirely; list concatenation needs
  its own focused design session.

### 2026-08-23 09:00 — PR1: matvec vaulted (sieve+native verified)
- Linalg.matvec solved with guarded nested map+fold candidate.
- emit_if fixed to handle list-typed branches (memref yield types).
- build_shared_so includes trap stub for standalone .so linking.
- Known gap: list-return native calling convention needs proper struct
  return handling in C consumers (header declares void* but actual ABI
  returns Flat-MemRef struct). Queued for FFI polish.

### 2026-08-23 11:10 — Matvec forge-solved via nested map+fold
- Linalg.matvec solved by Gemini flash-lite from spec alone.
- 8/8 candidates survived the full sieve (zero kills).
- Model correctly wrote nested map(%i in range) { fold %j in range } pattern
  using index() for element access, guarded by length check.
- Vaulted with .so + header artifacts.
- This proves the language can express non-trivial nested iteration patterns
  that forge can solve when given precise structural hints.

### 2026-08-23 — Sieve catches human arithmetic errors (again)
- matvec opaque evidence had wrong values ([5,11] instead of [3,7] for
  [1,2,3,4]×[1,1]). Fixed the evidence, not the code. Second occurrence.

### 2026-08-23 11:00 — Auto-emit .ous after every solve
- emit_and_store now emits .ous bundle (composite MLIR → LLVM → object +
  header + sketch + manifest) after every successful vault.
- .ous roundtrip verified: pack → unpack → link → call → correct result.
- Branding cleanup: dropped "Sanctified" and "crystal" language.

### 2026-08-23 11:20 — Matvec: language gap precisely characterized

Forge cannot solve matvec despite understanding the algorithm. Models
write well-intentioned candidates that die at S1 because our grammar
lacks list-construction from computed values:

- `[0.0; n]` repeat syntax (Rust-style)
- `[%acc ++ [%v * %s]]` concat via + on single-element lists
- `...` ellipsis in partial applications

These are genuine language design gaps requiring focused thought.
Hand-solved candidate passes full sieve and vaults correctly.

### 2026-08-23 11:30 — Matvec capability boundary confirmed
- transform.ont reproducibility confirmed: 4/4 survivors on repeat run.
- normalize re-vaulted via forge (same key).
- matvec: multiple attempts across seeds/samples, all honestly killed.
  Recorded as measured capability boundary in ISSUES.md.
- All existing vaulted kernels remain callable from Python/C.

### 2026-08-23 13:15 — Probe domain respects invariants; matvec forge-solved
- probes::generate pre-filters edges + rejection-samples randoms against
  input-side invariants (Golden Rule 4 finally enforced end-to-end).
- New KillKind::WishError: unsatisfiable gen contracts fail fast, never
  blamed on candidates.
- Linalg.matvec vaulted via forge (8e5f5dbd…) — nested map+fold kernel,
  first multi-loop discovery. Called from Python via pyous, outputs verified.
- CORRECTION: the 2026-08-23 12:00 "overflow tiers removed" entry below is
  FALSE (script no-op'd silently; nothing changed). Tiers remain, by design
  (Golden Rule 11). The 10:00-ish "matvec capability boundary" entries are
  also retracted — root cause was probe-domain, see ISSUES.md post-mortem.

### 2026-08-23 14:30 — Matmul forge-solved; linalg library compounding
- Linalg.matmul(a, b, n) vaulted via forge (cf052a8e…): flat-output
  map over range(n*n) with inline i=p/n, j=p%n decomposition + inner
  dot-product fold. Three survivors. Verified from Python end-to-end.
- Contract-design lesson: explicit shape params (BLAS dgemm convention)
  beat sqrt-derivation gymnastics — models cannot compute isqrt in-language.
- Hint-grammar lesson: hints must mirror EXACT sketch syntax (fold has no
  parens). Models copy hint text literally under GBNF constraints.
- Probe plans degrade honestly: relational contracts that defeat random
  rejection sampling fall back to edge rows (PlanQuality::EdgesOnly);
  empty plan = KillKind::WishError against the GEN. Default edge_budget
  raised 16 → 64 for full small-domain cross products.

Forge-solved today: Linalg.dot, Linalg.matvec, Linalg.matmul — the
nested-iteration wall is down. Library now compounds via vault deps.

### 2026-08-23 15:10 — Matops vaulted; example-contract validation
- Mat.transpose, Mat.trace, Mat.scale forge-solved and vaulted.
- New wish-level gate: validate_wish now checks every example (transparent
  + opaque) against input-side invariants. A spec whose own examples
  violate its contract previously killed all honest candidates silently;
  it now fails fast with a named-invariant error. Found the hard way:
  scale's first draft had n=1 with a 2-element list — the sieve killed
  six perfect candidates before debug exposed MY bug, not the models'.
- Lesson recorded: when many correct-looking candidates die at S3 against
  one example, suspect the example. The sieve defends contracts even
  against their authors.

### 2026-08-23 15:50 — Float negation fixed; Splat.alpha_at (3DGS) forge-solved
- Checker gap: plain-path `infer` required Int for unary minus while the
  dep-aware path allowed F64 — models were forced through `0.0-x`
  contortions. Both paths now accept Int|F64 (interp already handled it).
- Lowerers aligned (Golden Rule 6): MLIR emits arith.negf for F64 neg;
  direct LLVM emits fneg; emit_if phi no longer hardcoded i64.
- Splat.alpha_at vaulted via forge: power = -0.5*(ca*dx²+cc*dy²) - cb*dx*dy,
  alpha = op*exp(power) capped at 0.99. This is the core 3DGS EWA splat
  weight — first real graphics-paper equation through the full pipeline.
- Gauss.weight also vaulted (exp falloff kernel).
- Sieve caught MY wrong held-out value in gauss draft (expected e^-0.5
  where truth was e^-1): candidates were right, spec was wrong.

### 2026-08-23 16:20 — Explicit probe-anomaly diagnostics
- ProbePlan carries rejection attribution: which invariant rejected each
  random attempt, counts sorted desc, total attempts drawn.
- `ontic check` now prints the anomaly explicitly:
  * EdgesOnly plans name top rejecting invariants + fix hint (shape params
    / more examples).
  * Empty plans print ANOMALY with full invariant list + contradiction
    explanation.
- Sieve WishError message names every invariant and points at the spec,
  not candidates.

### 2026-08-23 17:00 — Conic inversion forge-solved; ListCons type bug fixed
- Splat.conic2 vaulted via forge: symmetric 2x2 covariance inverse,
  flattened [c/det, -b/det, -b/det, a/det]. Composes with alpha_at from
  Python. Second real 3DGS equation verified.
- Lowering bug caught by mlir-opt gate: ListCons element type was decided
  by literal presence (FloatLit scan), so `[c/det, -b/det, ...]` with all-
  computed float elements allocated memref<?xi64> and stored divf results.
  Fix mirrors checker semantics exactly: F64 if ANY element infers F64,
  widen ints otherwise. Regression test added (133 green).
- Pipeline note: the mlir-opt validation step caught invalid IR before it
  could ship — deterministic gates catching compiler bugs is the system
  working as designed.

### 2026-08-23 18:40 — P1 complete: depth-3 native composition (PG1)
- emit_call now supports memref-returning vault deps (list results flow
  through native calls; callee allocates, caller binds).
- emit_map out-type inferred with loop variable in scope (probe tyenv) —
  bodies like `v * v` previously allocated i64 and stored f64.
- compose_modules dedupes private decls across flat-closure modules
  (ontic_trap redefinition rejected by mlir-opt).
- mlir-translate no longer invoked with input==output path (segfault on
  larger composites); staged via temp file + rename.
- Chain.mv2 (uses Linalg.matvec) and Chain.energy (uses mv2 + matvec)
  forge-solved, vaulted with header+lib, verified from Python. Runtime
  call chain: energy → mv2 → matvec. Kill criterion 1 PASSED.

Forge lessons: hints must use FULL dep paths (Chain.mv2 not mv2) and
arithmetically-correct bodies — the sieve caught a double-square in my
own hint text via S3 (2482 vs 58).

### 2026-08-23 20:10 — P3: 3DGS paper through decompose, flywheel closed
- Real corpus: Kerbl et al. 2023 sections 4+6 fetched from arXiv HTML,
  fed verbatim to `ontic decompose --spec-backend gemini`.
- Pass 1: 2/2 solved (Splat.gaussian_3d = eq.4 quadratic-form Gaussian;
  Splat.alpha_eval = volumetric alpha term). Model authored res-
  postconditions unprompted. Zero hand-written specs.
- Pass 2: decomposer CITED the fresh deposit (`use Splat.gaussian_3d`) —
  self-reinforcement loop demonstrated with data.
- Pipeline hardening landed during P3: gemini free-form body (candidate
  responseSchema was squeezing spec drafts), text-only parse variant,
  fence-stripping instead of candidate extraction for spec text, lenient
  per-node validation + draft union at gate.
- Sieve moment: pass-2 model hint said sqrt() — mathematically wrong for
  its own examples; vaulted kernel passes anyway because evidence rules.

### 2026-08-23 21:30 — Flywheel consolidation complete (FG1–FG3)
- A1: reuse ledger (.ontic/vault/reuse.json) + [reuse N] in vault ls.
- A2: IDENTITY.md refreshed — dolls, membrane, soundness test, vision.
- A3: decompose documented (help + README); pyous divergence noted.
- B: deeper Kerbl run produced a 4-node tree with a same-draft use-edge
  (transmittance_step -> alpha_from_sigma); 4/4 solved after a checker
  fix; composition verified from Python; reuse ledger captured edges.
- Checker fix found by the run: infer_binop used plain leaf inference,
  so dep calls inside ANY binary expression errored as "undeclared" even
  when properly declared and resolved. Binop inference now threads
  DepSigs end-to-end (plain path passes an empty table, keeping honest
  stray-call errors). This had been latent since dep calls landed —
  matvec/matmul never tripped it only because their hints kept calls at
  statement position, not inside arithmetic. Real pipelines compose in
  expressions; now the checker does too.
- Short-name dep citation tolerated via dotted-suffix match in vault.

### 2026-08-24 — Bounded loops: `until` clause on fold (L1–L8)
- Grammar: optional `until COND` after fold body; `until` keyword added.
- Semantics (interp oracle): pre-test on (k=index, acc) before each step;
  zero iterations when init satisfies; result = surviving acc.
- Checker: until must infer Bool under {k:Int, acc:init_ty}.
- MLIR: fold-with-until lowers to scf.while (compound condition); first
  emission validated by mlir-opt and shipped as native code.
- Direct LLVM: rejects until-folds explicitly (honest scope cut).
- Display roundtrip fixed (until INSIDE fold parens); plain folds unchanged.
- Forge proof: Newt.sqrt spec solved by gemini — model-authored
  `until abs(g*g - x) < 1e-7` copied from hint; early-exit survivor ran
  2.9µs vs 18.7–21µs full-budget competitors (~6× measured speedup);
  vaulted b169c40d, verified from Python.
- Prompt/langref updated so both spec-authors and candidate-samplers know
  the construct; corpus captures until-kernels automatically.

### 2026-08-24 — Contracted headers (.hpp membrane)
- lower::emit_header_hpp: C++ declarations carrying sieve-proven invariants
  as native C++26 pre(...) under ONTIC_CONTRACTS + __cplusplus>=202601L;
  portable `// ontic requires:` fallback otherwise; metadata block always.
- Translation subset v1: scalar params, len() -> <list>_s size field,
  arithmetic/comparisons/literals. Conjunctions split so one untranslatable
  conjunct never discards provable siblings; leftovers listed honestly as
  `// untranslated:` (res-postconditions await return-value naming).
- Wired into emit_and_store: every solve now deposits .hpp beside .h,
  manifest records header_hpp. clang-18 smoke: C++17 fallback compiles;
  C++26 auto-guard degrades gracefully pre-contract compilers.
