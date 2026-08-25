# `.nous` — Vault Export Package

**Native Ontic Unified Store** — νοῦς, the intellect that grasps being.
A `.nous` file is an exportable slice of the vault: many verified
kernels, their dependency graph, guarded twins, and enough spec text to
re-verify them on another machine.

## Format

```
NOUS1\n
[u64 LE len][TOC JSON]
[u64 LE len][OUS1 blob] × N      verbatim ous::pack_full per kernel
[u64 LE len][EXTRA bytes] × K    manifest / guarded twins / hpp
```

TOC (JSON): `format: "nous1"`, `generator`, `created_unix`, `target`
(advisory arch-os tag), `entries[]` with `key`, `name`, `signature`,
`quality` (`full` | `edges_only` | `unknown`), `verifiable`,
`guarded`, and per-entry `extras[]` kinds.

Extras are declared in TOC order and stored flat after all kernels:

| kind        | content                                   |
|-------------|-------------------------------------------|
| `manifest`  | full vault manifest JSON (provenance etc.) |
| `guarded_so`| guarded shared object                      |
| `guarded_c` | guard shim source                          |
| `hpp`       | contracted C++ header                      |

The container guarantees integrity only. It carries no verdicts.

## Trust model

Imported kernels land **attested**: structurally intact, but their bytes
came from someone else's machine. Promote to **verified** with
`--verify`, which re-runs the deterministic sieve locally:

1. canonical-key check — the shipped gen text must hash to the package's
   claimed key (catches any tampering with spec or examples);
2. full S1–S7 re-run with the shipped candidate as a hand solution;
   declared deps resolve against the local vault (import lands
   dependencies first).

Determinism ⇒ same verdicts on any machine. A kernel is callable either
way; trust status only records who vouches for it. Local solves always
write `verified`. The ledger lives in `<vault>/trust.json` (local-only,
never shipped) and shows in `ontic vault`.

## Commands

```bash
# export one kernel (+ its dep chain), or everything
ontic vault export Stats.mean --out mean.nous
ontic vault export --all --out full.nous

# inspect without landing anything
ontic vault import full.nous --dry-run

# land as attested; collisions skipped unless --force
ontic vault import mean.nous

# land only what this machine's sieve can re-prove
ontic vault import mean.nous --verify

# status column in listing
ontic vault
```

Export closes dependencies automatically from each gen's `use` lines
(topological order). Entries whose manifests predate `gen_text` export
with `verifiable: false` — they can be imported attested but never
verified.

## Compatibility notes

- Packages are forward-incompatible by design: `format: "nous1"` must
  match exactly.
- Object bytes are host-target builds; consumers with an MLIR toolchain
  can re-lower from the embedded IR instead of trusting shipped objects.
- Import never writes to the training corpus (ONTIC_COLLECT feedstock is
  unaffected).
