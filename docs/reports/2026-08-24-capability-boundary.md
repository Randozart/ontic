# Capability Boundary Report

**Date:** 2026-08-25
**Status:** Generated from 23 algorithm-class probes + grammar analysis
**Goal:** Answer precisely: what code can Ontic express today, what can't it, and what minimal extension per gap.

---

## 1. Language surface inventory

### Types (sketch.rs `Ty`)
| Type | Status |
|---|---|
| `Int` (i64) | ✓ Expressible |
| `F64` (f64) | ✓ Expressible |
| `Bool` | ✓ Expressible |
| `List<Int>` | ✓ Expressible |
| `List<F64>` | ✓ Expressible |
| `F32` | ✗ Missing — all floats are 64-bit |
| `List<List<T>>` | ✗ Missing — no nested lists |
| `(T, U)` tuple types | ✗ Missing — tuples exist only inside fold bodies |
| `String` / `Char` | ✗ Missing |
| Structs / records | ✗ Missing |

### Expressions
| Form | Status |
|---|---|
| Literals (int, float, bool, list, float-list) | ✓ |
| Variables, let bindings | ✓ |
| BinOp (arithmetic, comparison, logical, concat) | ✓ |
| UnOp (neg, not) | ✓ |
| if/else | ✓ |
| Call (vault deps) | ✓ |
| map { body } over list/range | ✓ |
| fold acc from init, list { body } | ✓ |
| fold ... until <bool-expr> | ✓ (scf.while early exit) |
| fold ... with multi-accumulator { (expr, expr) } | ✓ (just landed) |
| Restricted tuple expression (fold bodies only) | ✓ |
| ListCons [e1, e2, ...] | ✓ |
| Index (list, i) — Builtin2 | ✓ |
| While / unbounded loops | ✗ Ruled out (soundness test: total oracle) |
| Recursion | ✗ Ruled out (same reason) |
| Higher-order (pass function as value) | ✗ Inline bodies only |

### Builtins
| Builtin | Domain | Operates on |
|---|---|---|
| `len(x)` | list length | List |
| `range(n)` | 0..n | Int |
| `sum(x)` / `max(x)` / `min(x)` | reductions | List |
| `abs(x)` / `sqrt(x)` / `exp(x)` / `log(x)` | scalar math | Int/F64 |
| `index(list, i)` | array indexing | List, Int |
| `min_el(a, b)` / `max_el(a, b)` | elementwise | List, List |

---

## 2. Algorithm-class × capability matrix

### 2.1 Probe results (empirical evidence)

Each representative was tested via `ontic check` on a minimal .ont spec.
Probe quality: **Full** = honest domain coverage (260+ rows); **EdgesOnly** = relational invariants defeat rejection sampling (≤9 rows); **Anomaly** = unsatisfiable over type domain.

## Addendum — 2026-08-26 scorecard re-run (F32 + linear probe solver)

Re-probed with the integer-linear constraint solver
(`src/probes_solver.rs`, commit 416da45) and the F32 type. Specs archived
at `docs/reports/assets/capsweep-2026-08-26/`.

| Class | Before | After | Driver |
|---|---|---|---|
| QR step | EdgesOnly (5) | **Full (258)** | probe solver (`len == n*n`) |
| Cholesky | EdgesOnly (2) | **Full (256)** | probe solver |
| Conjugate gradient | EdgesOnly (4) | **Full (260)** | probe solver |
| Matrix multiply | EdgesOnly (9) | **Full (260)** | probe solver |
| Jacobi relaxation | EdgesOnly (9) | **Full (256)** | probe solver |
| Moving average | EdgesOnly (4) | **Full (258)** | probe solver (`k <= len`) |
| K-means step | Anomaly (0) | **Full (256)** | probe solver (multi-constraint) |
| BFS adjacency probes | EdgesOnly (3) | **Full (259)** | probe solver; traversal still structurally partial |
| Softmax / conv1d / bsearch / knapsack (spot) | Full | Full | no regression |

**New totals: 21/23 classes at Full probe coverage** (was 13/23).
Probe-level degradation is eliminated for every expressible class.
Remaining non-Full are structural, not evidential: Levenshtein 2D
(needs nested lists, Gap 3) and unbounded graph traversal (Gap 5).

