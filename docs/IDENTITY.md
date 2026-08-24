# Ontic — Identity

**Adopted:** 2026-08-23 · **Revised:** 2026-08-23 (flywheel consolidation)

## The one-liner

> **Ontic is a domain-specific language whose products are verified native
> libraries — composed like Russian dolls to any depth.**

You write specifications (`.ont`); a transformer proposes implementations; a
deterministic seven-stage sieve proves them against your evidence; the output
is a shared library and header any FFI consumer can bind. Verified kernels
compose via vault dependencies, so papers become trees of specs and the tree
becomes one outermost kernel. The transformer proposes; it never decides what
is true. Trust scales with sieve strength, not model strength.

## Vision statement (owner, verbatim)

> By the end of this, once Ontic is fully functional, I should be able to
> take an advanced CS paper, dump it into a window, and it's fully decomposed
> into subfunctions, subfunctions of subfunctions, etc. until a program rolls
> out that added or utilised any number of cores to/from the vault. It's a
> self-reinforcing system, in that regard.

Demonstrated 2026-08-23 on the Kerbl et al. 3DGS corpus: paper text → spec
tree → solved deposits → next pass citing them (`examples/paper_runs/`,
research reports P1–P3).

## What Ontic is not

- **Not an application language.** No strings-in-sketch, no recursion, no
  unbounded control flow, no kernel-side I/O inside gens. Smallness is the
  feature; the soundness test governs every extension: *the interpreter must
  remain a total, deterministic, bounded-step oracle.*
- **Not an agent.** The transformer is a fixed-function candidate
  accelerator. One bounded refinement round per solve; bounded budgets also
  govern spec synthesis (`--repair-rounds`, `--recuts`). Unbounded retries
  would be a CPU wearing a GPU costume.
- **Not a Numba/PyPy competitor.** They make Python fast; Ontic turns
  specifications into libraries Python hosts.
- **Not an LLM-program generator.** Assembly is Russian-doll composition,
  not glue code. Glue shrinks to allocate → call → read — simple enough for
  any consumer, human or model.

## Division of labour

| Layer | Who writes it | Verification |
|---|---|---|
| Gen spec (.ont) | Human, or LLM behind `ontic decompose`'s confirm gate | sieve S1–S7 judges candidates against it |
| Spec trees from papers | Decomposer LLM (differential drafts + union gate + budgets) | every node passes parse + wish gate before solving |
| Sketch candidates | Transformer (GBNF-constrained / schema-constrained) | never trusted |
| Recipes | Human | deterministic driver-code generation |
| Trusted intrinsics & writers | Rust stdlib | verified once vs reference fixtures |
| Membrane code (shells, sorts, pixel loops) | Consumer languages | C++26 contracts emitted from `[pre][post]`; violations are deterministic failures |

## Product surface

| Artifact | Role |
|---|---|
| `.so` / `.dll` | Verified machine code, Flat-MemRef ABI, depth-N composites linked |
| `.h` | Generated C header (`extern "C"`, include-guarded); contracted `.hpp` planned |
| `.ous` | Single-file distribution bundle (manifest + sketch + IR + object + header) |
| `.ont` | Editable source of truth — signatures, invariants, evidence |
| `reuse.json` | Append-only flywheel ledger: which cores fed which solves |

## Standing rules

1. THE WALL: model output enters only as text through deterministic gates.
2. Every extension passes the soundness test (see above).
3. Contracts bind their authors too — the sieve has repeatedly caught its
   own operators' arithmetic (see research reports).
4. Hints are advice; evidence is truth.
