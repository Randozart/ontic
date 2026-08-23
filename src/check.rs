//! Stage S2: sketch typechecker. Rejects ill-typed candidates before any
//! evaluation, so the oracle only sees well-formed programs.

use crate::sketch::{BinOp, Candidate, Expr, Ty, UnOp};
use std::collections::HashMap;

/// Public type-inference entry: lowerers and tools query expression types
/// under a signature environment (params + optional %res).
pub fn infer_type(e: &Expr, env: &HashMap<String, Ty>) -> Result<Ty, String> {
    infer(e, env)
}

/// Typecheck a candidate against its own signature. Returns Err with a
/// human-readable reason on first mismatch.
pub fn check(cand: &Candidate) -> Result<(), String> {
    let mut env: HashMap<String, Ty> = HashMap::new();
    for (n, t) in &cand.params {
        if env.insert(n.clone(), t.clone()).is_some() {
            return Err(format!("duplicate parameter %{}", n));
        }
    }
    let body_ty = infer(&cand.body, &env)?;
    if body_ty != cand.ret {
        return Err(format!(
            "body has type {}, signature demands {}",
            body_ty.name(),
            cand.ret.name()
        ));
    }
    Ok(())
}

/// Infer the type of `e` under `env` (let/fold scopes included).
fn infer(e: &Expr, env: &HashMap<String, Ty>) -> Result<Ty, String> {
    match e {
        Expr::IntLit(_) => Ok(Ty::Int),
        Expr::FloatLit(_) => Ok(Ty::F64),
        Expr::BoolLit(_) => Ok(Ty::Bool),
        Expr::ListLit(_) => Ok(Ty::ListInt),
        Expr::Var(n) => env
            .get(n)
            .cloned()
            .ok_or_else(|| format!("unbound variable %{}", n)),
        Expr::Len(inner) => {
            let t = infer(inner, env)?;
            match t {
                Ty::ListInt | Ty::ListF64 => Ok(Ty::Int),
                other => Err(format!("len of {}", other.name())),
            }
        }
        Expr::UnOp(UnOp::Neg, inner) => expect_ty(inner, env, &Ty::Int),
        Expr::UnOp(UnOp::Not, inner) => expect_ty(inner, env, &Ty::Bool),
        Expr::If(c, t, f) => {
            expect_ty(c, env, &Ty::Bool)?;
            let tt = infer(t, env)?;
            let ft = infer(f, env)?;
            if tt != ft {
                return Err(format!(
                    "if branches disagree: {} vs {}",
                    tt.name(),
                    ft.name()
                ));
            }
            Ok(tt)
        }
        Expr::Let(n, value, body) => {
            let vt = infer(value, env)?;
            let mut scoped = env.clone();
            scoped.insert(n.clone(), vt);
            infer(body, &scoped)
        }
        Expr::Fold {
            var,
            acc,
            list,
            init,
            body,
        } => {
            let list_ty = infer(list, env)?;
            let elem_ty = match list_ty {
                Ty::ListInt | Ty::ListF64 => Ok(Ty::Int),
                ref other => Err(format!("fold over {}", other.name())),
            }?;
            // Element type follows the list: List<F64> folds bind %v : F64.
            let elem_ty = if matches!(list_ty, Ty::ListF64) {
                Ty::F64
            } else {
                elem_ty
            };
            let init_ty = infer(init, env)?;
            let mut scoped = env.clone();
            scoped.insert(var.clone(), elem_ty);
            scoped.insert(acc.clone(), init_ty.clone());
            expect_ty_in(body, &scoped, &init_ty)
        }
        Expr::BinOp(op, l, r) => infer_binop(*op, l, r, env),
    }
}

fn expect_ty(e: &Expr, env: &HashMap<String, Ty>, want: &Ty) -> Result<Ty, String> {
    expect_ty_in(e, env, want)
}

fn expect_ty_in(e: &Expr, env: &HashMap<String, Ty>, want: &Ty) -> Result<Ty, String> {
    let got = infer(e, env)?;
    if got != *want {
        return Err(format!("expected {}, got {}", want.name(), got.name()));
    }
    Ok(got)
}

