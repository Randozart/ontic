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
use crate::wish::{Example, Value, Wish};
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
            edge_budget: 16,
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

/// Validate wish-level preconditions before any candidate is examined:
/// invariants must typecheck against signature + `%res`.
pub fn validate_wish(wish: &Wish) -> Result<(), String> {
    check::check_invariants(&wish.invariants, &wish.params, &wish.ret)
}

/// Run the full pipeline over labeled candidate texts.
pub fn run(
    wish: &Wish,
    texts: &[(String, String)],
    cfg: &SiegeConfig,
) -> Result<SieveReport, String> {
    validate_wish(wish)?;
    let mut report = SieveReport::default();
    for (label, text) in texts {
        match run_one(wish, text, cfg) {
            Ok(survivor) => report.survivors.push(survivor),
            Err(rej) => report.rejections.push((label.clone(), rej)),
        }
    }
    rank(&mut report);
    Ok(report)
}

fn reject(stage: Stage, kind: KillKind, reason: String) -> Rejection {
    Rejection { stage, kind, reason }
}

fn run_one(wish: &Wish, text: &str, cfg: &SiegeConfig) -> Result<Survivor, Rejection> {
    // S1 parse
    let cand = sketch::parse(text)
        .map_err(|e| reject(Stage::Parse, KillKind::Invalid, format!("offset {}: {}", e.offset, e.message)))?;

    // S2 well-formedness
    check::check(&cand)
        .map_err(|m| reject(Stage::WellFormed, KillKind::Invalid, m))?;

    // S3 transparent evidence
    eval_set(wish, &cand, &wish.transparent).map_err(|(i, m)| {
        reject(
            Stage::Transparent,
            KillKind::WrongOutput,
            format!("visible example #{}: {}", i, m),
        )
    })?;

    // S4 held-out evidence
    eval_set(wish, &cand, &wish.opaque).map_err(|(i, m)| {
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
    run_probes(wish, &cand, cfg)?;

    // S6 shape scan
    if let OverfitVerdict::Suspicious(m) =
        overfit::scan(&cand, &wish.transparent, &cfg.overfit)
    {
        return Err(reject(Stage::Shape, KillKind::Overfit, m));
    }

    // S7 bench
    let ns = bench(wish, &cand, cfg.bench_iters);
    Ok(Survivor {
        candidate: cand,
        source_text: text.to_string(),
        ns_per_call: ns,
    })
}

/// Evaluate every example against `cand`; on mismatch return (index, why).
fn eval_set(wish: &Wish, cand: &Candidate, set: &[Example]) -> Result<(), (usize, String)> {
    for (i, ex) in set.iter().enumerate() {
        if ex.inputs.len() != wish.params.len() {
            continue; // arity already validated at wish level
        }
        let got = interp::eval_candidate(cand, &ex.inputs).map_err(|e| (i, e.to_string()))?;
        if got != ex.output {
            return Err((
                i,
                format!(
                    "inputs [{}] expected {}, got {}",
                    inputs_str(&ex.inputs),
                    ex.output,
                    got
                ),
            ));
        }
    }
    Ok(())
}

/// S5: seeded probe rows; runtime errors and invariant violations kill with
/// a recorded counterexample.
fn run_probes(wish: &Wish, cand: &Candidate, cfg: &SiegeConfig) -> Result<(), Rejection> {
    let rows = probes::generate(wish, cfg.probe_count, cfg.seed, cfg.edge_budget);
    for row in rows {
        let res = match interp::eval_candidate(cand, &row) {
            Ok(v) => v,
            Err(e) => {
                return Err(reject(
                    Stage::Probe,
                    KillKind::RuntimeError,
                    format!("input [{}] raised {}", inputs_str(&row), e),
                ))
            }
        };
        if let Some(reason) = check_invariants_on(wish, &row, &res) {
            return Err(reject(Stage::Probe, KillKind::InvariantViolation, reason));
        }
    }
    Ok(())
}

/// Evaluate every invariant under env(params → inputs, "res" → result).
/// Returns None when all hold; otherwise a counterexample reason string.
fn check_invariants_on(wish: &Wish, inputs: &[Value], res: &Value) -> Option<String> {
    let mut env: Env = HashMap::new();
    for ((name, _), v) in wish.params.iter().zip(inputs.iter()) {
        env.insert(name.clone(), v.clone());
    }
    env.insert("res".to_string(), res.clone());
    for inv in &wish.invariants {
        let held = match interp::eval(inv, &env) {
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

fn inputs_str(inputs: &[Value]) -> String {
    inputs
        .iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

/// S7: coarse deterministic timing — round-robin transparent examples.
/// Ranking signal only; correctness was settled by S3–S6.
fn bench(wish: &Wish, cand: &Candidate, iters: usize) -> u64 {
    let set = &wish.transparent;
    if set.is_empty() || iters == 0 {
        return u64::MAX;
    }
    let start = Instant::now();
    for i in 0..iters {
        let ex = &set[i % set.len()];
        let _ = interp::eval_candidate(cand, &ex.inputs);
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
        Expr::IntLit(_) | Expr::BoolLit(_) | Expr::Var(_) | Expr::ListLit(_) => 0,
        Expr::Len(i) | Expr::UnOp(_, i) => ast_size(i),
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
    use crate::wish;

    const SUM_WISH: &str = "\
fn Ledger.total(%items: List<Int>) -> Int
  => [1,2,3] -> 6
  => [] -> 0
  => [5] -> 5
  ?? [4,5] -> 9
";

    const HONEST: &str =
        "fn @total(%items: List<Int>) -> Int { fold %x in %items, %acc from 0 { %acc + %x } }";

    fn sum_wish() -> Wish {
        wish::parse(SUM_WISH).expect("wish parses")
    }

    #[test]
    fn test_honest_fold_survives_all_stages() {
        let w = sum_wish();
        let texts = vec![("honest".to_string(), HONEST.to_string())];
        let r = run(&w, &texts, &SiegeConfig::default()).expect("wish valid");
        assert_eq!(r.survivors.len(), 1, "{:?}", r.rejections);
        assert!(r.rejections.is_empty());
    }

    #[test]
    fn test_lookup_table_killed_by_hidden_evidence() {
        let w = sum_wish();
        let table = "fn @t(%items: List<Int>) -> Int { if len(%items) == 3 { fold %x in %items, %a from 0 { %a + %x } } else { if len(%items) == 0 { 0 } else { 5 } } }";
        let texts = vec![("table".to_string(), table.to_string())];
        let r = run(&w, &texts, &SiegeConfig::default()).expect("wish valid");
        assert_eq!(r.survivors.len(), 0);
        assert_eq!(r.rejections[0].1.stage, Stage::HeldOut);
        assert_eq!(r.rejections[0].1.kind, KillKind::Overfit);
    }

    #[test]
    fn test_invariant_violator_caught_at_probe_stage() {
        let w = wish::parse(
            "fn f(%items: List<Int>) -> Int\n  | %res >= 0\n  => [1] -> 1\n  => [] -> 0\n",
        )
        .unwrap();
        // Passes both visible examples, but treats the first element
        // specially — negative elements flow straight through on probes.
        let sneaky =
            "fn @f(%items: List<Int>) -> Int { fold %x in %items, %acc from 0 { if %acc == 0 { %x } else { %acc + %x } } }";
        let texts = vec![("sneaky".to_string(), sneaky.to_string())];
        let r = run(&w, &texts, &SiegeConfig::default()).expect("wish valid");
        assert_eq!(r.survivors.len(), 0);
        assert_eq!(r.rejections[0].1.stage, Stage::Probe);
        assert_eq!(r.rejections[0].1.kind, KillKind::InvariantViolation);
        assert!(r.rejections[0].1.reason.contains("violated"));
    }

    #[test]
    fn test_malformed_candidate_dies_at_s1() {
        let w = sum_wish();
        let texts = vec![("junk".to_string(), "fn @t( {".to_string())];
        let r = run(&w, &texts, &SiegeConfig::default()).expect("wish valid");
        assert_eq!(r.rejections[0].1.stage, Stage::Parse);
    }

    #[test]
    fn test_type_error_dies_at_s2() {
        let w = sum_wish();
        // Internally inconsistent: body is Int, own signature says Bool.
        let bad = "fn @t(%items: List<Int>) -> Bool { len(%items) }";
        let texts = vec![("badty".to_string(), bad.to_string())];
        let r = run(&w, &texts, &SiegeConfig::default()).expect("wish valid");
        assert_eq!(r.rejections[0].1.stage, Stage::WellFormed);
    }

    #[test]
    fn test_wrong_visible_output_dies_at_s3() {
        let w = sum_wish();
        let off = "fn @t(%items: List<Int>) -> Int { fold %x in %items, %acc from 1 { %acc + %x } }";
        let texts = vec![("offbyone".to_string(), off.to_string())];
        let r = run(&w, &texts, &SiegeConfig::default()).expect("wish valid");
        assert_eq!(r.rejections[0].1.stage, Stage::Transparent);
        assert_eq!(r.rejections[0].1.kind, KillKind::WrongOutput);
    }

    #[test]
    fn test_division_reachable_on_probes_kills_candidate() {
        let w = wish::parse(
            "fn f(%a: Int, %b: Int) -> Int\n  | %res >= -1000000\n  => 6, 3 -> 2\n  => 9, 3 -> 3\n",
        )
        .unwrap();
        // Passes both visible examples; probe edge b=0 makes the division
        // blow up at S5 instead.
        let risky = "fn @f(%a: Int, %b: Int) -> Int { %a / %b }";
        let texts = vec![("risky".to_string(), risky.to_string())];
        let r = run(&w, &texts, &SiegeConfig::default()).expect("wish valid");
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
        let r = run(&w, &texts, &SiegeConfig::default()).expect("wish valid");
        assert_eq!(r.survivors.len(), 2);
        for pair in r.survivors.windows(2) {
            assert!(pair[0].ns_per_call <= pair[1].ns_per_call);
        }
    }

    #[test]
    fn test_bad_invariant_is_wish_error_not_candidate_kill() {
        let w = wish::parse("fn f(%a: Int) -> Int\n  | %zz > 0\n  => 1 -> 2\n").unwrap();
        assert!(run(&w, &[("h".into(), HONEST.into())], &SiegeConfig::default())
            .is_err());
    }

    #[test]
    fn test_ast_size_counts_nodes() {
        let e = sketch::parse_expr_str("%a + (len(%l))").unwrap();
        assert_eq!(ast_size(&e), 4); // binop + var + len + var
    }
}
