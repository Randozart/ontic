# Ontic — Strategy Report: The Nesting-Dolls Direction

**Date:** 2026-08-23
**Character:** Direction-setting session. No roadmap items executed; this document
records decisions, rejections, and falsification criteria agreed in discussion.
Supersedes the assembly-model portions of earlier plans where they conflict.

---

## 1. Where the project stands at time of writing

The pipeline is end-to-end proven on real kernels:

| Layer | Status |
|---|---|
| Sieve S1–S7 | Working; zero false accepts across hundreds of candidates |
| Forge backends | llama / OpenAI-compat / Gemini native / uniform enumeration |
| Vault composition | Proven for scalar chains (devsq-composed); list-returning callees blocked only in `emit_call` (src/lower.rs) |
| Python bridge | pyous: spec text → callable native kernel, numpy zero-copy |
| Verified library | ledger ops, transforms, dot, normalize, **matvec**, **matmul**, transpose, trace, scale, gaussian weight, **3DGS splat alpha**, **conic inversion** |

Session findings that made this possible are recorded in CHANGES.md and
ISSUES.md; the headline is that the matvec "capability boundary" was a
probe-domain bug, not a model limit. Contracts now bind their authors too:
the sieve caught two spec bugs authored by the project's own agent in one day.

## 2. North star (owner's words)

> The ability to write a kernel and an OS in Ontic.

Interpretation agreed in session: the OS clause is *hypothetical aspiration* —
"stochastic generation so good it could formulate an OS" — not a near-term
architecture commitment. It functions as a direction test: does a proposed
feature make pure verified cores more composable? If yes, it serves both the
3DGS path and the distant OS path simultaneously.

Intermediate goal: a pipeline steerable by a simple LLM, where the LLM writes
`.ont` files from a user prompt (`ontic ask`, planned separately).

## 3. The central decision: nesting dolls over program-writing

### 3.1 What was considered

Three assembly architectures were analysed for turning verified kernels into
running programs:

1. **Model 1 — deterministic recipes**: extend the recipe language with loops
   and conditionals; emit driver programs mechanically.
2. **Model 2 — LLM-written glue**: an LLM assembles final programs around pyous
   kernels; correctness gated by end-to-end acceptance examples.
3. **Hybrid** (initially chosen): typed recipe spine plus LLM escape glue,
   gated by confirmed e2e I/O pairs.

All three were superseded by the owner's reframing:

> We don't need Ontic to write full programs. We need it to wrap functions
> inside functions so often that final kernels become Russian nesting dolls
> that even a dumb LLM can glue into any language.

### 3.2 Why dolls win

| Property | Programs | Dolls |
|---|---|---|
| Verification surfaces | One per layer (glue, manifest, recipes) | **One — the existing sieve, applied recursively** |
| Glue surface | Program logic, needs review/tests | "Allocate buffers → call → read" |
| Shape checking | New static algebra needed | **Sieve kills wrong wiring at S3/S5** |
| Consumer effort | Understand the program | Read one header + examples |
| Fits BLAS/LAPACK reality | No | **Yes — real numerical libraries ARE nested composition** |

Market framing sharpened: "AI writes full programs" is a crowded, LLM-judged
race against trillion-parameter labs. "Verified kernels from specs, composed
to arbitrary depth" is an empty niche judged by machines. Weaker models are
an asset there, not a liability.

THE WALL generalises per artifact, not per layer-count. Dolls keep every level
of composition inside the one wall that already works.

## 4. C++26 contracts as membrane surface

C++26 (`pre`/`post`/`contract_assert`) adopted as the second emission target
(behind Python), with two binding conditions:

1. **Toolchain reality**: portable runtime checks compiled from the same
   `[pre][post]` metadata ship unconditionally; native contract syntax behind
   feature detection (Clang-first). Identical deterministic behaviour either
   way. Enforcement mode declared per build — never silently off (Golden Rule
   11 spirit).
2. **Glue dialect**: no manual memory management, spans/memref views over
   caller-owned buffers only, no exceptions across kernel boundaries,
   violations route to `ontic_trap`-style abort — sieve kill semantics at the
   membrane.

Key insight recorded: contracts migrate sieve authority INTO consumers. Vault
headers emit contracted declarations derived from gen invariants; call-site
violations by outside code become deterministic diagnosable failures instead
of silent miscomposition. Under the dolls model this lands on the outermost
membrane — exactly where checks matter most.

## 5. The domain ceiling — decomposed, not accepted

