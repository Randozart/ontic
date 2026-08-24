//! Sieve pipeline S1–S7: the deterministic judge of every candidate.
//!
//! THE WALL (AGENTS.md rule 1): model output enters only here as candidate
//! *text*. Everything past S1 is pure Rust. Every kill carries a structured,
//! machine-readable reason for forge feedback and `ontic check`.

use crate::check;
use crate::interp::{self, Env};
use crate::lower::expr_display;
use crate::overfit::{self, OverfitConfig, OverfitVerdict};
use crate::probes;
use crate::sketch::{self, Candidate, Expr};
use crate::gen::{Example, Value, Gen};
use std::collections::HashMap;
use std::time::Instant;

/// All sieve thresholds in one place — never scattered literals.
#[derive(Debug, Clone)]
pub struct SiegeConfig {
    pub probe_count: usize,
    pub seed: u64,
    pub edge_budget: usize,
    pub bench_iters: usize,
    pub overfit: OverfitConfig,
}

impl Default for SiegeConfig {
    fn default() -> Self {
        SiegeConfig {
            probe_count: 256,
            seed: 0x5EED,
            edge_budget: 64,
            bench_iters: 2_000,
            overfit: OverfitConfig::default(),
        }
    }
}

/// Sieve stages in execution order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Parse,
    WellFormed,
    Transparent,
    HeldOut,
    Probe,
    Shape,
    Bench,
}

impl Stage {
    /// Stable machine-readable stage label (forge feedback + CLI output).
    pub fn label(&self) -> &'static str {
        match self {
            Stage::Parse => "S1-parse",
            Stage::WellFormed => "S2-well-formed",
            Stage::Transparent => "S3-transparent",
            Stage::HeldOut => "S4-held-out",
            Stage::Probe => "S5-probe",
            Stage::Shape => "S6-shape",
            Stage::Bench => "S7-bench",
        }
    }
}

/// Why a candidate died.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KillKind {
    /// Malformed or ill-typed code.
    Invalid,
    /// Runtime error reachable on evidence inputs.
    RuntimeError,
    /// Failed visible evidence.
    WrongOutput,
    /// Passed visible evidence but failed hidden evidence — overfit.
    Overfit,
    /// Violated an invariant on a probe input.
    InvariantViolation,
    /// The gen itself is broken (unsatisfiable contract) — not a candidate fault.
    WishError,
}

impl KillKind {
    /// Stable machine-readable kind label.
    pub fn label(&self) -> &'static str {
        match self {
            KillKind::Invalid => "invalid",
            KillKind::RuntimeError => "runtime-error",
            KillKind::WrongOutput => "wrong-output",
            KillKind::Overfit => "overfit",
            KillKind::InvariantViolation => "invariant-violation",
            KillKind::WishError => "wish-error",
        }
    }
}

/// Structured rejection record — the forge feedback payload.
#[derive(Debug, Clone)]
pub struct Rejection {
    pub stage: Stage,
    pub kind: KillKind,
    pub reason: String,
}

/// A candidate that passed every stage, with its measured cost.
#[derive(Debug, Clone)]
pub struct Survivor {
    pub candidate: Candidate,
    pub source_text: String,
    pub ns_per_call: u64,
}

/// Outcome of sieving a candidate batch.
#[derive(Debug, Default)]
pub struct SieveReport {
    pub survivors: Vec<Survivor>,
    pub rejections: Vec<(String, Rejection)>,
}

/// Validate gen-level preconditions before any candidate is examined:
/// invariants must typecheck against signature + `%res`, and every example
/// (transparent, opaque, auto-hidden) must satisfy the input-side invariants.
/// An example outside the declared contract would silently kill every honest
/// candidate — the spec is broken, not the code.
pub fn validate_wish(gen: &Gen) -> Result<(), String> {
    check::check_invariants(&gen.invariants, &gen.params, &gen.ret)?;
    let ctx = interp::Ctx::checked();
    for (label, ex) in gen
        .transparent
        .iter()
        .map(|e| ("transparent", e))
        .chain(gen.opaque.iter().map(|e| ("opaque", e)))
    {
        let env: interp::Env = gen
            .params
            .iter()
            .zip(ex.inputs.iter())
            .map(|((n, _), v)| (n.clone(), v.clone()))
            .collect();
        for inv in &gen.invariants {
            match interp::eval_ctx(inv, &env, &ctx) {
                Ok(Value::Bool(true)) => {}
                Ok(Value::Bool(false)) => {
                    return Err(format!(
                        "gen {} example (output {:?}) violates invariant `{}` on inputs {:?}",
                        label, ex.output, crate::lower::expr_display(inv), ex.inputs
                    ))
                }
                _ => {} // res-referencing or non-Bool: typechecked already
            }
        }
    }
    Ok(())
}

