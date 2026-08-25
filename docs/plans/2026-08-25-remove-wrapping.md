# Remove `wrapping` Tier

**Date:** 2026-08-25
**Status:** executing

## Decision

Remove the declared `wrapping` overflow tier from the language entirely.
All arithmetic is checked (default honest tier). The future `proven` tier
(M3) is unaffected.

## Rationale

Wrapping was kept as a declared fast path per the 2026-08-23 strategy
report, but no gen in practice needs mod-2^64 semantics; the tier doubles
the interp/lower surface and every example carried a boilerplate line.
Checked arithmetic is the honest default; speed work continues via the
proven tier when it lands.

## Blast radius

| File | Change |
|------|--------|
| src/gen.rs | drop `Gen.wrapping`, parse line, canonical() term |
| src/interp.rs | drop `Tier.wrapping`; checked arms only |
| src/lower.rs | drop `Emitter.wrapping`; always emit checked arith |
| src/lower_llvm.rs | drop wrapping branches |
| src/sieve.rs | drop tier-aware plumbing |
| src/vault.rs / src/ous.rs | stop writing manifest key; loader ignores old keys |
| src/program.rs, src/pipeline.rs | drop emission args |
| src/main.rs | drop CLI plumbing |
| src/ask.rs, src/forge.rs, src/genrand.rs | drop prompt/grammar mentions |
| src/recipe.rs | stop attaching prefix lines |
| src/ask_langref.txt | rewrite language reference |
| examples/*.ont | strip lines; delete ledger-wrapping.ont |

## Compatibility

- Vault keys change for re-solved gens (canonical text loses the line).
- Old `.ous` manifests carrying `"wrapping": bool` still load (manual JSON
  parse tolerates unknown keys).
- Old vault .so binaries remain valid; their semantics were bit-exact
  under both tiers.

## Verification

1. `cargo build --release` clean
2. `cargo test --lib` green
3. `ontic check examples/matvec.ont` + end-to-end solve re-run
4. grep: zero tier references outside sha256/rng internals