| Algorithm class | Representative | Probe quality | Expressible? | Notes |
|---|---|---|---|---|
| **Numeric linear algebra** | QR step | EdgesOnly (5) | Yes (flat) | Matrix as flat list + index math; probe degrades on `len(a)==n*n` |
| | Cholesky | EdgesOnly (2) | Yes (flat) | Same relational degradation |
| | Conjugate gradient | EdgesOnly (4) | Yes (flat) | Multi-system solve; needs iteration budget |
| | Matrix multiply (adj_mul) | EdgesOnly (9) | Yes (flat) | `matmul.ont` already in vault |
| **Iterative solvers** | Jacobi relaxation | EdgesOnly (9) | Yes (flat) | 2D grid as flat + n*n indexing |
| | Bisection (numeric) | Full (262) | Yes | Scalar iteration via until-fold |
| **Sorting / search** | Binary search | Full (264) | Yes | `bsearch.ont` |
| | Insertion sort | Full (260) | Yes | Flat list, bounded fold |
| **Graph** | BFS | EdgesOnly (3) | Partially | Flat adjacency matrix expressible; **traversal unbounded** — needs queue/while; sieve can't prove termination |
| | Dijkstra | EdgesOnly (2) | Partially | Same: flat matrix OK, but priority queue + unbounded loop structurally out |
| | PageRank | EdgesOnly (8) | Partially | Iteration expressible via fold; convergence via until; but "converged" predicate on vector difference needs 2D traversal |
| **Strings** | Palindrome | Full (260) | Yes | List<Int> encoding works |
| | Substring count | Full (268) | Yes | Flat scan expressible |
| **Dynamic programming** | Knapsack 1D | Full (276) | Yes | 1D DP table as flat list |
| | Levenshtein 2D | Full (272) | **Structurally blocked** | 2D DP needs `List<List<T>>` or manual `i*n+j` flat-index — spec type-checks but implementation can't express 2D update pattern |
| **Geometry** | 2D cross product | Full (320) | Yes | Scalar-only |
| | Point-in-polygon | Full (256) | Yes | Winding number via fold |
| **Signal** | 1D convolution | Full (260) | Yes | Fold over kernel × window |
| | Moving average | EdgesOnly (4) | Yes (flat) | `k <= len(x)` relational degrades probes |
| **ML** | Softmax | Full (258) | Yes | exp + sum + map |
| | ReLU backward | Full (261) | Yes | Elementwise + condition |
| | K-means step | **Anomaly** (0) | Yes (spec) | Relational constraints unsatisfiable by rejection sampling — **Z3 win case** |
| **FFT** | Butterfly | Full (261) | Yes (small) | 2-point DFT expressible; **radix-N FFT** needs recursion/log-depth |
| **PageRank** | Power iteration | EdgesOnly (8) | Partially | Convergence via until; flat adjacency OK; but vector-diff predicate needs elementwise comparison |

### 2.2 Summary

| Category | Count | Examples |
|---|---|---|
| **Full expressible** (Full probes) | 13 | bsearch, softmax, conv1d, knapsack, cross2d, palindrome, substr_count, relu_bw, fft (2-point) |
| **Expressible but degraded probes** (EdgesOnly) | 6 | QR, cholesky, CG, matmul, jacobi, pagerank — all hit `len(a)==n*n` relational constraint |
| **Partially expressible** (flat works, algorithm structurally blocked) | 3 | BFS, Dijkstra, PageRank — flat adjacency OK but unbounded traversal out |
| **Structurally blocked** | 1 | Levenshtein 2D (needs `List<List<T>>`) |
| **Unsatisfiable probes** (Z3 win case) | 1 | K-means (relational multi-constraint) |

---

## 3. Gap taxonomy

### Gap 1: Float32 (effort: **S**)

**What's blocked**: Any ML kernel operating on f32 tensors (the natural ML type). Memory bandwidth halves without f32; SIMD lanes double. The 3DGS demo (flagship) uses f32 internally.

**Minimal fix**: Add `F32` to `Ty` enum. Parser: `"F32"` keyword. Checker: promote rules (F32 ↔ F64). Interp: `Value::Float32(f32)`. MLIR: `f32` type. Header: `float` vs `double`. Sentinel: `NANF`.

**Estimated effort**: S (mechanical — one new type, follow F64 pattern everywhere)
**Unblocks**: ML kernels at native precision, pixel ops, half-precision ML

### Gap 2: Tuple types (effort: **M**)

**What's blocked**: Multi-output functions. Mean+variance = two passes over data today. The 3DGS decompose report flagged: "projection/Jacobian layers not emitted unprompted" because they naturally return multiple values.