/// Run the full pipeline over labeled candidate texts.
pub fn run(
    gen: &Gen,
    texts: &[(String, String)],
    cfg: &SiegeConfig,
    deps: &interp::DepMap,
) -> Result<SieveReport, String> {
    validate_wish(gen)?;
    let mut report = SieveReport::default();
    for (label, text) in texts {
        match run_one(gen, text, cfg, deps) {
            Ok(survivor) => report.survivors.push(survivor),
            Err(rej) => report.rejections.push((label.clone(), rej)),
        }
    }
    rank(&mut report);
    Ok(report)
}

/// Dependency signature table for the typechecker.
fn dep_sigs(deps: &interp::DepMap) -> crate::check::DepSigs {
    deps.iter()
        .map(|(p, d)| {
            (
                p.clone(),
                (
                    d.cand.params.iter().map(|(_, t)| t.clone()).collect(),
                    d.cand.ret.clone(),
                ),
            )
        })
        .collect()
}

fn reject(stage: Stage, kind: KillKind, reason: String) -> Rejection {
    Rejection { stage, kind, reason }
}

fn run_one(
    gen: &Gen,
    text: &str,
    cfg: &SiegeConfig,
    deps: &interp::DepMap,
) -> Result<Survivor, Rejection> {
    let tier = if gen.wrapping {
        interp::Tier::wrapping()
    } else {
        interp::Tier::checked()
    };
    let ictx = interp::Ctx {
        tier,
        deps: std::sync::Arc::new(deps.clone()),
    };
    let _ = &ictx;
    // S1 parse
    let cand = sketch::parse(text)
        .map_err(|e| reject(Stage::Parse, KillKind::Invalid, format!("offset {}: {}", e.offset, e.message)))?;

    // S2 well-formedness (dep-aware when dependencies are resolved)
    if deps.is_empty() {
        check::check(&cand).map_err(|m| reject(Stage::WellFormed, KillKind::Invalid, m))?;
    } else {
        let sigs = dep_sigs(deps);
        check::check_with(&cand, &sigs)
            .map_err(|m| reject(Stage::WellFormed, KillKind::Invalid, m))?;
    }

    // S3 transparent evidence
    eval_set(gen, &cand, &gen.transparent, &ictx).map_err(|(i, m)| {
        reject(
            Stage::Transparent,
            KillKind::WrongOutput,
            format!("visible example #{}: {}", i, m),
        )
    })?;

    // S4 held-out evidence
    eval_set(gen, &cand, &gen.opaque, &ictx).map_err(|(i, m)| {
        reject(
            Stage::HeldOut,
            KillKind::Overfit,
            format!(
                "hidden example #{} failed: {} (generalization failure)",
                i, m
            ),
        )
    })?;

    // S5 probes
    run_probes(gen, &cand, cfg, &ictx)?;

    // S6 shape scan
    if let OverfitVerdict::Suspicious(m) =
        overfit::scan(&cand, &gen.transparent, &cfg.overfit)
    {
        return Err(reject(Stage::Shape, KillKind::Overfit, m));
    }

    // S7 bench
    let ns = bench(gen, &cand, cfg.bench_iters, &ictx);
    Ok(Survivor {
        candidate: cand,
        source_text: text.to_string(),
        ns_per_call: ns,
    })
}

/// Evaluate every example against `cand`; on mismatch return (index, why).
fn eval_set(
    gen: &Gen,
    cand: &Candidate,
    set: &[Example],
    ctx: &interp::Ctx,
) -> Result<(), (usize, String)> {
    let _ = gen;
    for (i, ex) in set.iter().enumerate() {
        if ex.inputs.len() != gen.params.len() {
            continue; // arity already validated at gen level
        }
        let got =
            interp::eval_candidate(cand, &ex.inputs, ctx).map_err(|e| (i, e.to_string()))?;
        if !evidence_holds(&ex.output, &got, ex.tol) {
            return Err((
                i,
                format!(
                    "inputs [{}] expected {} ± {}, got {}",
                    inputs_str(&ex.inputs),
                    ex.output,
                    ex.tol,
                    got
                ),
            ));
        }
    }
    Ok(())
}

