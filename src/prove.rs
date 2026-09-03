//! Overflow-absence proofs for the `proven` tier (feature-gated, GR11).
//!
//! A candidate earns `proven` when every integer arithmetic site in its
//! body is PROVABLY free of checked-tier traps across the WHOLE
//! invariant-satisfying parameter domain — not merely the probed sample.
//! This is the deterministic precondition for flag-free native codegen
//! later: a trap branch that cannot fire may eventually be omitted
//! ("speed requires declaration", GR11).
//!
//! Encoding — z3 Int theory (unbounded integers match our checked
//! semantics exactly; there is no wrapping behavior to model):
//! - One query per candidate. Assert `domain ∧ violation-disjunction`.
//!   Unsat ⇒ Proven. Sat ⇒ Unproven, printed witness as evidence.
//! - Body subset v1: straight-line Int expressions over scalar Int params
//!   and literals — Var/IntLit/Add/Sub/Mul/Neg/comparisons/And/Or/Not/
//!   Let/if-with-uniform-type. Everything else ⇒ Unproven naming the
//!   shape. Restricted coverage only WEAKENS toward conservative answers;
//!   a Proven verdict is never fabricated from partial information.
//! - DIVISION FAMILY IS EXCLUDED BY DESIGN: SMT-LIB `div`/`mod` round
//!   Euclidean-style while the oracle truncates toward zero. Divergent
//!   VALUES feeding a parent's range check could fabricate Proven — an
//!   unsound direction we refuse until value-faithful modeling lands.
//! - Invariant conjuncts mapping onto the supported subset become domain
//!   constraints; conjuncts mentioning unknowns (list lengths, fold state)
//!   are dropped, never approximated — again safe-direction-only.
//!
//! THE WALL: runs AFTER sieve acceptance on candidates the oracle already
//! passed. Sat changes nothing about vaulting; this module annotates, it
//! never admits.

use crate::sketch::{BinOp, Expr, Ty};
use crate::{gen::Gen, sketch::Candidate};
use std::collections::HashMap;
use z3::ast::{Ast, Bool as ZBool, Int as ZInt};
use z3::{Context, SatResult, Solver};

/// Verdict for one candidate's overflow-absence analysis.
#[derive(Debug, Clone)]
pub enum Proof {
    /// Every trap is impossible under the declared invariants.
    Proven(String),
    /// Proof unavailable or refuted; carries the honest reason or a
    /// concrete parameter witness trapping under the current contract.
    Unproven(String),
}

/// Shared proven-subset gate: `None` when the candidate body is within
/// the flag-free proven subset (scalar-Int params, scalar Int/Bool return,
/// straight-line Int shapes only). `Some(reason)` is the honest reason it
/// is NOT. Both `proof_for` (z3 encoding) and `lower::emit_fn_tier`
/// (emission selection) call this — a single source so the emitter and
/// the prover cannot drift.
pub fn subset_ok(cand: &Candidate) -> Option<String> {
    for (n, t) in &cand.params {
        if !matches!(t, Ty::Int) {
            return Some(format!(
                "param %{n}: {} — proven v1 covers scalar Int params",
                t.name()
            ));
        }
    }
    if !matches!(cand.ret, Ty::Int | Ty::Bool) {
        return Some(format!(
            "return {} outside scalar-Int scope of proven v1",
            cand.ret.name()
        ));
    }
    unsupported_shape(&cand.body)
}

