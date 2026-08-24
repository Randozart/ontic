# Ontic — Research Report: Flywheel Consolidation

**Date:** 2026-08-23 (closing)
**Covers:** Consolidation stage A1–A3 + B (deeper paper run)
**Commits:** `3ab993d` (A-track), `b7d83f6` (B-track)
**Tests:** 142 behavioral, all green
**Plan reference:** `docs/plans/2026-08-23-flywheel-consolidation.md`

---

## 1. Gates

| Gate | Claim | Evidence |
|---|---|---|
| **FG1** | Reuse is measured, not anecdotal | `.ontic/vault/reuse.json` ledger; `ontic vault` prints `[reuse N]`; edges captured for both hand chains and paper-run deposits |
| **FG2** | Identity and docs match reality | IDENTITY.md refreshed (dolls, membrane, soundness test, vision verbatim); `decompose` in help + README; pyous divergence documented |
| **FG3** | Papers yield *chained* trees, not loose piles | 4-node same-run tree from the Kerbl corpus; `Splat.transmittance_step → Splat.alpha_from_sigma` use-edge; 4/4 solved; composition verified from Python (`T(1,1)=0.36788 = 1−α`) |

## 2. The bug the deeper run was designed to find

The chained node died at S2 with "requires declared `use` dependency" even
though resolution succeeded (the reuse ledger proved it). Bisection landed on
`infer_binop`: binary-expression inference used **plain leaf inference with
no dep table**, so any dep call inside arithmetic — `1.0 − alpha(σ,δ)` —
errored as undeclared regardless of declaration.

This defect had been latent since dep calls landed. Every earlier composed
kernel avoided it only because hints kept calls at statement position
(`map(v in matvec(...))`, bare fold lists). Real pipelines compose calls
*inside expressions*; the moment the decomposer authored that shape, the gap
appeared. Fix: binop inference threads `DepSigs` end-to-end; the plain path
passes an empty table so stray-call errors stay honest.

Method note: this took a full contradiction-driven debug (resolution recorded
vs checker claiming absence) resolved by instrumenting the miss site — the
initial instrumentation targeted the wrong function, which itself is recorded
as a lesson: match the debug print to the error's *origin*, not its message.

## 3. Flywheel metrics (first real numbers)

Reuse edges now on record (dep → user, hits): matvec→energy, mv2→energy,
alpha_from_sigma→{transmittance_step ×7, gate-edited variants}. Small, but
these are the first *machine-recorded* reuse facts; every future solve adds
edges automatically. P4's remaining work is presentation, not capture.

## 4. Decomposition behaviour, second look

With chaining made mandatory in the language reference, drafts moved from
1-node to 3–5 nodes with genuine internal edges. Draft disagreement remains
common (different cut boundaries) and is handled by union-at-gate with the
diff shown. One model-authored spec still needed no human arithmetic fixes —
unlike earlier sessions where my own evidence was wrong twice. Sample size:
two runs. Not a trend yet.

## 5. Carried forward

- Decomposition breadth still clusters around the Gaussian falloff family;
  projection/Jacobian/cov2d layers have not yet been emitted by the
  decomposer unprompted. Next lever is corpus richness (include section 6
  equations verbatim with numbered display math), not prompt pressure.
- Uniform sampler still cannot emit dep calls (offline composed solving).
- Bounded loops milestone is next per decision; plan doc to follow.