/// S5: seeded probe rows; runtime errors and invariant violations kill with
/// a recorded counterexample.
fn run_probes(
    gen: &Gen,
    cand: &Candidate,
    cfg: &SiegeConfig,
    ctx: &interp::Ctx,
) -> Result<(), Rejection> {
    let plan = probes::generate(gen, cfg.probe_count, cfg.seed, cfg.edge_budget, ctx).map_err(
        |_| {
            let invs: Vec<String> = gen
                .invariants
                .iter()
                .map(crate::lower::expr_display)
                .collect();
            reject(
                Stage::Probe,
                KillKind::WishError,
                format!(
                    "gen contract excludes every probeable input: no canonical edge row satisfies the invariants [{}]. \
                     The declared domain is empty or narrower than the type domain — fix the spec (loosen an invariant, \
                     widen a type) rather than the candidates.",
                    invs.join("; ")
                ),
            )
        },
    )?;
    let rows = plan.rows;
    for row in rows {
        let res = match interp::eval_candidate(cand, &row, &ctx) {
            Ok(v) => v,
            Err(e) => {
                return Err(reject(
                    Stage::Probe,
                    KillKind::RuntimeError,
                    format!("input [{}] raised {}", inputs_str(&row), e),
                ))
            }
        };
        if let Some(reason) = check_invariants_on(gen, &row, &res, ctx) {
            return Err(reject(Stage::Probe, KillKind::InvariantViolation, reason));
        }
    }
    Ok(())
}

/// Evaluate every invariant under env(params → inputs, "res" → result).
/// Returns None when all hold; otherwise a counterexample reason string.
fn check_invariants_on(
    gen: &Gen,
    inputs: &[Value],
    res: &Value,
    ctx: &interp::Ctx,
) -> Option<String> {
    let mut env: Env = HashMap::new();
    for ((name, _), v) in gen.params.iter().zip(inputs.iter()) {
        env.insert(name.clone(), v.clone());
    }
    env.insert("res".to_string(), res.clone());
    for inv in &gen.invariants {
        let held = match interp::eval_ctx(inv, &env, &ctx) {
            Ok(Value::Bool(b)) => b,
            Ok(other) => {
                return Some(format!(
                    "invariant `{}` evaluated to {} instead of Bool",
                    expr_display(inv),
                    other
                ))
            }
            Err(e) => {
                return Some(format!("invariant `{}` errored: {}", expr_display(inv), e))
            }
        };
        if !held {
            return Some(format!(
                "invariant `{}` violated on input [{}] (res = {})",
                expr_display(inv),
                inputs_str(inputs),
                res
            ));
        }
    }
    None
}

/// Evidence comparison: exact for Int/Bool/List; abs+rel epsilon for F64
/// outputs (tolerance is contract — cited verbatim in every kill reason).
fn evidence_holds(want: &Value, got: &Value, tol: f64) -> bool {
    let f_ok = |w: f64, g: f64| -> bool {
        let slack = tol + 1e-9 * w.abs();
        (g - w).abs() <= slack.max(f64::EPSILON * 4.0)
    };
    match (want, got) {
        (Value::Float(w), Value::Float(g)) => f_ok(*w, *g),
        (Value::FloatList(ws), Value::FloatList(gs)) => {
            ws.len() == gs.len()
                && ws.iter().zip(gs.iter()).all(|(w, g)| f_ok(*w, *g))
        }
        _ => want == got,
    }
}