**Minimal fix**: Extend `Ty` to `Tuple(Vec<Ty>)`. Checker: tuple expressions infer `Tuple(T1, T2)`. Interp: `Value::Tuple(Vec<Value>)`. MLIR: multiple return values (`(f64, f64)`). Header: struct return or out-params. Sieve: need to handle tuple comparison (probe domain).

**Estimated effort**: M (touches 5+ files; MLIR emission non-trivial for multi-return)
**Unblocks**: composite kernels (mean+variance, SVD), richer decompose output

### Gap 3: Nested lists (effort: **L**)

**What's blocked**: Matrix-native code, 2D DP (Levenshtein), image grids, any paper with 2D structure. Currently requires flat-index math which makes specs unreadable and probes weak.

**Minimal fix**: `List<List<T>>` type. Parser: `List<List<F64>>`. Checker: uniform nesting. Interp: `Value::ListList(Vec<Vec<Value>>)`. MLIR: `memref<memref<?xf64>>` or strided. ABI: recursive flat-memref. Header: nested pointer+size structs.

**Estimated effort**: L (deep changes to type system, interp, MLIR, ABI; probe domain needs 2D support)
**Unblocks**: 2D DP, image kernels, matrix-as-first-class, tensor ops

### Gap 4: SMT probe widening (effort: **M**)

> **2026-08-25 UPDATE:** Partially resolved by `src/probes_solver.rs`
> (hand-rolled integer-linear solver, zero dependencies — the "hybrid"
> option below). QR/cholesky/matmul-style `len == n*n` relations and
> kmeans-lite multi-constraint specs now generate Full probe plans.
> Remaining: nonlinear multivariate and float relations still degrade
> honestly to EdgesOnly.

**What's blocked**: Any relational invariant degrades probe quality to EdgesOnly (≤9 rows). Seen live on: QR (5 rows), cholesky (2 rows), kmeans (0 rows — anomaly). This means 9/23 algorithm classes get weak verification evidence.

**Minimal fix**: Integrate z3 or hand-rolled constraint solver into `probes::generate`. For each relational invariant, encode as SMT constraint + type domain bounds, enumerate satisfying assignments with varied edge cases.

**Options**:
- **system libz3** via `z3` crate: full power, but external dependency (tension with GR10 self-containedness; serde precedent exists)
- **vendored libz3**: bundle the C library; ~50MB build artifact
- **hand-rolled interval propagation**: weaker but zero dependencies; covers linear constraints over integers; may miss nonlinear (e.g. `len(a) == n*n`)
- **hybrid**: linear constraints hand-rolled, nonlinear fallback to EdgesOnly

**Estimated effort**: M (z3 crate integration is ~200 LOC; hand-rolled is ~500 LOC but weaker)
**Unblocks**: honest probe coverage for ALL 23 algorithm classes; kills EdgesOnly degradation

### Gap 5: Unbounded traversal (effort: **XL / ruled out)**

**What's blocked**: BFS, Dijkstra, DFS, any graph algorithm with queue/stack traversal. Also: general recursion (divide-and-conquer, trees).

**Status**: Ruled out by soundness test (2026-08-24-bounded-loops.md:43). The interpreter must remain a total, bounded-step oracle. Unbounded loops break this invariant.

**Escape**: Fuel-parameterized recursion (explicit depth bound `fuel: Int`, invariant `fuel >= 0`, recursive calls on `fuel-1`). This would be sound (termination decidable, oracle stays total) but requires design doc touching the soundness test core.

**Estimated effort**: XL (design doc + parser + type system + interp + MLIR + soundness proof)
**Unblocks**: graph algorithms, divide-and-conquer, trees, any recursive data structure

### Gap 6: String type (effort: **L / membrane-routable)**

**What's blocked**: String algorithms, parsing, text processing.

**Status**: Strings are **membrane-routable** per GR14 (trusted writers pattern). No kernel synthesis needed — string ops are membrane code, called via `use` deps. No reason to add strings to the core language.

---

## 4. Membrane inventory

What already exists as callable intrinsics vs what's missing:

