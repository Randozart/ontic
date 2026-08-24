# Paper-Flywheel Track — Decomposition to Depth-N Dolls

**Date:** 2026-08-23
**Author:** Randy Smits-Schreuder Goedheijt (vision) + session agent (plan)
**Status:** Approved (session of 2026-08-23)
**Depends on:** Strategy report `docs/reports/2026-08-23-strategy-report.md`;
vault composition (M2); ask-pipeline design decisions (this session).

## 1. Vision statement

> By the end of this, once Ontic is fully functional, I should be able to
> take an advanced CS paper, dump it into a window, and it's fully
> decomposed into subfunctions, subfunctions of subfunctions, etc. until a
> program rolls out that added or utilised any number of cores to/from the
> vault. It's a self-reinforcing system.

Architecture mapping: the decomposition tree is made of **plain `.ont`
files** — the tree structure *is* the `use` graph; solve order is a
deterministic topological sort. No manifests, no recipes, no e2e harnesses
(dolls decision holds at every level). The outermost node of the tree is the
program: one kernel. Glue remains allocate → call → read.

### Self-reinforcement loop (measured, not rhetoric)

| Loop | Mechanism | Metric |
|---|---|---|
| Vault grows → solves easier | hints cite dep signatures; models pattern-match survivors | solve success rate vs vault size |
| Papers deposit reusable cores | provenance lineage | vault-entry reuse rate |
| Decomposer improves with substrate | compact signature inventory in prompt | depth-to-trivial-leaf trend |

### Standing frictions (carried honestly)

1. Wrong cut boundaries propagate down subtrees — sieve catches wrong code,
   never wrong cuts. Mitigation: one tree-level human confirm gate.
2. Bounded-retry discipline (no GPU-costume CPUs): hard budgets per node and
   per tree, recorded in provenance.
3. Per-pixel iteration over gaussian counts exceeds oracle budgets — the
   outermost doll is the single-gaussian splat contribution; pixel loops are
   consumer-side forever.

## 2. Work items

| # | Item | Gate |
|---|------|------|
| P1 | `emit_call` memref-returning dep calls (callee allocates output memref, returns SSA; private-func decls updated). Depth-3 composed gen (`examples/composed3.ont`) solving, vaulting, callable from Python | PG1: lib tests green + Python end-to-end |
| P2 | Hierarchical `ask`: `ontic decompose <paper.txt|-> [--spec-backend …] [--repair-rounds K] [--recuts N]`. Tree = directory of `.ont`; topo-sort solve order (cycle ⇒ wish error); differential draft diff; **one tree-level confirm gate** rendering node table; budgets K=2/node N=2/tree recorded in sidecars; offline `file:` backend for tests | PG2: fixture paper decomposes→solves offline without network |
| P3 | 3DGS splat chain authored exclusively via decompose+ask. Sort handling decided in-flight (in-language fixed network for tiny k vs trusted-writer preprocessing). Metrics recorded | PG3: zero hand-written specs; reuse/deposit numbers published |
| P4 | Consolidation: dep_count/reuse metrics in vault manifests; IDENTITY.md refresh (dolls, C++26 contracts, LLM spec-authorship shift, vision verbatim); strategy-report addendum with measured results | PG4: docs merged |

Deferred from earlier discussions (recorded so they stay findable): tuple
returns, bounded-loop construct, contracted `.hpp` emission, optimizer
allocation-collapse passes. Each re-enters via the soundness test when its
trigger fires.

## 3. Design notes

- **Topo-sort**: Kahn's algorithm over `use` edges among the generated files;
  deterministic tie-break by name; cycle = wish error naming the cycle.
- **Differential drafts**: two independent decomposition samples compared on
  normalized signature sets + dependency edges; mismatch surfaces as diff at
  the gate rather than silent acceptance.
- **Compact inventory**: vault listing format for decomposer prompts =
  one line per entry: `name(params) -> ret  # uses: [deps]`. Context budget
  guard: truncate with explicit `[+N more]` marker, never silently.
- **Provenance sidecars**: `<name>.ask.json` per generated file — prompt
  hash, backend, seeds, budgets consumed, draft diffs.
- **P3 sort scoping**: prefer trusted-writer preprocessing first (membrane),
  in-language sorting networks only if a chain piece genuinely needs it.

## 4. Kill criteria (inherited from strategy report)

Direction revisited if: depth-3 native linking fails after P1; forge cannot
solve ≥ half of remaining 3DGS pieces within language limits; composed-call
overhead grows superlinearly with depth.