The array-language identity ("flat memrefs in/out, structured iteration")
was challenged and decomposed:

| Constraint | Verdict | Mechanism |
|---|---|---|
| Single return value | Solvable, cheap | Tuple returns (`Value::Tuple`, multi-sret ABI) |
| No structs | Dissolved free | SoA discipline — nine parallel lists per gaussian is idiomatic GPU layout anyway; CSR flattens graphs |
| No while/recursion | Partial lift, high value | **Bounded loops + break** (see below) |
| Strings, pointer graphs, general recursion | Permanent residents of the shell | By design — they fail the soundness test |

### 5.1 Bounded loops preserve the oracle

Numerical computing universally bounds its own iteration (`max_iterations`
in LAPACK refinement, CG, EM, Adam — float convergence is probabilistic).
A `loop %k < %max_iter { … if %done { break } }` construct therefore:

- keeps termination decidable (machine-managed saturating counter),
- keeps the interpreter total (hard deterministic step budget; overrun =
  RuntimeError kill — same class as div-by-zero),
- stays S6-compatible, lowers to `scf.for`/br-loops the emitter needs anyway,

and unlocks iterative solvers, optimisation loops, and EM-style training —
the largest capability-per-sieve-risk ratio on the board.

### 5.2 The soundness test (standing rule for all future extensions)

> Can the interpreter remain a total, deterministic, bounded-step oracle?

Strings fail it (I/O), graphs fail it (aliasing), recursion fails it
(nontermination). SoA conventions, tuples, bounded loops pass it. The ceiling
is not arbitrary; it is the exact shape of verification soundness. Extensions
that fail the test belong at the membrane — trusted writers, shells — forever.

## 6. Falsification experiments (kill criteria)

The direction is treated as falsifiable science. It gets revisited if any of:

1. Depth-3 composed linking fails after the `emit_call` memref-return fix.
2. Forge cannot solve ≥ half of the remaining 3DGS chain pieces (depth-sort,
   blend fold, projection wiring) within current language limits.
3. Composed-call overhead grows superlinearly with depth (would indicate even
   optimised fusion is hopeless).

All three are cheap, bounded, and runnable within a session or two. They are
deliberately scheduled FIRST so the strategy pays for its evidence before
asking for investment.

Honest tradeoff on record: naive nesting allocates intermediates per level;
deep dolls will sit well under fused implementations until the optimizer era.
Correctness-first sequencing is deliberate, and Ontic's interpreter-oracle
makes future aggressive optimisation *safe* via differential testing — a
property most compilers lack.

## 7. Revised roadmap (subtraction-first)

### Dropped or indefinitely deferred

- Program-manifest artifact + structural checker + planner stage
- Recipe control flow (loops/conditionals in `.ont` program blocks)
- End-to-end acceptance harness for glue logic
- Static shape algebra (sieve is the shape checker)

### Kept

- `ontic ask` pipeline (LLM-authored specs; confirm gate + differential
  draft agreement; pluggable spec backend) — spec authoring still needs steering
- Contracted `.hpp` outer surface — now more central under dolls
- Honest intent-limitation documentation: S4 detects candidate overfitting,
  NOT spec-intent errors; the confirm gate is the real guard

### Ordered priorities

1. `emit_call`: memref-returning vault-dep calls + depth-3 composed linking
   test *(falsification experiment 1)*
2. Forge attempts on remaining 3DGS chain pieces *(experiment 2)*
3. Mega-kernel proof: full splat chain as ONE vault entry, spec authored via
   `ask`
4. Outer-surface metadata: auto-emitted binding snippets (pyous / C++ / C)
   inside `.ous`; contracts on outermost header
5. Bounded-loop construct (grammar, checker, interp budget, both lowerers,
   differential tests)
6. Tuples when multi-output first bites
7. Optimizer passes (allocation collapse/fusion) — perf era, oracle-guarded

## 8. Limitations register (carried forward honestly)

- Naive composition performance (see §6 tradeoff)
- Spec-intent errors undetectable by sieve; confirm gate is social, not formal
- Cloud-sampled runs not byte-reproducible across providers (verdicts are;
  provenance stored)
- Single-output ABI until tuples land
- Direct LLVM emitter still lacks fold/map (MLIR required for loop kernels)

## 9. Next-stage determination

Owner to select from §7 priorities. Recommended entry point: priority 1 —
it is simultaneously the biggest unblocker, the cheapest experiment, and the
first kill-criterion test.
