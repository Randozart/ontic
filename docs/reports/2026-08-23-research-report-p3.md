# Ontic — Research Report: P3 (Paper-Flywheel Proof)

**Date:** 2026-08-23 (closing session)
**Covers:** Paper-flywheel plan item P3 — 3DGS corpus through the full pipeline
**Commits:** `caf95ee` (+ hardening landed en route)
**Tests:** 142 behavioral, all green
**Plan reference:** `docs/plans/2026-08-23-paper-flywheel.md` §2 P3

---

## 1. What was done

The real corpus was used, not a fixture: Kerbl et al. 2023 (*3D Gaussian
Splatting for Real-Time Radiance Field Rendering*, ACM TOG 42(4)) fetched
from arXiv HTML; sections 4 and 6 extracted verbatim (~6k chars) and piped
to `ontic decompose --spec-backend gemini`. Every downstream step ran as
shipped: differential drafts, union gate, topological solve, budgeted repair.

## 2. Results

| Pass | Outcome | Deposit / Reuse |
|---|---|---|
| 1 | 2/2 solved | **+2**: `Splat.gaussian_3d` (paper eq. 4 — G(x)=exp(−½·quadratic form)), `Splat.alpha_eval` (volumetric α term from eq. 1) |
| 2 | 1/1 solved | **+1 reuse**: decomposer declared `use Splat.gaussian_3d`, building on the deposit from pass 1 |

Both deposits verified from Python through their native `.so`
(`gaussian_3d(0)=1.0`; `alpha_eval(1,1)=0.63212…`). Run artifacts archived at
`examples/paper_runs/gs_kerbl2023/`.

**The self-reinforcement claim now has data:** vault grows → next
decomposition cites the new cores by full path. Zero hand-written specs were
involved at any point.

## 3. Model behaviour worth recording

- **Contract vocabulary adopted spontaneously**: pass-1 specs included
  postconditions over `res` (`res > 0.0 && res <= 1.0`) — learned from the
  language reference alone.
- **Hints lie; evidence decides**: pass-2's model-authored hint said
  `sqrt(%quad)`, mathematically inconsistent with its own examples. The
  vaulted kernel is correct anyway because S3–S5 check evidence, not hints.
  Golden Rule 12 validated in the wild.
- Drafts disagreed across passes (different cut boundaries); the union-gate
  presented the diff rather than hiding it — per design.

## 4. Pipeline hardening forced by reality

P3 on a real corpus exposed four spec-synthesis defects that fixtures never
would have:

1. Gemini candidate `responseSchema` squeezed free-form spec drafts into
   candidate-shaped JSON → added `gemini_body_free`.
2. `gemini_parse` deep-decodes schema JSON → added text-only variant.
3. `extract_candidate` invents sketch scaffolding (`fn @…`) for non-candidate
   text → spec path uses fence-stripping only.
4. One malformed file killed whole drafts → lenient per-node validation with
   dropped-file reporting, plus draft union at the gate.

Also relaxed: dotted filename stems allowed (namespace-style), path traversal
still rejected; stray `=== end ===` tolerated when no block is open.

## 5. Sort scoping decision (as promised in plan)

Depth-sorting gaussians stays **consumer-side** (trusted writer / host loop):
per-pixel iteration and radix sort exceed oracle step budgets by design, and
the paper itself treats sort as infrastructure around the splat math. The
in-language dolls cover every *equation*; the shell covers scheduling.

## 6. Limitations carried forward

- Decomposition breadth still skews conservative (pass-1 produced 2 nodes
  where the section supports ~4). Langref now nudges 3–6 files; more runs
  needed to see whether this holds.
- Differential agreement cannot catch shared-wrong-math drafts; the human
  gate remains the intent authority.
- Repair loop exercised only lightly this run (drafts validated first try);
  its budget accounting remains tested offline via P2's uniform path.

## 7. Status against the flywheel plan

| Item | State |
|---|---|
| P1 depth-3 composition | ✅ PG1 |
| P2 hierarchical ask | ✅ PG2 |
| **P3 3DGS via pipeline only** | ✅ PG3 (this report) |
| P4 metrics + IDENTITY refresh | open — reuse counters are manual today; formalize in manifests next |