/// Analyze one accepted candidate against its gen's invariants.
pub fn proof_for(gen: &Gen, cand: &Candidate) -> Proof {
    if let Some(reason) = subset_ok(cand) {
        return Proof::Unproven(reason);
    }
    let names: Vec<String> = cand
        .params
        .iter()
        .map(|(n, _)| n.clone())
        .collect();
    let sites = count_arith(&cand.body);
    if sites == 0 && !contains_neg(&cand.body) {
        return Proof::Proven("no arithmetic sites in body".to_string());
    }

    let cfg = z3::Config::new();
    let ctx = Context::new(&cfg);
    let vars: Vec<(String, ZInt<'_>)> = names
        .iter()
        .map(|n| (n.clone(), ZInt::fresh_const(&ctx, n)))
        .collect();

    let solver = Solver::new(&ctx);
    let mut conj_count = 0usize;
    for inv in &gen.invariants {
        for c in crate::lower::conjuncts(inv) {
            let mut env: HashMap<String, ZInt<'_>> = HashMap::new();
            if let Ok(p) = bool_of(&ctx, c, &vars, &mut env) {
                solver.assert(&p);
                conj_count += 1;
            }
            // Unknown conjunct shape: dropped on purpose (conservative).
        }
    }
    if conj_count == 0 && !names.is_empty() {
        return Proof::Unproven(
            "no invariants constrain these parameters; domains span all of i64".to_string(),
        );
    }

    let mut env: HashMap<String, ZInt<'_>> = vars
        .iter()
        .map(|(n, v)| (n.clone(), v.clone()))
        .collect();
    let mut parts: Vec<ZBool<'_>> = Vec::new();
    collect_violations(&ctx, &cand.body, &vars, &mut env, &mut parts);
    if parts.is_empty() {
        return Proof::Proven(format!(
            "{sites} arithmetic sites unreachable from traps under {conj_count} invariant constraint(s)"
        ));
    }
    let violations = ZBool::or(&ctx, &parts.iter().collect::<Vec<_>>());
    solver.assert(&violations);
    match solver.check() {
        SatResult::Unsat => Proof::Proven(format!(
            "{sites} arithmetic sites trap-free under {conj_count} invariant constraint(s)"
        )),
        SatResult::Sat => {
            let witness = solver_witness(&solver, &vars);
            Proof::Unproven(format!("traps reachable at {witness}"))
        }
        SatResult::Unknown => Proof::Unproven("solver returned unknown".to_string()),
    }
}

fn solver_witness<'c>(solver: &Solver<'c>, vars: &[(String, ZInt<'c>)]) -> String {
    match solver.get_model() {
        Some(m) => vars
            .iter()
            .map(|(n, v)| {
                let text = m
                    .eval(v, true)
                    .map(|x| x.to_string())
                    .unwrap_or_else(|| "?".to_string());
                format!("%{n}={text}")
            })
            .collect::<Vec<_>>()
            .join(", "),
        None => "(no model)".to_string(),
    }
}

fn i64const<'c>(ctx: &'c Context, v: i64) -> ZInt<'c> {
    ZInt::from_i64(ctx, v)
}

/// Boolean-valued expression translation; Err on unsupported mention.
fn bool_of<'c>(
    ctx: &'c Context,
    e: &Expr,
    vars: &[(String, ZInt<'c>)],
    env: &mut HashMap<String, ZInt<'c>>,
) -> Result<ZBool<'c>, ()> {
    match e {
        Expr::BoolLit(v) => Ok(ZBool::from_bool(ctx, *v)),
        Expr::BinOp(op, l, r) => match op {
            BinOp::And => {
                let a = bool_of(ctx, l, vars, env)?;
                let b = bool_of(ctx, r, vars, env)?;
                Ok(ZBool::and(ctx, &[&a, &b]))
            }
            BinOp::Or => {
                let a = bool_of(ctx, l, vars, env)?;
                let b = bool_of(ctx, r, vars, env)?;
                Ok(ZBool::or(ctx, &[&a, &b]))
            }
            cmp @ (BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge) => {
                let a = val_of(ctx, l, vars, env)?;
                let b = val_of(ctx, r, vars, env)?;
                Ok(match cmp {
                    BinOp::Eq => a._eq(&b),
                    BinOp::Ne => a._eq(&b).not(),
                    BinOp::Lt => a.lt(&b),
                    BinOp::Le => a.le(&b),
                    BinOp::Gt => a.gt(&b),
                    _ => a.ge(&b),
                })
            }
            _ => Err(()),
        },
        Expr::UnOp(crate::sketch::UnOp::Not, x) => Ok(bool_of(ctx, x, vars, env)?.not()),
        Expr::If(c, a, b) => {
            let cnd = bool_of(ctx, c, vars, env)?;
            let va = bool_of(ctx, a, vars, env)?;
            let vb = bool_of(ctx, b, vars, env)?;
            Ok(cnd.ite(&va, &vb))
        }
        Expr::Let(name, v, body) => {
            let vv = val_of(ctx, v, vars, env)?;
            env.insert(name.clone(), vv);
            let out = bool_of(ctx, body, vars, env);
            env.remove(name);
            out
        }
        _ => Err(()),
    }
}

