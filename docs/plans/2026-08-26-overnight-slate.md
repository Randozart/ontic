# Overnight Slate — 2026-08-26

**Status:** executing autonomously. Every leg commits only behind a green
`cargo test --lib`. No pushes. No cloud samplers. CHANGES.md entry per leg.
THE WALL untouched throughout — sieve-protected changes only.

## Leg 1 — vault surface completion

- `ontic vault --json`: machine-readable listing `{key, name, path,
  signature, trust, reuse}`.
- `ontic vault status <name>`: per-version trust + artifact inventory
  (.so / .guarded.so / .h / .hpp / .ous presence).
- Unit tests against a temp vault fixture.

## Leg 2 — imported kernels callable

- `land_entry` build step: re-lower shipped MLIR → obj → `cc -shared`
  when toolchain present; else link shipped `.o` via cc directly.
- Guarded twin rebuilt from shipped `.guarded.c` shim when present.
- Build failure = loud warning, landing still completes (attested).
- E2E gate: export mean → scratch import → ctypes call returns 3.0 for [2,4].

## Leg 3 — capability scorecard re-run

- Re-probe the 23 algorithm classes post-F32 + probe-solver.
- Dated addendum table in `docs/reports/2026-08-24-capability-boundary.md`
  (class | before | after | driver).

## Leg 4 — hygiene sweep

- Dead code out (`format_args_types`, `n_of`); unused params/imports/mut fixed.
- `find_by_path` prefers gen_text-carrying manifests then latest key;
  regression test for the stale-pick case.

## Leg 5 — tuple types (stretch; hard-gated)

- Vertical slice: `Ty::Tuple(Vec<Ty>)`, parser, checker component rules,
  interp `Value::Tuple`, MLIR multi-result returns, C ABI via pointer
  out-params, probes edges, honest rejects in lower_llvm.
- **Gate:** suite green AND a QR-style two-list-return spec solves +
  vaults end-to-end. Fallback: design doc at
  `docs/plans/2026-08-26-tuples-design.md`.

## Morning report

CHANGES.md entries + final session summary: legs landed with hashes,
scorecard deltas, tuple verdict.