| Membrane intrinsic | Status | Called via |
|---|---|---|
| `sort` | Listed as trusted (IDENTITY.md) | `use Sort.sort` (not yet in examples) |
| CSV/PLY/OBJ parsers | Listed as trusted writers (GR14) | Recipe layer |
| `Linalg.matvec` | Vault-deposited, verified | `use Linalg.matvec` |
| `Chain.mv2` / `Chain.energy` | Vault-deposited | `use Chain.mv2` |
| `Stats.mean` | Vault-deposited | `use Stats.mean` |
| Graph traversal (BFS/Dijkstra) | **Missing** | Would need hand-written + vault deposit |
| 2D array ops | **Missing** | Blocked by Gap 3 (nested lists) |
| String operations | **Missing** | Blocked by Gap 6 (membrane-routable) |

---

## 5. Verification quality analysis

### EdgesOnly degradation: root cause

The probe generator uses **rejection sampling** against invariants. When invariants contain relational constraints (`len(a) == n*n`, `len(a) == len(b)`, `k <= len(x)`), rejection sampling fails because:

- Scalar domains are unbounded → random sampling rarely hits equality
- Relational constraints reduce the valid region to a measure-zero set in the full domain
- 256 rejection attempts yield ≤9 edge cases

**Impact**: 9/23 algorithm classes get EdgesOnly (2–9 probe rows). These specs ARE checked on edge cases, but interior coverage is empty. A spec that passes EdgesOnly is less trustworthy than one passing Full.

### Z3 impact projection

With Z3-backed probe generation:
- `len(a) == n*n` → solver enumerates n=1..5, picks representative `a` of size n*n
- `len(a) == len(b)` → solver samples pairs from same-length distributions
- Multi-relational (kmeans) → solver finds satisfying (n, k, pts, centers) tuples

**Projected outcome**: all 9 EdgesOnly classes upgrade to Full. K-means anomaly resolves.

---

## 6. Gravity ranking (which papers to eat next)

Ranked by how many paper-pipelines each extension unblocks:

| Rank | Extension | Papers unblocked | Effort |
|---|---|---|---|
| 1 | F32 | All ML/graphics papers (3DGS, transformers, convnets) | S |
| 2 | Z3 probes | All relational-invariant papers (matrix algos, geometry, signal) — same specs, stronger evidence | M |
| 3 | Tuple types | Multi-output papers (stats, decomposition, stateful transforms) | M |
| 4 | Nested lists | 2D DP, image kernels, tensor ops, matrix-native algorithms | L |
| 5 | Fuel-recursion | Graph algorithms, divide-and-conquer, tree traversals | XL |

### Recommended implementation order

**Phase 1** (quick wins, high gravity):
- F32 type end-to-end → immediate domain unlock for ML/graphics
- Z3 probe widening → fixes EdgesOnly for 9/23 classes, strengthens everything already expressible

**Phase 2** (structural extension):
- Tuple types → multi-output, richer decomposition
- Nested lists → 2D, tensor, image

**Phase 3** (design doc only):
- Fuel-recursion design doc → decide if graph algorithms are in-scope for the paper-flywheel

---

## 7. Honest assessment

**What the core language IS**: a total, bounded, functional-style array kernel language. By design, not by failure. The soundness test constrains it to what a deterministic oracle can prove.

**What the core language IS NOT**: a general-purpose language. Graphs, trees, strings, recursion — these live in the membrane or recipe layer.

**The ceiling**: ~10-20% of algorithmic content in a typical CS paper (the numeric inner kernels). But that 10-20% is the high-reuse, high-trust core that the flywheel deposits. The rest decomposes into membrane calls or is structurally out-of-scope.

**The real question**: does the paper-flywheel need to ingest papers where the 10-20% isn't enough? If yes → fuel-recursion design doc. If the target domain is ML/graphics/numerics → F32+Z3+tuples covers most ground.

---

## Appendix: Raw probe data

```
Full probes (260+ rows):
  bisect        262 rows    bsearch        264 rows
  conv1d        260 rows    cross2d        320 rows
  fft           261 rows    insertion_sort 260 rows
  knapsack      276 rows    levenshtein    272 rows
  palindrome    260 rows    point_in_poly  256 rows
  relu_bw       261 rows    softmax        258 rows
  substr_count  268 rows

EdgesOnly probes (2-9 rows):
  adj_mul       9 rows      bfs            3 rows
  cg            4 rows      cholesky       2 rows
  dijkstra      2 rows      jacobi         9 rows
  moving_avg    4 rows      pagerank       8 rows
  qr_step       5 rows

Anomaly:
  kmeans        0 rows      (unsatisfiable multi-relational contract)
```