fn infer_binop(op: BinOp, l: &Expr, r: &Expr, env: &HashMap<String, Ty>) -> Result<Ty, String> {
    match op {
        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
            let lt = infer(l, env)?;
            let rt = infer(r, env)?;
            // Numeric promotion: mixing Int with F64 widens to F64
            // (research-language convention, documented in AGENTS.md).
            match (&lt, &rt) {
                (Ty::Int, Ty::Int) => Ok(Ty::Int),
                (Ty::F64, _) | (_, Ty::F64)
                    if matches!(lt, Ty::Int | Ty::F64) && matches!(rt, Ty::Int | Ty::F64) =>
                {
                    Ok(Ty::F64)
                }
                _ => Err(format!("arith on {} vs {}", lt.name(), rt.name())),
            }
        }
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
            let lt = infer(l, env)?;
            let rt = infer(r, env)?;
            match (&lt, &rt) {
                (Ty::Int | Ty::F64, Ty::Int | Ty::F64) => Ok(Ty::Bool),
                _ => Err(format!("compare {} vs {}", lt.name(), rt.name())),
            }
        }
        BinOp::And | BinOp::Or => {
            expect_ty(l, env, &Ty::Bool)?;
            expect_ty(r, env, &Ty::Bool)?;
            Ok(Ty::Bool)
        }
        // Equality is polymorphic over matching scalar types (v0: Int|Bool).
        BinOp::Eq | BinOp::Ne => {
            let lt = infer(l, env)?;
            let rt = infer(r, env)?;
            if lt != rt || matches!(lt, Ty::ListInt) {
                return Err(format!("== needs equal scalar types, got {} vs {}", lt.name(), rt.name()));
            }
            Ok(Ty::Bool)
        }
    }
}

/// Validate wish invariants against the signature environment (`%params` +
/// `%res`). A wish whose invariants do not typecheck is malformed spec, not a
/// candidate failure — reported before any sampling happens.
pub fn check_invariants(
    invariants: &[Expr],
    params: &[(String, Ty)],
    ret: &Ty,
) -> Result<(), String> {
    let mut env: HashMap<String, Ty> = HashMap::new();
    for (n, t) in params {
        env.insert(n.clone(), t.clone());
    }
    env.insert("res".to_string(), ret.clone());
    for inv in invariants {
        let got = infer(inv, &env)?;
        if got != Ty::Bool {
            return Err(format!(
                "invariant `{}` has type {}, must be Bool",
                crate::lower::expr_display(inv),
                got.name()
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch;

    #[test]
    fn test_well_typed_sum_passes() {
        let c = sketch::parse(
            "fn @t(%items: List<Int>) -> Int { fold %x in %items, %acc from 0 { %acc + %x } }",
        )
        .unwrap();
        assert!(check(&c).is_ok());
    }

    #[test]
    fn test_body_type_mismatch_rejected() {
        let c = sketch::parse("fn @t(%items: List<Int>) -> Bool { len(%items) }").unwrap();
        assert!(check(&c).is_err());
    }

    #[test]
    fn test_fold_element_must_be_int_use() {
        let c = sketch::parse("fn @t(%items: List<Int>) -> Int { fold %x in %items, %acc from 0 { %acc + len([%x]) } }").unwrap_err();
        // len() takes an expression; [%x] is not valid syntax — parse must fail.
        assert!(matches!(c, crate::sketch::ParseError { .. }));
    }

    #[test]
    fn test_if_branch_disagreement_rejected() {
        let c = sketch::parse("fn @i(%a: Int) -> Int { if %a > 0 { 1 } else { false } }").unwrap();
        assert!(check(&c).is_err());
    }

    #[test]
    fn test_list_equality_rejected_v0() {
        let c = sketch::parse("fn @e(%a: List<Int>) -> Bool { %a == [1] }").unwrap();
        assert!(check(&c).is_err());
    }
}
