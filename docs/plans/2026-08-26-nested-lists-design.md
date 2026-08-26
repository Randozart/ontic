# Nested Lists Design — 2026-08-26

**Status:** design review requested
**Finding up front:** the capability report's "L effort" estimate assumed
new container types. The existing language already has `++` concatenation
at every layer (parse/check/interp/lower), and the probe solver proves
`len == n*n` shape relations. The 2D-DP blocker reduces to two small
gaps — no list-valued iteration construct, no 2D indexing sugar.

## The actual blocker

DP tables need to BUILD a flat table incrementally while READING earlier
cells. Today a candidate can read (`index`) and concatenate (`++`) but
cannot emit a variable-length list per iteration step: `map` bodies must
be scalar-typed, so there is no way to grow a table by rows inside one
expression.

## Options

### Option A — `flatmap` builtin (effort: **S**)

```
flatmap(i in range(%n)) { <list-valued body> }   // :: List<T>
```

Semantics: map whose body evaluates to a `List<T>`; results concatenated
in order into one flat `List<T>`. Oracle first: interp evaluates by
concatenating component lists; MLIR lowers as an alloc of total length +
per-row copy loop (same shape as emit_concat's machinery).

Unblocks: row-wise DP construction —

```
fold i in range(%n), %acc from %init {
  %acc ++ flatmap(j in range(%m)) { <update using index(%acc, ...)> }
}
```

Reading earlier cells through `%acc` works because it grows monotonically.

### Option B — `index2` sugar (effort: **S**, rides along)

```
index2(%t, %i, %j, %stride)   ≡   index(%t, %i * %stride + %j)
```

Bounds check on BOTH dimensions (native traps exactly like the oracle).
Pure readability + double-bounds-check; zero ABI impact.

### Option C — true nested types `List<List<T>>` (effort: **L**; not recommended)

New Ty variant, nested memrefs (`memref<?x?xT>` or vector-of-descriptors),
6+-field C ABI expansion, shim/header/probe/solver surface, oracle Value
nesting. Buys nothing the membrane cannot route: heavy matrix cores live
in the vault as flat kernels today and compose fine. Revisit only if an
external consumer demands nested FFI types.

## Recommendation

Implement A + B as one leg (~S–M total): two builtins through the full
seven-stage pipeline, oracle-first, honest native rejection until MLIR
lands, then differential tests including a Levenshtein-style spec that
the capability report marks structurally blocked — reclassifying it.

## Blast radius (A+B)

| Layer | Change |
|-------|--------|
| sketch.rs | Builtin::FlatMap parse (+ Index2 as Builtin3), GRAMMAR |
| check.rs | body-type rules: FlatMap body :: List<T>, result :: List<T>; Index2 arg typing |
| interp.rs | oracle eval both |
| lower.rs | alloc-total + row-copy emission; index2 → two checked loads' worth of guard + offset math |
| probes/gen | nothing (List params unchanged) |
| lint | nothing |

## Acceptance

1. Oracle/native bit-parity on flatmap kernels.
2. Levenshtein edit-distance spec passes S1–S7 end-to-end.
3. Capability report Gap 3 reclassified.