/// Integer-valued expression translation; Err on unsupported mention.
fn val_of<'c>(
    ctx: &'c Context,
    e: &Expr,
    vars: &[(String, ZInt<'c>)],
    env: &mut HashMap<String, ZInt<'c>>,
) -> Result<ZInt<'c>, ()> {
    match e {
        Expr::IntLit(v) => Ok(i64const(ctx, *v)),
        Expr::Var(n) => {
            if let Some((_, x)) = vars.iter().find(|(vn, _)| vn == n) {
                return Ok(x.clone());
            }
            env.get(n).cloned().ok_or(())
        }
        Expr::BinOp(op @ (BinOp::Add | BinOp::Sub | BinOp::Mul), l, r) => {
            let a = val_of(ctx, l, vars, env)?;
            let b = val_of(ctx, r, vars, env)?;
            Ok(match op {
                BinOp::Add => ZInt::add(ctx, &[&a, &b]),
                BinOp::Sub => ZInt::sub(ctx, &[&a, &b]),
                _ => ZInt::mul(ctx, &[&a, &b]),
            })
        }
        Expr::UnOp(crate::sketch::UnOp::Neg, x) => {
            let a = val_of(ctx, x, vars, env)?;
            Ok(a.unary_minus())
        }
        Expr::If(c, a, b) => {
            let cnd = bool_of(ctx, c, vars, env)?;
            let ia = val_of(ctx, a, vars, env)?;
            let ib = val_of(ctx, b, vars, env)?;
            Ok(cnd.ite(&ia, &ib))
        }
        Expr::Let(name, v, body) => {
            let vv = val_of(ctx, v, vars, env)?;
            env.insert(name.clone(), vv);
            let out = val_of(ctx, body, vars, env);
            env.remove(name);
            out
        }
        _ => Err(()),
    }
}

