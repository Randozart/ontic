# K-track — Kernel Artifacts & FFI

**Date:** 2026-08-23
**Author:** Randy Smits-Schreuder Goedheijt
**Status:** Approved (session of 2026-08-23)
**Depends on:** M2 composite emission; native pipeline.

## 1. Vision alignment

Ontic as a **programming GPU**: the transformer is a fixed-function
candidate accelerator (GBNF ISA, prompt prefill); the sieve is the
error-discard logic; the vault accumulates verified kernels as immutable
artifacts.

- Feedback round = branch misprediction handling: speculative execution,
  structured mispredict signal, one bounded resubmission with state folded
  in. Unbounded retries would be an agent wearing a GPU costume.
- **Editability model (Golden Rule 15): kernels are artifacts; humans edit
  wishes and recipes.** Quick-change workflow: edit wish → re-solve → new
  key; old kernel versions coexist harmlessly in the vault.
- `.ous` reserved as the single-file bundle format name (`OUS1` magic);
  directory-per-kernel ships first.

## 2. Work items

| # | Item | Gate |
|---|------|------|
| K0 | This doc + AGENTS.md Golden Rule 15 | — |
| K1 | `lower::emit_header(name,params,ret)` — C header from signature using the flat-MemRef ABI (`<p>_a/_b/_o/_s/_st` per list arg; scalars plain). Deterministic: no timestamps. Return type `long` for Int/Bool, `double` for F64 | snapshot unit tests |
| K2 | Auto-artifacts per solve: `emit_and_store` writes `<name>.h` and links `lib<name>-<key8>.so` from composite objects (cc -shared over cand.o + dep .o files). Manifest records `header`/`lib` paths | KG1 |
| K3 | `ontic lib ls / promote <Path> / demote <Path>` managing `.ontic/lib.manifest`; `vault ls` shows LIB badge for promoted entries | KG2 |
| K4 | FFI proof: gated C-caller test (includes generated header, links .so, asserts result) + `examples/ffi_demo.py` ctypes script | KG3 |

`.ous` single-file bundling deferred to next stretch (magic OUS1 reserved).

## 3. ABI reference (flat expansion, verified earlier)

Each List<T> argument expands to five scalars:
`(void* <n>_a, void* <n>_b, long <n>_o, long <n>_s, long <n>_st)`.
Int/Bool scalars pass as `long`; F64 scalars as `double`.