fn inputs_str(inputs: &[Value]) -> String {
    inputs
        .iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

/// S7: coarse deterministic timing — round-robin transparent examples.
/// Ranking signal only; correctness was settled by S3–S6.
fn bench(gen: &Gen, cand: &Candidate, iters: usize, ctx: &interp::Ctx) -> u64 {
    let _ = gen;
    let set = &gen.transparent;
    if set.is_empty() || iters == 0 {
        return u64::MAX;
    }
    let start = Instant::now();
    for i in 0..iters {
        let ex = &set[i % set.len()];
        let _ = interp::eval_candidate(cand, &ex.inputs, ctx);
    }
    let calls = (iters * set.len()) as u128;
    (start.elapsed().as_nanos() / calls) as u64
}

/// Order survivors: measured cost first, AST size breaks ties deterministically.
fn rank(report: &mut SieveReport) {
    report
        .survivors
        .sort_by(|a, b| a.ns_per_call.cmp(&b.ns_per_call).then(ast_size(&a.candidate.body).cmp(&ast_size(&b.candidate.body))));
}

/// Total AST node count — deterministic tie-break for equal timings.
pub fn ast_size(e: &Expr) -> usize {
    1 + match e {
        Expr::IntLit(_) | Expr::FloatLit(_) | Expr::BoolLit(_) | Expr::Var(_)
        | Expr::ListLit(_) | Expr::FloatListLit(_) => 0,
        Expr::Builtin(_, i) | Expr::UnOp(_, i) => ast_size(i),
        Expr::ListCons(elems) => 1 + elems.iter().map(ast_size).sum::<usize>(),
        Expr::Builtin2(_, a, b) => ast_size(a) + ast_size(b),
        Expr::Map { var: _, list, body } => 1 + ast_size(list) + ast_size(body),
        Expr::Call(_, args) => {
            1 + args.iter().map(ast_size).sum::<usize>()
        }
        Expr::If(c, t, f) => ast_size(c) + ast_size(t) + ast_size(f),
        Expr::Let(_, v, b) => ast_size(v) + ast_size(b),
        Expr::Fold {
            list,
            init,
            body,
            ..
        } => ast_size(list) + ast_size(init) + ast_size(body),
        Expr::BinOp(_, l, r) => ast_size(l) + ast_size(r),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gen;

    const SUM_GEN: &str = "\
fn Ledger.total(%items: List<Int>) -> Int
  => [1,2,3] -> 6
  => [] -> 0
  => [5] -> 5
  ?? [4,5] -> 9
";

    const HONEST: &str =
        "fn @total(%items: List<Int>) -> Int { fold %x in %items, %acc from 0 { %acc + %x } }";

    fn sum_wish() -> Gen {
        gen::parse(SUM_GEN).expect("gen parses")
    }

    #[test]
    fn test_honest_fold_survives_all_stages() {
        let w = sum_wish();
        let texts = vec![("honest".to_string(), HONEST.to_string())];
        let r = run(&w, &texts, &SiegeConfig::default(), &interp::DepMap::new()).expect("gen valid");
        assert_eq!(r.survivors.len(), 1, "{:?}", r.rejections);
        assert!(r.rejections.is_empty());
    }

    #[test]
    fn test_lookup_table_killed_by_hidden_evidence() {
        let w = sum_wish();
        let table = "fn @t(%items: List<Int>) -> Int { if len(%items) == 3 { fold %x in %items, %a from 0 { %a + %x } } else { if len(%items) == 0 { 0 } else { 5 } } }";
        let texts = vec![("table".to_string(), table.to_string())];
        let r = run(&w, &texts, &SiegeConfig::default(), &interp::DepMap::new()).expect("gen valid");
        assert_eq!(r.survivors.len(), 0);
        assert_eq!(r.rejections[0].1.stage, Stage::HeldOut);
        assert_eq!(r.rejections[0].1.kind, KillKind::Overfit);
    }

    #[test]
    fn test_invariant_violator_caught_at_probe_stage() {
        let w = gen::parse(
            "fn f(%items: List<Int>) -> Int\n  | %res >= 0\n  => [1] -> 1\n  => [] -> 0\n",
        )
        .unwrap();
        // Passes both visible examples, but treats the first element
        // specially — negative elements flow straight through on probes.
        let sneaky =
            "fn @f(%items: List<Int>) -> Int { fold %x in %items, %acc from 0 { if %acc == 0 { %x } else { %acc + %x } } }";
        let texts = vec![("sneaky".to_string(), sneaky.to_string())];
        let r = run(&w, &texts, &SiegeConfig::default(), &interp::DepMap::new()).expect("gen valid");
        assert_eq!(r.survivors.len(), 0);
        assert_eq!(r.rejections[0].1.stage, Stage::Probe);
        assert_eq!(r.rejections[0].1.kind, KillKind::InvariantViolation);
        assert!(r.rejections[0].1.reason.contains("violated"));
    }

    #[test]
    fn test_malformed_candidate_dies_at_s1() {
        let w = sum_wish();
        let texts = vec![("junk".to_string(), "fn @t( {".to_string())];
        let r = run(&w, &texts, &SiegeConfig::default(), &interp::DepMap::new()).expect("gen valid");
        assert_eq!(r.rejections[0].1.stage, Stage::Parse);
    }

    #[test]
    fn test_type_error_dies_at_s2() {
        let w = sum_wish();
        // Internally inconsistent: body is Int, own signature says Bool.
        let bad = "fn @t(%items: List<Int>) -> Bool { len(%items) }";
        let texts = vec![("badty".to_string(), bad.to_string())];
        let r = run(&w, &texts, &SiegeConfig::default(), &interp::DepMap::new()).expect("gen valid");
        assert_eq!(r.rejections[0].1.stage, Stage::WellFormed);
    }

    #[test]
    fn test_wrong_visible_output_dies_at_s3() {
        let w = sum_wish();
        let off = "fn @t(%items: List<Int>) -> Int { fold %x in %items, %acc from 1 { %acc + %x } }";
        let texts = vec![("offbyone".to_string(), off.to_string())];
        let r = run(&w, &texts, &SiegeConfig::default(), &interp::DepMap::new()).expect("gen valid");
        assert_eq!(r.rejections[0].1.stage, Stage::Transparent);
        assert_eq!(r.rejections[0].1.kind, KillKind::WrongOutput);
    }

    #[test]
    fn test_division_reachable_on_probes_kills_candidate() {
        let w = gen::parse(
            "fn f(%a: Int, %b: Int) -> Int\n  | %res >= -1000000\n  => 6, 3 -> 2\n  => 9, 3 -> 3\n",
        )
        .unwrap();
        // Passes both visible examples; probe edge b=0 makes the division
        // blow up at S5 instead.
        let risky = "fn @f(%a: Int, %b: Int) -> Int { %a / %b }";
        let texts = vec![("risky".to_string(), risky.to_string())];
        let r = run(&w, &texts, &SiegeConfig::default(), &interp::DepMap::new()).expect("gen valid");
        assert!(matches!(
            &r.rejections[0].1.kind,
            KillKind::RuntimeError | KillKind::InvariantViolation
        ));
    }

    #[test]
    fn test_rank_orders_survivors_deterministically() {
        let w = sum_wish();
        let padded = "fn @p(%items: List<Int>) -> Int { fold %x in %items, %acc from 0 { %acc + %x + 0 * len([1,2,3]) } }";
        let texts = vec![
            ("honest".to_string(), HONEST.to_string()),
            ("padded".to_string(), padded.to_string()),
        ];
        let r = run(&w, &texts, &SiegeConfig::default(), &interp::DepMap::new()).expect("gen valid");
        assert_eq!(r.survivors.len(), 2);
        for pair in r.survivors.windows(2) {
            assert!(pair[0].ns_per_call <= pair[1].ns_per_call);
        }
    }

    #[test]
    fn test_bad_invariant_is_wish_error_not_candidate_kill() {
        let w = gen::parse("fn f(%a: Int) -> Int\n  | %zz > 0\n  => 1 -> 2\n").unwrap();
        assert!(run(
            &w,
            &[("h".into(), HONEST.into())],
            &SiegeConfig::default(),
            &interp::DepMap::new()
        )
        .is_err());
    }

    #[test]
    fn test_example_violating_invariant_is_wish_error() {
        // n=1 with a 2-element list contradicts len(%a) == %n * %n; the
        // spec is broken and must fail validation, never kill candidates.
        let w = gen::parse(
            "fn s(%s: F64, %a: List<F64>, %n: Int) -> List<F64>\n  | %n > 0\n  | len(%a) == %n * %n\n  => 0.5, [4.0], 1 -> [2.0]\n",
        )
        .unwrap();
        assert!(validate_wish(&w).is_ok());

        let bad = gen::parse(
            "fn s(%s: F64, %a: List<F64>, %n: Int) -> List<F64>\n  | %n > 0\n  | len(%a) == %n * %n\n  => 0.5, [2.0, 4.0], 1 -> [1.0, 2.0]\n",
        )
        .unwrap();
        let err = validate_wish(&bad).unwrap_err();
        assert!(err.contains("violates invariant"), "got: {}", err);
    }

    #[test]
    fn test_ast_size_counts_nodes() {
        let e = sketch::parse_expr_str("%a + (len(%l))").unwrap();
        assert_eq!(ast_size(&e), 4); // binop + var + len + var
    }

    #[test]
    fn test_wrapping_tier_survives_overflow_values() {
        // i64::MAX-scale sums would kill the checked tier; declared wrapping
        // makes them defined semantics, so the honest fold survives.
        let w = gen::parse(
            "fn f(%items: List<Int>) -> Int\n  wrapping\n  => [1] -> 1\n  => [] -> 0\n",
        )
        .unwrap();
        assert!(w.wrapping);
        let texts = vec![("honest".to_string(), HONEST.to_string())];
        let r = run(&w, &texts, &SiegeConfig::default(), &interp::DepMap::new()).expect("gen valid");
        assert_eq!(r.survivors.len(), 1, "{:?}", r.rejections);
    }

    #[test]
    fn test_checked_tier_still_kills_overflow_reachable() {
        // Passes both visible examples; probe rows with 3+ elements blow
        // past i64 under repeated ×1e9 — checked tier must kill at S5.
        let w = gen::parse(
            "fn f(%items: List<Int>) -> Int\n  => [1] -> 1000000000\n  => [] -> 1\n",
        )
        .unwrap();
        let big = "fn @f(%items: List<Int>) -> Int { fold %x in %items, %acc from 1 { %acc * 1000000000 } }";
        let texts = vec![("big".to_string(), big.to_string())];
        let r = run(&w, &texts, &SiegeConfig::default(), &interp::DepMap::new()).expect("gen valid");
        assert_eq!(r.survivors.len(), 0);
        assert_eq!(r.rejections[0].1.stage, Stage::Probe);
        assert_eq!(r.rejections[0].1.kind, KillKind::RuntimeError);
    }

    #[test]
    fn test_canonical_includes_wrapping() {
        let plain = gen::parse("fn f(%a: Int) -> Int\n  => 1 -> 2\n").unwrap();
        let wrap = gen::parse("fn f(%a: Int) -> Int\n  wrapping\n  => 1 -> 2\n").unwrap();
        assert_ne!(plain.canonical(), wrap.canonical());
    }
}

#[cfg(test)]
mod compose_tests {
    use super::*;
    use crate::{sketch, gen};
    use std::collections::HashMap;

    const MEAN_SRC: &str =
        "fn @mean(%xs: List<F64>) -> F64 { let %n = len(%xs); if %n == 0 { 0.0 } else { sum(%xs) / %n } }";
    const DEVSQ_SRC: &str = "fn @devsq(%xs: List<F64>) -> F64 { let %m = Stats.mean(%xs); if len(%xs) == 0 { 0.0 } else { sum((%xs - %m) * (%xs - %m)) / len(%xs) } }";

    /// G3 gate: a candidate calling a VAULT function composes through the
    /// full sieve. The dep executes under its own tier.
    #[test]
    fn test_vault_call_composition_survives_sieve() {
        let w = gen::parse(
            "use Stats.mean\nfn DevSq(%xs: List<F64>) -> F64\n  | %res >= 0\n  => [2.0,4.0] -> 1.0 ± 1e-9\n  ?? [3.0] -> 0.0 ± 1e-12\n",
        )
        .expect("gen parses");
        assert_eq!(w.deps, vec!["Stats.mean".to_string()]);

        let mut deps: interp::DepMap = HashMap::new();
        let mean_cand = sketch::parse(MEAN_SRC).unwrap();
        check::check(&mean_cand).unwrap();
        deps.insert(
            "Stats.mean".to_string(),
            interp::DepFn {
                cand: mean_cand,
                tier: interp::Tier::wrapping(),
            },
        );

        let texts = vec![("composed".to_string(), DEVSQ_SRC.to_string())];
        let r = run(&w, &texts, &SiegeConfig::default(), &deps).expect("gen valid");
        assert_eq!(r.survivors.len(), 1, "{:?}", r.rejections);
    }

    #[test]
    fn test_undeclared_call_killed_at_s2() {
        let w = gen::parse(
            "fn Bad(%xs: List<F64>) -> F64\n  => [1.0] -> 0.0\n",
        )
        .unwrap();
        // No deps declared; candidate calls anyway.
        let texts = vec![("bad".to_string(), DEVSQ_SRC.to_string())];
        let r = run(&w, &texts, &SiegeConfig::default(), &interp::DepMap::new())
            .expect("gen valid");
        assert_eq!(r.rejections[0].1.stage, Stage::WellFormed);
        assert!(
            r.rejections[0].1.reason.contains("requires declared `use`"),
            "{}",
            r.rejections[0].1.reason
        );
    }
}
