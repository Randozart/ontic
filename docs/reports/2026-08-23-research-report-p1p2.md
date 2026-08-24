# Ontic — Research Report: P1–P2 (Deep Composition & Spec Synthesis)

**Date:** 2026-08-23 (late session)
**Covers:** Paper-flywheel plan items P1 and P2, executed back-to-back
**Commits:** `86fc846` (P1), `e59178c` (P2)
**Tests:** 142 behavioral, all green (up from 134)
**Plan reference:** `docs/plans/2026-08-23-paper-flywheel.md`

---

## 1. Headline results

| Gate | Claim | Evidence |
|---|---|---|
| **PG1** | Native composition works at depth ≥3, list-returning intermediates included | `Chain.energy → Chain.mv2 → Linalg.matvec` vaulted with header+`.so`, verified from Python (`58.0` / `13.0` / `[9 49]`) |
| **PG2** | Paper text → spec tree → solved kernels, fully offline | Fixture paper decomposed via `file:` backend, differential agreement, gate skipped (`--yes`), topo-solved **2/2** via uniform enumeration, exit 0 |

Kill criterion 1 (depth-3 linking fails) is **retired by evidence**, not
assumption.

## 2. Bugs the gates caught this stretch

Every failure was a real defect; none was a false accept. In discovery order:

1. **emit_call F64-only returns** — list-returning dep calls died at
   lowering ("only F64-returning dep calls supported"). The single hard
   blocker for nesting dolls; fixed by hoisting param typing and adding
   memref-return arms.
2. **emit_map unscoped out-type** — output element type was inferred before
   the loop variable existed in scope, so bodies like `v * v` allocated
   `memref<?xi64>` then stored f64 products. Fixed with a pre-bound probe
   type environment. Same *family* as the ListCons literal-heuristic bug
   fixed earlier: **type decisions computed at the wrong scope point**.
3. **compose_modules duplicate private decls** — flat-closure modules each
   declare `ontic_trap`; concatenation produced redefinitions that mlir-opt
   correctly rejected. Dedupe keeps first occurrence.
4. **mlir-translate input==output invocation** — translate ran with
   `-o <same-path-as-input>`, a lazy-truncate segfault lottery that had
   silently worked on small modules. Staged via temp file + rename.
5. **Double-square in my own hint text** — hint said `acc + v * v` over an
   already-squared dependency; S3 killed every candidate with
   `got 2482, expected 58` until the spec author (me) fixed his arithmetic.
6. **Short-name vs full-path dep calls** — hints must use full declared
   paths (`Chain.mv2`, not `mv2`); models copy hints literally.
7. **Repair budget burned on candidate-side failures** — "no candidate
   survived the sieve" is a sampler fact, not a spec defect; spec repair
   cannot help. Decompose now classifies failures and skips repair
   accordingly.

Pattern across the session: the deterministic side keeps auditing everyone —
the model, the agent, and the toolchain glue.

## 3. What P2 built

- `forge::sample_text`: raw-prompt completion primitive (cloud + llama;
  uniform refuses by construction — enumeration cannot author specs).
- `src/ask.rs`: tree parser (traversal-safe filenames), node validation
  (parse + wish gate), Kahn topological order with named-cycle errors,
  context-bounded inventory blocks, normalized draft diff.
- `src/ask_langref.txt`: contract-authoring language reference encoding the
  project's accumulated lessons (explicit shape params, exact-grammar
  hints, arithmetically-correct examples, full-path calls).
- `ontic decompose`: differential drafts (bounded resamples), ONE
  tree-level confirm gate, `.ask.json` provenance sidecars (backend, seed,
  budgets, prompt hash), leaves-first solving through the proven CLI path,
  per-node repair loop with hard budgets.

## 4. Honest limitations discovered

- **Uniform enumeration never emits dep-call candidates.** Composed specs
  solve offline only when a candidate can satisfy them without calling its
  declared deps (legal: the sieve does not require dep usage). True offline
  composed solving needs genrand extension — deferred, recorded.
- Differential agreement currently compares signatures/dep-edges/examples
  counts; it cannot catch two drafts agreeing on the same wrong math. The
  confirm gate remains the intent guard (documented in strategy report §B).
- Repair prompts reuse the whole original prompt + failure tail; token cost
  per repair grows linearly. Fine at K≤2; revisit if budgets rise.
- `quad` needed 400 uniform samples where gemini needed ~2 candidates.
  Sampler efficiency gap is expected and now measurable via
  `--candidate-samples`.

## 5. Next: P3

Run the 3DGS chain exclusively through decompose+ask with the real paper
text: projection wiring, cov2d, alpha/conic already vaulted as reusable
cores the decomposer should cite. Sort scoping decision lands here.
Success metric: zero hand-written specs; deposit/reuse metrics recorded.
