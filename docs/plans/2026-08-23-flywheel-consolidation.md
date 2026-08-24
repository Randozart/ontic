# Flywheel Consolidation Stage

**Date:** 2026-08-23
**Author:** Randy Smits-Schreuder Goedheijt + session agent
**Status:** Approved (session of 2026-08-23)
**Depends on:** Paper-flywheel plan P1–P3 (complete); strategy report.

## 1. Scope

Close the flywheel loop formally (P4) and stress it with a deeper paper run,
before any new language surface lands. Bounded loops follow as a dedicated
milestone immediately after this stage.

## 2. Work items

| # | Item | Gate |
|---|------|------|
| A1 | Reuse ledger: append-only `.ontic/vault/reuse.json` keyed by `(dep_key, used_by_key)`, incremented in `resolve_deps`; `vault ls` REUSE column | FG1: counts appear; deterministic given op sequence |
| A2 | IDENTITY.md refresh: dolls direction, C++26 membrane, LLM spec-authorship behind gate, vision statement verbatim, soundness test as standing rule | FG2 |
| A3 | Hygiene: help text lists `decompose`; README paper-pipeline section; pyous_pkg drift audited | FG2 |
| B1 | Deeper Kerbl-corpus run through decompose+ask. Success bar: ≥2-level tree where at least one node depends on a same-run deposit. No prompt-laundering: shallow output is recorded signal, not hidden | FG3 |
| B2 | Deposits verified from Python; metrics via A1 ledger; research report | FG3 |

## 3. Non-goals

Bounded loops (next milestone), genrand dep-calls, tuples, contracted .hpp,
optimizer passes — all parked by earlier decisions.

## 4. Risk carried openly

If cov2d-style composition proves inexpressible today (shape threading across
2×2 chains), that evidence promotes tuples/shape work up the queue. The stage
is designed to surface this cheaply.
