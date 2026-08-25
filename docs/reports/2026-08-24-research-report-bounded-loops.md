# Ontic — Research Report: Bounded Loops (`until`)

**Date:** 2026-08-24
**Covers:** Bounded-loops milestone L1–L8
**Commits:** `576691f`
**Tests:** 147 behavioral, all green
**Plan reference:** `docs/plans/2026-08-24-bounded-loops.md`

---

## 1. The construct

```
fold k in range(MAX), acc from INIT { STEP } until DONE
```

Pre-test early exit inside a decidable budget: `until` evaluates on the
current (iteration index, accumulator) BEFORE each step; stops on true.
Zero iterations when INIT already satisfies DONE; full budget otherwise;
result is the surviving accumulator. Termination remains provable — the
counter is machine-owned and range-bounded, exactly per the soundness test.

Design deviation recorded: strategy report sketched `break` statements; this
milestone shipped an `until` clause instead. Same capability (data-dependent
exit for solvers), strictly less machinery — no effect-typing of diverging
branches, and `scf.for`'s missing break stops mattering because until-folds
lower to `scf.while`.

## 2. Proof chain

| Gate | Evidence |
|---|---|
| Oracle semantics | unit tests: zero-iteration, mid-loop exit, full-budget paths |
| Parser/roundtrip | until parses; display re-parses byte-identical |
| Native parity | hand-solved Newt.sqrt through probes (interp) + mlir-opt + llc + native bench |
| Forge discovery | gemini authored `until abs(g*g − x) < 1e-7` from spec alone; vaulted same key as hand solution (`b169c40d`) |

## 3. The measured payoff

Native ranking of survivors on the same gen:

| Strategy | ns/call |
|---|---|
| Full-budget folds (8–64 fixed iterations) | 18 706 – 20 961 |
| **Until-fold (early exit at convergence)** | **2 915** |

≈ **6× speedup**, discovered by the model, proven by the sieve, measured by
the native bench — the first time Ontic's optimizer story showed up *through
specification* rather than compiler passes.

## 4. Prompt-surface lessons

Model candidates ignored `until` while the forge prompt's own LANGUAGE
section omitted it — conflicting guidance loses to the system prompt every
time. After adding the clause to both the forge language block and
`ask_langref.txt`, candidates copied it verbatim. Rule of thumb confirmed:
a new construct exists for the model only when EVERY prompt surface teaches
it.

## 5. Scope honesty

Direct-LLVM emitter rejects until-folds with a clear error instead of
risking wrong machine code (its fold phi is i64-hardcoded; MLIR pipeline is
the validated route). General `break` anywhere, unbounded while, recursion:
still membrane residents, by the soundness test.

## 6. Corpus note

`ONTIC_COLLECT=1` captured the until-kernel solves automatically — the
training set now teaches future fine-tunes both the construct and the
early-exit performance pattern, closing the loop between language growth and
self-distillation.

## 7. Queue

Tuples (multi-output) when a paper needs them · contracted `.hpp` emission ·
genrand dep-calls for offline composed tests · optimizer passes (now with a
concrete target: collapse until-fold overhead).
