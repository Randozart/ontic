# D0–D2 — First Paper-RE Slice: Coords → Verified Transform → PLY

**Date:** 2026-08-23
**Author:** Randy Smits-Schreuder Goedheijt
**Status:** Approved (session of 2026-08-23)
**Depends on:** PR0 intrinsics (index/range); P1/P2 float+broadcast layers; pyous bridge.

## 1. Scope

Prove the reverse-engineering workflow: researcher transcribes math from a
paper (or invents a transformation), writes a `.ont` spec, forge synthesizes,
sieve verifies, Python consumes the resulting `.so` via numpy zero-copy and
writes the output file.

**Two-world doctrine enforced**: synthesis owns the pure transform between
parse and emit. Python owns file reading/writing (trusted IO world). No
kernel touches disk.

## 2. Deliverables

| # | Item | Detail |
|---|------|--------|
| D0a | `examples/coords.txt` — sample 3D point cloud (x y z per line) |
| D0b | `examples/transform.ont` — two gens: `Transform.scale_translate` (broadcast scale+translate) and `Transform.centroid` (fold-reduce) |
| D0c | pyous.py extension: List<F64> return support (read memref descriptor from ctypes struct return) |
| D1 | `examples/write_ply.py` — trusted PLY writer (Python side) |
| D2 | Live gate: forge-solve both gens → Python calls kernels with coords.txt data → writes valid PLY |

## 3. Gate

D2-GATE: `python3 examples/write_ply.py examples/coords.txt` produces a valid
PLY file whose vertex data was computed by sieve-verified native kernels.
