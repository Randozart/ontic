# H — Author Hints & E — Recipe Effects

**Date:** 2026-08-23
**Author:** Randy Smits-Schreuder Goedheijt
**Status:** Approved (session of 2026-08-23)

## H — hints

`hint "free text"` metadata lines on wishes. Quarantined channel:

- Flow ONLY into the forge prompt (`=== AUTHOR GUIDANCE ===`).
- Excluded from `canonical()` — same contract ⇒ same vault key regardless
  of hints.
- Shown as advisory in `ontic check`.
- A lying hint wastes samples; the sieve still decides everything.

Rationale: resolves the rms-class tension where task guidance was impossible
without violating THE WALL. Hints shape generation; judgment stays
deterministic.

## E — effects

Recipe-level statements over locals, compiled to deterministic driver C:

```
write %r -> "out/result.csv"     # scalar = 1 row; list = column
dump  %r -> "result.json"        # {"name": value} scalars/lists
log   "meansqdev done: %r"       # console with %var interpolation
```

- Typecheck: target local exists (any scalar/list kind).
- Byte-deterministic given same inputs.
- ZERO sieve impact — effects execute at driver runtime only.
- Wishes stay 100% pure; no caller-inherited hidden IO ever.

Reading data (`%xs = data "f.csv"`) remains P3 scope.

## Golden rules added

12. **Hints are advice, never evidence.**
13. **Effects live in recipes, never wishes.**

## Gates

| # | Gate |
|---|------|
| HG1 | hint parse/canonical-stability/prompt-inclusion tests green |
| HG2 | rms.ont retried with hints + composition |
| EG1 | recipe effect statements parse/typecheck/codegen |
| EG2 | gated e2e writes expected CSV/JSON/log content to temp dir |