/// Accumulate one safety-negation term per arithmetic site into `out`.
/// Conditions mirror interp.rs exactly:
/// - Add/Sub/Mul: result < i64::MIN ∨ result > i64::MAX
/// - unary minus: operand == i64::MIN (checked_neg)
/// Operand translation failures simply omit that site's term — the
/// disjunction shrinks toward Unsat-harder, never toward fabricated proof.
fn collect_violations<'c>(
    ctx: &'c Context,
    e: &Expr,
    vars: &[(String, ZInt<'c>)],
    env: &mut HashMap<String, ZInt<'c>>,
    out: &mut Vec<ZBool<'c>>,
) {
    match e {
        Expr::BinOp(op @ (BinOp::Add | BinOp::Sub | BinOp::Mul), l, r) => {
            collect_violations(ctx, l, vars, env, out);
            collect_violations(ctx, r, vars, env, out);
            if let (Ok(a), Ok(b)) = (
                val_of(ctx, l, vars, env),
                val_of(ctx, r, vars, env),
            ) {
                let s = match op {
                    BinOp::Add => ZInt::add(ctx, &[&a, &b]),
                    BinOp::Sub => ZInt::sub(ctx, &[&a, &b]),
                    _ => ZInt::mul(ctx, &[&a, &b]),
                };
                let below = s.lt(&i64const(ctx, i64::MIN));
                let above = s.gt(&i64const(ctx, i64::MAX));
                out.push(ZBool::or(ctx, &[&below, &above]));
            }
        }
        Expr::BinOp(_, l, r) => {
            collect_violations(ctx, l, vars, env, out);
            collect_violations(ctx, r, vars, env, out);
        }
        Expr::UnOp(crate::sketch::UnOp::Neg, x) => {
            collect_violations(ctx, x, vars, env, out);
            if let Ok(a) = val_of(ctx, x, vars, env) {
                out.push(a._eq(&i64const(ctx, i64::MIN)));
            }
        }
        Expr::UnOp(_, x) => collect_violations(ctx, x, vars, env, out),
        Expr::Let(name, v, body) => {
            // Save/restore scoping so shadowed names never leak terms
            // across their own binding's horizon.
            let saved = env.get(name).cloned();
            if let Ok(vv) = val_of(ctx, v, vars, env) {
                env.insert(name.clone(), vv);
            }
            collect_violations(ctx, body, vars, env, out);
            match saved {
                Some(v) => {
                    env.insert(name.clone(), v);
                }
                None => {
                    env.remove(name);
                }
            }
        }
        Expr::If(c, a, b) => {
            collect_violations(ctx, c, vars, env, out);
            collect_violations(ctx, a, vars, env, out);
            collect_violations(ctx, b, vars, env, out);
        }
        _ => {}
    }
}

/// Unary minus is a checked op (traps iff operand == i64::MIN); bodies
/// with zero Add/Sub/Mul but some `-x` still need the solver query.
fn contains_neg(e: &Expr) -> bool {
    match e {
        Expr::UnOp(crate::sketch::UnOp::Neg, _) => true,
        Expr::BinOp(_, l, r) => contains_neg(l) || contains_neg(r),
        Expr::UnOp(_, x) => contains_neg(x),
        Expr::Let(_, v, b) => contains_neg(v) || contains_neg(b),
        Expr::If(c, a, b) => contains_neg(c) || contains_neg(a) || contains_neg(b),
        _ => false,
    }
}

/// Does the expression use shapes outside the proven-v1 subset?
fn unsupported_shape(e: &Expr) -> Option<String> {
    use crate::sketch::{BinOp as B, UnOp};
    match e {
        Expr::Var(_) | Expr::IntLit(_) | Expr::BoolLit(_) => None,
        Expr::BinOp(op, l, r)
            if matches!(
                op,
                B::Add | B::Sub | B::Mul | B::Eq | B::Ne | B::Lt | B::Le | B::Gt | B::Ge | B::And | B::Or
            ) =>
        {
            unsupported_shape(l).or_else(|| unsupported_shape(r))
        }
        Expr::UnOp(UnOp::Neg, x) => unsupported_shape(x),
        Expr::UnOp(UnOp::Not, x) => unsupported_shape(x),
        Expr::Let(_, v, b) => unsupported_shape(v).or_else(|| unsupported_shape(b)),
        Expr::If(c, a, b) => unsupported_shape(c)
            .or_else(|| unsupported_shape(a))
            .or_else(|| unsupported_shape(b)),
        other => Some(format!(
            "{} outside straight-line Int subset",
            crate::lower::expr_display(other)
        )),
    }
}

fn count_arith(e: &Expr) -> usize {
    use crate::sketch::BinOp as B;
    match e {
        Expr::BinOp(op, l, r)
            if matches!(op, B::Add | B::Sub | B::Mul | B::Div | B::Mod) =>
        {
            1 + count_arith(l) + count_arith(r)
        }
        Expr::BinOp(_, l, r) => count_arith(l) + count_arith(r),
        Expr::UnOp(_, x) => count_arith(x),
        Expr::Let(_, v, b) => count_arith(v) + count_arith(b),
        Expr::If(c, a, b) => count_arith(c) + count_arith(a) + count_arith(b),
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recipe;

    /// Parse a one-gen spec and its hand candidate for prover tests.
    fn setup(spec: &str, cand: &str) -> (Gen, Candidate) {
        let file = recipe::parse_ont(spec).expect("spec parse");
        let g = file.gens.into_iter().next().expect("one gen");
        let c = crate::sketch::parse(cand).expect("candidate parse");
        (g, c)
    }

    #[test]
    fn proven_bounded_multiply() {
        let (g, c) = setup(
            "fn T.dbl(%x: Int) -> Int\n  | %x >= 0\n  | %x <= 100\n  => 3 -> 6 ± 0\n",
            "fn @run(%x: Int) -> Int { %x * 2 }",
        );
        match proof_for(&g, &c) {
            Proof::Proven(_) => {}
            other => panic!("expected Proven, got {other:?}"),
        }
    }

    #[test]
    fn unproven_unbounded_witness() {
        let (g, c) = setup(
            "fn T.dbl(%x: Int) -> Int\n  | %x >= 0\n  => 3 -> 6 ± 0\n",
            "fn @run(%x: Int) -> Int { %x * 2 }",
        );
        match proof_for(&g, &c) {
            Proof::Unproven(why) => assert!(why.contains("%x="), "witness in: {why}"),
            other => panic!("expected Unproven, got {other:?}"),
        }
    }

    #[test]
    fn trivial_no_arithmetic() {
        let (g, c) = setup(
            "fn T.id(%x: Int) -> Int\n  | %x >= 0\n  | %x <= 10\n  => 3 -> 3 ± 0\n",
            "fn @run(%x: Int) -> Int { %x }",
        );
        match proof_for(&g, &c) {
            Proof::Proven(how) => assert!(how.contains("no arithmetic")),
            other => panic!("expected trivial Proven, got {other:?}"),
        }
    }

    #[test]
    fn div_family_excluded_conservatively() {
        let (g, c) = setup(
            "fn T.half(%x: Int) -> Int\n  | %x >= 1\n  | %x <= 100\n  => 4 -> 2 ± 0\n",
            "fn @run(%x: Int) -> Int { %x / 2 }",
        );
        match proof_for(&g, &c) {
            // Even with a tight domain, Euclidean/truncated divergence bars
            // v1 from proving division-family ops.
            Proof::Unproven(why) => assert!(why.contains("outside straight-line"), "{why}"),
            other => panic!("expected conservative Unproven, got {other:?}"),
        }
    }
}
