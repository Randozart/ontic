# Ontic — Identity

**Adopted:** 2026-08-23

## The one-liner

> **Ontic is a domain-specific language whose products are verified native
> libraries.**

You write specifications (`.ont`); a local transformer proposes
implementations; a deterministic seven-stage sieve proves them against your
evidence; the output is a shared library and C header that Python, C, Rust,
Julia, or anything with an FFI can consume. The transformer proposes; it
never decides what is true.

## What Ontic is not

- **Not an application language.** No strings-in-sketch, no recursion, no
  general control flow, no kernel-side I/O. The sketch language is the
  intermediate representation of verified numerics — its smallness is the
  feature.
- **Not an agent.** The transformer is a fixed-function candidate
  accelerator (the "programming GPU"). One bounded refinement round per
  solve; unbounded retries would be a CPU wearing a GPU costume.
- **Not a Numba/PyPy competitor.** They make Python code fast; Ontic makes
  specifications into libraries that Python *hosts*. Python is the
  consumer/choreography layer (see `examples/pyous.py`).

## The product surface

| Artifact | Role |
|---|---|
| `.so` / `.dll` | Verified machine code, Flat-MemRef ABI |
| `.h` | Generated C header (`extern "C"`, include-guarded) |
| `.ous` | Single-file distribution bundle (manifest + sketch + MLIR + object + header) |
| `.ont` | The editable source of truth — wishes→gens: signatures, invariants, evidence |

## Division of labour

| Layer | Who writes it | Verification |
|---|---|---|
| Gen spec (.ont) | Human | sieve S1–S7 judges candidates against it |
| Sketch candidates | Transformer (GBNF-constrained) | never trusted |
| Recipes | Human | deterministic driver-code generation |
| Trusted intrinsics | Rust stdlib | verified once vs reference fixtures |

## Open design questions

- ABI stability across re-solves: key-suffixed filenames today; semver'd
  bundles or stable-name+manifest resolution needed before any registry.
- Multi-architecture `.ous` variants (AVX/NEON targets).
