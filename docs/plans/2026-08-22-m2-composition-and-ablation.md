# M2 — Vault Composition + Sampler Ablation

**Date:** 2026-08-22
**Author:** Randy Smits-Schreuder Goedheijt
**Status:** Approved (session of 2026-08-23)
**Depends on:** M0/M1 complete; vault live.

## 1. Keystone decision

The capability boundary found via rms.ont (models cannot compose two passes)
is answered by architecture, not model upgrades: **candidates may call
verified vault symbols**. Decomposition moves from the model to the spec
author; the machine implements+verifies each piece.

Stdlib policy (user directive): the library compounds — every verified wish
can graduate into an ever-expanding standard library that later wishes call.
Trusted intrinsics (`len`, `sort`, `range`, ...) and graduated synthesized
functions are first-class citizens of the same lib.

## 2. Design

### Wish header dependencies

```
use Stats.mean

fn Stats.meansqdev(%xs: List<F64>) -> F64
  | %res >= 0
  => ...
```

- `use` lines before `fn` declare deps (same syntax as programs).
- Sketch bodies may call `Path.name(%args)` where Path.name is a declared dep.
- Checker binds call arity/types against the DEP'S VAULT SIGNATURE.
- Interp executes dep calls by evaluating the dep's stored sketch text with
  its own tier context.
- Forge prompts include each dep's signature + invariants ("available
  functions you may call").
- Sieve: S3/S4/S5 run deps on demand; a dep failing = candidate kill with
  dependency-attributed reason.

### Lib graduation

- `.ontic/lib.manifest`: one path per line — promoted, stable symbols.
- `ontic lib ls` / `ontic lib promote <Path>` / `ontic lib demote <Path>`.
- Graduated symbols are callable WITHOUT local `use` (implicit global scope).
- Promotion is curation, never automatic.

### Sampler ablation

`ontic solve --sampler uniform|llm`:

- uniform: seeded enumeration of grammar-valid candidates (bounded depth),
  no model involved.
- Report per-stage hit rates side by side (parse/typecheck/evidence/probe
  survival), plus survivor count and best bench.
- Purpose: quantify whether the transformer earns its slot vs grammar-
  constrained enumeration. Standing experiment per benchmark.

## 3. Gates

| # | Gate |
|---|------|
| G1 | mean wish solved + promoted to lib |
| G2 | meansqdev composed over `Stats.mean` call survives full sieve |
| G3 | rms.ont benchmark closes via composition (mean + devsq) |
| G4 | ablation report exists for >=1 benchmark with both samplers |
