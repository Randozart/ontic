# Overflow Semantics — Three Tiers

**Date:** 2026-08-22
**Author:** Randy Smits-Schreuder Goedheijt
**Status:** Approved (session of 2026-08-22)
**Depends on:** `2026-08-22-step-a-mlir-toolchain-and-recipes.md` (A0–A4 done)

## 1. Decision

Speed requires declaration. The fast path never exists without a visible
contract word in the wish.

| Tier | Syntax | Interpreter | Emission |
|------|--------|-------------|----------|
| Wrapping (opt-in) | bare `wrapping` line | wraps mod 2^64, bit-exact | plain arith ops, no flags → LLVM reassociates freely |
| Checked (default) | nothing | kills candidate on overflow | bounds-check expansion → native trap (`ontic_trap`) |
| Proven (M3) | automatic | — | Z3 no-overflow proof ⇒ compiler grants flag-free codegen |

## 2. Background correction

First parity table (576 vs 377 ns/call) had TWO causes, initially
misattributed to overflow flags alone:

1. No flags on addi blocks reassociation-based unrolling (real).
2. **Constant trip count**: the C reference hardcodes N=1024 and fully
   unrolls with 4 parallel accumulators; Ontic's fold bound is a runtime
   memref dim. Constant-shape specialization is deferred to the eclipse-track
   P2 notes (autotuning-shaped work).

## 3. Work items

| # | Work | Gate |
|---|------|------|
| W0 | This doc + AGENTS.md Golden Rule 11 | — |
| W1 | `wrapping` clause end-to-end: wish parse, interp Ctx{wrapping}, sieve reporting, canonical() inclusion | wrapping wish survives sieve on i64::MAX-class values |
| W1b | Differential bit-parity test: interpreter vs native object on identical inputs (harness prints acc) | exact match or hard fail |
| W3 | Parity table rerun with `wrapping` ledger wish vs C reference; brievc protocol | ≤ C reference ns/call |
| W2 | Checked-tier native traps via sign-split bounds checks before each add/sub/mul, branch to extern `ontic_trap`; harness links abort() | native overflow-input exits nonzero; clean input unaffected |
| W5 | CHANGES.md, ISSUES.md correction entry | suite green |

## 4. Notes

- Div/mod-by-zero remain fatal errors in BOTH tiers (probe kills stand).
- canonical() change shifts vault keys — one-time churn, self-healing.
- W2 IR bloat (~4x per op) accepted: default tier is the honest-slow lane
  until M3 proofs replace checks.
