# D-track — Proven Data Transformation

**Date:** 2026-08-23
**Author:** Randy Smits-Schreuder Goedheijt
**Status:** Approved (session of 2026-08-23)
**Depends on:** PR0 intrinsics (index/range); P3-style text IO pulled forward.

## 1. Thesis

Ontic becomes a **proven data-transformation engine**: give input/output
example FILES plus a spec, receive a synthesized+verified kernel between
trusted parse/emit machinery, with byte-parity against reference outputs as
reproduction evidence. Target use: custom generators over user-defined data
models (first: 3D mesh formats).

## 2. Two-world doctrine

- **Sievable kernel world** (synthesized, probed, lowered): pure transforms
  over structured values — flat lists per SOA/CSR conventions.
- **Trusted IO world** (Rust stdlib, verified once vs reference files,
  never lowered): read_lines/parse_f64s/write_ply_ascii/write_obj/write_csv.

File evidence desugars at PARSE time into values (`=> @"in.txt" ->
@expected.ply`), so probes stay pure and the sieve never touches disk.

## 3. User data models

Flat SOA/CSR conventions over existing List types:
- vertices: interleaved stride-3 `List<F64>` (x,y,z,...)
- faces: CSR — flat index `List<Int>` + offsets `List<Int>`
No nested lists, no structs until flat conventions demonstrably hurt.

## 4. Phases

| Phase | Delivers | Gate |
|-------|----------|------|
| D0 | Trusted text intrinsics: `read_lines`, `parse_f64s`; file-evidence syntax desugaring | evidence roundtrip test |
| D1 | Trusted writers verified vs reference files: CSV, PLY ASCII, OBJ | byte-parity fixtures |
| D2 | Vertical slice: coords.txt → valid PLY via synthesized kernel | end-to-end parity |
| D3 | SOA/CSR mesh pattern documented; parametric generator wishes on top | generator wish survives sieve |

Binary formats deferred.

## 5. Sequencing

After M2 polish + H/E effects. PR0 (index/range) lands inside D0 since every
mesh kernel needs it.
