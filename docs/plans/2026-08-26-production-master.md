# Ontic Production Readiness — Master Plan

**Date:** 2026-08-26
**Status:** COMPLETE — all 8 phases landed 2026-08-26 (commits
8674659, 6b6b94e, 0794f75, b950b74, ee0a7d0, 3085c8e; P5 regression run,
P8 sweep). See CHANGES.md entry for the phase map.
**Scope:** everything discussed this session. No deferrals, no skips.
Every item listed here ships functional, tested, and documented by the
final commit of this run.

## Exit criteria (definition of "production ready")

1. **No vacuous verification** — every vaulted kernel passed differential
   *value* parity against the interpreter oracle. No skipped shapes, no
   empty-buffer drivers.
2. **Forge vocabulary current** — `ask_langref.txt` documents every
   language feature; GRAMMAR↔parser parity is mechanically enforced;
   example specs exercise each capability.
3. **Trustworthy vault** — broken/orphaned entries detectable
   (`vault doctor`) and removable (`vault rm`, `vault gc --orphans`);
   debug residue purged.
4. **Truthful documentation** — capability report, README, IDENTITY.md
   match shipped reality.
5. **Flywheel proof** — one fresh `ontic decompose` run through the new
   vocabulary, topo-solved through the parity gate.
6. **Strings functional** — `Str` type with `str_len`/`str_eq` through all
   seven stages, native parity enforced.
7. **Proven tier functional** — z3-backed overflow-absence proofs emit
   flag-free code for provably-safe kernels; feature-gated build keeps
   default installs toolchain-light; checked tier remains the universal
   fallback.
8. **Hardening sweep clean** — zero warnings, unwrap audit on main paths,
   full e2e matrix green.

## Phases

### Phase 1 — Verification completeness *(S)* → commit 1

| Item | Detail |
|------|--------|
| `RetSpec::ListI64` | driver arm printing MR descriptor as `n v0..v3` for `List<Int>` returns |
| Tuple parity | wire `RetSpec::Tuple` into `differential_parity`; compare components element-wise |
| F32 streams | populate `lists_f32` / `scalars_f32` from row values so F32-list/scalar params receive real data |
| Fail-closed shapes | return shapes without a driver **kill** the candidate (`differential-unproven`) instead of skipping |

Acceptance: matvec (List<F64>), a List<Int> kernel, a tuple kernel and an
F32 kernel each pass or fail *value* parity explicitly; suite green.

### Phase 2 — Forge-surface sync *(S-M)* → commits 2–3

| Item | Detail |
|------|--------|
| `ask_langref.txt` rewrite | tuples `(A, B)` returns; `let (a,b) = Dep.f(..);` destructuring; `flatmap`; `index2(t,i,j,stride)`; `++` concat; F32/F32-list types; remove false "NEVER concat"; keep honest-iteration guidance |
| Parse-parity harness | fixture list covering every GRAMMAR production round-trips GBNF text ↔ Rust parser; single `#[test]` walks them |
| Examples | `examples/qr_tuple.ont`, `examples/levenshtein.ont`, `examples/f32_scale.ont` — all solve end-to-end |

Acceptance: harness green; three examples `solve --hand` through parity.

### Phase 3 — Vault hygiene *(S)* → commit 4

| Item | Detail |
|------|--------|
| `vault rm <key-prefix\|name>` | removes manifest + artifacts (+ trust entry) |
| `vault doctor` | reports unparseable manifests, missing artifacts, path sprawl (>1 version), untrusted entries |
| `vault gc --orphans` | deletes ONLY manifests whose artifacts are missing or which fail to parse (user-selected policy) |
| Debug purge | remove tonight's isolation kernels via `vault rm` |

Policy note (user decision): gc never deletes well-formed versions —
orphans only.

### Phase 4 — Truthful docs *(S)* → commit 5

Capability report addendum #2: Gap 2 closed (tuples + destructure),
Gap 3 closed (flatmap/index2; Levenshtein native), scorecard final.
README: tuple/flatmap examples replace stale snippets; overflow section
gains proven-tier row placeholder→filled in Phase 7. IDENTITY.md artifact
table gains `.nous`, trust ledger, lint/proven rows. CHANGES consolidated.

