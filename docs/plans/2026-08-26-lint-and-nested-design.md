# Spec-Lint (`ontic lint`) + Nested-Lists Design

**Date:** 2026-08-26
**Status:** executing

## Leg A — `ontic lint` (S-M)

Static spec-quality pass run BEFORE forge spend. The sieve proves what
specs say; lint raises what they probably didn't mean. Advisory except
where marked ERR.

### Checks

| Rule | Severity | Logic |
|------|----------|-------|
| `unsat-invariants` | **ERR** | input-side integer skeleton has NO solution (reuses `probes_solver`; NoSolution ⇒ contradictory constraints like `%n >= 5 && %n <= 4`) |
| `zero-tol-float` | WARN | F64/F32 example output with `± 0` — exact float equality is brittle across tiers |
| `loose-tol` | WARN | tolerance > 1.0 — evidence weaker than typical arithmetic error |
| `thin-evidence` | WARN | fewer than 2 transparent examples |
| `no-opaque` | WARN | 0 opaque examples and no explicit splits — overfit gate S6 weakened |
| `hint-unparseable` | INFO | hint text does not parse as a candidate expression (advisory per GR12; machine-readable hints improve forge success) |
| `postcondition-guarded-note` | INFO | res-referencing invariants present: guarded twin enforces preconditions only |
| `duplicate-vault-path` | INFO | same gen path solved under multiple keys (find_by_path prefers verifiable manifests) |

Exit code: 0 clean / findings-only-warnings, 1 when any ERR.

### Surface

```
ontic lint <file.ont> [--all-gens] 
```
Output lines: `SEVERITY [rule] path: detail`.

### Tests

Unit tests per rule on synthetic gens (contradictory intervals → ERR;
zero-tol float → WARN; healthy spec → no findings). Solver-backed check
skips cleanly when skeleton Unsupported.

## Leg B — nested lists design doc (no code)

`docs/plans/2026-08-26-nested-lists-design.md`: evaluates true 2D
memrefs vs flat+stride composite vs shape-carrying lists; recommends one;
lays out ABI/probe/sieve blast radius. Implementation stays out of scope
until the design is reviewed.
