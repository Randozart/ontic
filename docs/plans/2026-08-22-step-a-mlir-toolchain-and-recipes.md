# Step A — MLIR Toolchain Integration + Recipes

**Date:** 2026-08-22
**Author:** Randy Smits-Schreuder Goedheijt
**Status:** Approved (session of 2026-08-22)
**Predecessor:** `2026-08-22-ontic-mvp.md` (M0 scaffold + forge live)

## 1. Goal

Make the MLIR path real: emitted IR must be validated by the actual toolchain,
survivor ranking must time *compiled* code (not the interpreter), and a linear
recipe layer lets users compose vaulted functions into runnable programs.

Toolchain baseline: Ubuntu noble, LLVM 18.1.3 installed; `mlir-18-tools`
(mlir-opt, mlir-cpu-runner, mlir-translate) installed via apt this session.
`llc`/`opt` already present.

## 2. Part A — toolchain becomes real

| # | Work | Gate |
|---|------|------|
| A0 | Smoke: run existing ledger emission through mlir-opt; record every rejection | know the gap |
| A1 | Fix `lower.rs` until clean; flip validation best-effort → mandatory in solve; corpus = all sketch-test programs re-emitted | zero rejections in cargo test |
| A2 | S7 honest benching: wrapper module inlines candidate into internal ~10k-iteration scf.for; ONE mlir-cpu-runner spawn; wall-clock ÷ iters. Interpreter timing kept as reported fallback | survivors ranked by compiled ns/op |
| A3 | Differential oracle: same probe inputs through interpreter AND JIT harness; mismatch ⇒ hard failure with input + both outputs | divergence is CI failure |
| A4 | Object path: `mlir-translate --mlir-to-llvmir | llc -O3 -o cand.o`; A/B vs `clang -O3` C fold using brievc protocol (interleaved ×N, LC_ALL=C, averages) | parity table committed under benchmarks/results/ |
| A5 | CLI `ontic build --emit-object`; docs; tests green; commit | — |

### Known risks (pre-named)

- Quoted SSA names (`%"items"`) — verify mlir-opt accepts; rename strategy if not.
- scf.for iter_args text syntax details.
- cpu-runner shared-lib paths (`--shared-libs`, libmlir_runner_utils) vary per
  distro — discovered in A0, pinned in one config constant.
- Timing noise → median-of-9 from day one; shared-machine etiquette documented.

## 3. Part R — recipes (linear programs)

Depends on A2 (JIT path). Design decisions locked this session:

- **Strictly linear**: bindings, calls, print. No if/loops in recipes — logic
  lives in wishes where it is verifiable. THE WALL preserved: recipes are
  unsieved glue between sieved parts.
- **print only**: no argv/files in v0.
- **Same file**: one `.ont` may hold many `fn` wishes plus at most one
  `program` block.

```
program Demo
  wish Ledger.total          // dependency; must be vault-verified before run
start
  %xs = [1,2,3]              // literal binding
  %r  = Ledger.total(%xs)    // call binding — arity/type vs vault signature
  print(%r)
end
```

Semantics: locals inferred; unknown dep ⇒ solve-on-demand then vault;
unsolvable ⇒ error naming the wish.

New modules:
- `src/recipe.rs` — parser + typechecker for program blocks.
- `src/program.rs` — module assembly: candidate fns + `main`; printf via
  libmlir_runner_utils during dev.

New CLI: `ontic run demo.ont` — resolve deps → assemble → execute via
cpu-runner → relay stdout.

Tests: recipe parse/typecheck units; e2e run test gated on toolchain presence
(skips cleanly when absent); negatives — unverified callee, arity mismatch.

## 4. Execution order

A0 → A5 fully green, then R. Commit after each gate.