### Phase 5 — Flywheel integration *(M)* → commit 6

Fresh `ontic decompose` over an in-repo paper text exercising ≥1 use of
new vocabulary (tuple output or flatmap). Repair rounds budgeted;
failures captured as DPO records. If forge backends unavailable locally,
run with `--spec-backend file:` fixtures so the pipeline itself is still
exercised end-to-end and say so in the report.

### Phase 6 — Strings v1 + minimal builtins *(M)* → commits 7–8

| Layer | Change |
|-------|--------|
| sketch.rs | `Ty::Str`; lexer: quoted `"..."` string literal token; parser: StrLit expr + Str in types; GRAMMAR |
| gen.rs | `Value::Str(String)`; example syntax `= "abc"`; probe edges: empty, 1-char, canonical words from small alphabet; deterministic sampler |
| check.rs | Str params allowed (opaque); `str_len :: Str -> Int`; `str_eq :: Str×Str -> Bool` |
| interp.rs | `Value::Str`; builtin evals |
| lower.rs | ABI: `(char* data, long len)` per Str param; `str_len` = len field load; `str_eq` = length-compare + byte loop; header/shim/hpp emission; probes unchanged shape |
| pipeline.rs | driver arms for string args (`char b[] = "..."` + strlen) and `const char*` returns printed via `%s` bounded |
| main.rs | parity: strings compare exactly |
| ask_langref/lint | document; no new rules needed beyond existing |

In-language text ops beyond the two builtins stay out (design line held).

### Phase 7 — Proven tier via full z3 *(L-XL)* → commits 9–11

| Item | Detail |
|------|--------|
| Cargo feature | `[features] proven = ["z3/static-link-z3"]`; default builds never compile z3 |
| `src/proven.rs` | encode: inputs as Z3 i64 vars constrained by input-side invariants; walk body arith ops asserting each intermediate ∈ [i64::MIN, i64::MAX]; nonlinear ops encoded directly (z3 handles nonlinear integers incompletely) |
| Query semantics | `Unsat`(no overflow reachable) → PROVEN; `Sat` → checked; `Unknown` → checked (soundness never depends on solver completeness) |
| Emitter | tier-aware arith selection: proven bodies skip widen-check-trap, emit plain `addi/subi/muli`; manifest stamps `"tier": "proven"` |
| Equivalence gate | every proven emission differentially tested vs checked-tier object on probe rows before acceptance |
| Docs | README overflow table + GUARDS note + GR10 amendment: z3 is the second approved dependency, feature-gated, vendored-build capable |

Nonlinear honesty: z3 may answer `unknown` where a human would prove
safety — those kernels stay checked. The tier is additive speed, never a
correctness claim beyond what the solver returned.

### Phase 8 — Final hardening *(S)* → commit 12

Unwrap audit on cmd_solve/native_rerank/import paths; e2e matrix re-run:
matvec, lev, qr_tuple chain, f32_scale, strings kernel, proven-vs-checked
equivalence; final capability scorecard numbers; morning report.

## Risk register

| Risk | Mitigation |
|------|-----------|
| z3 system-dep friction | feature-gated; static-vendor optional; default builds untouched |
| z3 `unknown` on nonlinear | falls back to checked — soundness independent of solver completeness |
| Decompose exposes forge gaps | budgeted repair rounds; failures are DPO feedstock |
| Proven-tier misproofs | every proven emission differentially tested against checked object before landing |
| Strings scope creep | opaque type + 2 builtins; emitter sees bytes only |

## Commit ledger (planned)

1. feat(pipeline): parity completeness — ListI64/tuple/F32 streams/fail-closed
2. docs(forge): ask_langref rewrite for current vocabulary
3. test(syntax): GRAMMAR↔parser parity harness + examples
4. feat(vault): rm/doctor/gc + debug purge
5. docs: capability report + README + IDENTITY refresh
6. feat(decompose): fresh flywheel integration run
7. feat(types): Str type + str_len/str_eq through all stages
8. feat(pipeline): string ABI drivers + parity
9. feat(proven): z3 encoding module (feature-gated)
10. feat(lower): tier-aware emission + equivalence tests
11. docs: proven tier + GR10 amendment
12. chore: hardening sweep + final scorecard + report
