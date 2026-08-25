//! Stage S2: sketch typechecker. Rejects ill-typed candidates before any
//! evaluation, so the oracle only sees well-formed programs.

use crate::sketch::{BinOp, Builtin, Candidate, Expr, Ty, UnOp};
use std::collections::HashMap;

/// Dependency signatures for vault calls: path -> (param types, ret).
pub type DepSigs = HashMap<String, (Vec<Ty>, Ty)>;

#[allow(dead_code)]
fn unreachable_index() -> ! {
    panic!("internal: Index handled via builtin2")
}

/// Static type of a builtin application.
fn builtin_ty(b: Builtin, t: Ty) -> Result<Ty, String> {
    match b {
        Builtin::Len => match t {
            Ty::ListInt | Ty::ListF64 => Ok(Ty::Int),
            other => Err(format!("len of {}", other.name())),
        },
        Builtin::Index => unreachable_index(),
        Builtin::Range => match t {
            Ty::Int => Ok(Ty::ListInt),
            other => Err(format!("range of {}", other.name())),
        },
        Builtin::MinEl | Builtin::MaxEl => Err(format!(
            "{:?} is elementwise (two arguments)",
            b
        )),
        Builtin::Sum | Builtin::Max | Builtin::Min => match t {
            Ty::ListInt => Ok(Ty::Int),
            Ty::ListF64 => Ok(Ty::F64),
            other => Err(format!("{:?} of {}", b, other.name())),
        },
        // Numeric transforms are F64-only; Int arguments promote implicitly.
        Builtin::Sqrt | Builtin::Exp | Builtin::Log | Builtin::Abs => match t {
            Ty::Int | Ty::F64 => Ok(Ty::F64),
            other => Err(format!("numeric builtin on {}", other.name())),
        },
    }
}

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

/// Typecheck with declared vault dependencies available for call typing.
pub fn check_with(cand: &Candidate, deps: &DepSigs) -> Result<(), String> {
    let mut env: HashMap<String, Ty> = HashMap::new();
    for (n, t) in &cand.params {
        if env.insert(n.clone(), t.clone()).is_some() {
            return Err(format!("duplicate parameter %{}", n));
        }
    }
    for p in deps.keys() {
        if env.contains_key(p) {
            return Err(format!("dependency `{}` shadows a parameter", p));
        }
    }
    // Body inference with dep-aware Call typing.
    let body_ty = infer_dep(&cand.body, &env, deps)?;
    if body_ty != cand.ret {
        return Err(format!(
            "body has type {}, signature demands {}",
            body_ty.name(),
            cand.ret.name()
        ));
    }
    Ok(())
}

/// Dep-aware inference: identical to `infer` except Call arms consult
/// declared dependency signatures (with numeric widening into F64 params).
fn infer_dep(
    e: &Expr,
    env: &HashMap<String, Ty>,
    deps: &DepSigs,
) -> Result<Ty, String> {
    match e {
        Expr::Call(path, args) => typecheck_call(path, args, env, deps),
        Expr::Let(n, value, body) => {
            let vt = infer_dep(value, env, deps)?;
            let mut scoped = env.clone();
            scoped.insert(n.clone(), vt);
            infer_dep(body, &scoped, deps)
        }
        Expr::Fold {
            var,
            acc,
            list,
            init,
            body,
            ref until,
            ref aux,
        } => {
            let list_ty = infer_dep(list, env, deps)?;
            let elem = match list_ty {
                Ty::ListInt => Ty::Int,
                Ty::ListF64 => Ty::F64,
                other => return Err(format!("fold over {}", other.name())),
            };
            let init_ty = infer_dep(init, env, deps)?;
            let mut scoped = env.clone();
            scoped.insert(var.clone(), elem);
            scoped.insert(acc.clone(), init_ty.clone());
            let mut want = vec![init_ty.clone()];
            for (n, ie) in aux {
                let t = infer_dep(ie, env, deps)?;
                scoped.insert(n.clone(), t.clone());
                want.push(t);
            }
            if aux.is_empty() {
                expect_dep(body, &scoped, &init_ty, deps)?;
            } else {
                expect_tuple_components(body, &scoped, &want, deps)?;
            }
            if let Some(u) = until {
                expect_dep(u, &scoped, &Ty::Bool, deps)?;
            }
            Ok(init_ty)
        }
        Expr::Map { var, list, body } => {
            let list_ty = infer_dep(list, env, deps)?;
            let elem_ty = match list_ty {
                Ty::ListInt => Ty::Int,
                Ty::ListF64 => Ty::F64,
                other => return Err(format!("map over {}", other.name())),
            };
            let mut scoped = env.clone();
            scoped.insert(var.clone(), elem_ty);
            let body_ty = infer_dep(body, &scoped, deps)?;
            Ok(if matches!(body_ty, Ty::F64) { Ty::ListF64 } else { Ty::ListInt })
        }
        Expr::If(c, t, f) => {
            expect_ty_in(c, env, &Ty::Bool)?;
            let tt = infer_dep(t, env, deps)?;
            let ft = infer_dep(f, env, deps)?;
            if tt != ft {
                return Err(format!(
                    "if branches disagree: {} vs {}",
                    tt.name(),
                    ft.name()
                ));
            }
            Ok(tt)
        }
        Expr::UnOp(UnOp::Neg, inner) => {
            let t = infer_dep(inner, env, deps)?;
            match t {
                Ty::Int | Ty::F64 => Ok(t),
                other => Err(format!("neg on {}", other.name())),
            }
        }
        Expr::UnOp(UnOp::Not, inner) => expect_ty_in(inner, env, &Ty::Bool),
        Expr::BinOp(op, l, r) => infer_binop(*op, l, r, env, deps),
        leaf => infer(leaf, env),
    }
}

/// Type a restricted tuple body against expected component types.
fn expect_tuple_components(
    body: &Expr,
    env: &HashMap<String, Ty>,
    want: &[Ty],
    deps: &DepSigs,
) -> Result<(), String> {
    match body {
        Expr::Tuple(items) => {
            if items.len() != want.len() {
                return Err(format!(
                    "fold body yields {} values, accumulators want {}",
                    items.len(),
                    want.len()
                ));
            }
            for (it, w) in items.iter().zip(want.iter()) {
                expect_dep(it, env, w, deps)?;
            }
            Ok(())
        }
        _ => Err(
            "multi-accumulator fold bodies must be tuple expressions `(a, b, ...)`"
                .to_string(),
        ),
    }
}

fn expect_dep(
    e: &Expr,
    env: &HashMap<String, Ty>,
    want: &Ty,
    deps: &DepSigs,
) -> Result<Ty, String> {
    let got = infer_dep(e, env, deps)?;
    if got != *want {
        return Err(format!("expected {}, got {}", want.name(), got.name()));
    }
    Ok(got)
}

/// Typecheck one vault call against declared dependency signatures.
fn typecheck_call(
    path: &str,
    args: &[Expr],
    env: &HashMap<String, Ty>,
    deps: &DepSigs,
) -> Result<Ty, String> {
    let (want_ps, want_rt) = deps.get(path).ok_or_else(|| {
        if std::env::var("ONTIC_DEBUG").is_ok() {
            eprintln!("DEBUG dep-miss: asked `{}` have {:?}", path, deps.keys().collect::<Vec<_>>());
        }
        format!("call to undeclared dependency `{}`", path)
    })?;
    if args.len() != want_ps.len() {
        return Err(format!(
            "`{}` expects {} args, got {}",
            path,
            want_ps.len(),
            args.len()
        ));
    }
    for (i, (a, w)) in args.iter().zip(want_ps.iter()).enumerate() {
        let got = infer_dep(a, env, deps)?;
        let numeric_pair = matches!(
            (got, *w),
            (Ty::Int | Ty::F64, Ty::Int | Ty::F64)
        );
        let ok = got == *w || (numeric_pair && matches!(w, Ty::F64));
        if !ok {
            return Err(format!(
                "`{}` arg #{} wants {}, got {}",
                path,
                i + 1,
                w.name(),
                got.name()
            ));
        }
    }
    Ok(*want_rt)
}

/// Infer the type of `e` under `env` (let/fold scopes included).
fn infer(e: &Expr, env: &HashMap<String, Ty>) -> Result<Ty, String> {
    let _ = env;
    match e {
        // Bare infer has no dep context: honest error unless check_with used.
        Expr::Call(p, _) => Err(format!(
            "call to `{}` requires declared `use` dependency (checker context missing)",
            p
        )),
        Expr::IntLit(_) => Ok(Ty::Int),
        Expr::FloatLit(_) => Ok(Ty::F64),
        Expr::BoolLit(_) => Ok(Ty::Bool),
        Expr::ListLit(items) if items.is_empty() => Ok(Ty::ListF64),
        Expr::ListLit(_) => Ok(Ty::ListInt),
        Expr::FloatListLit(_) => Ok(Ty::ListF64),
        Expr::Var(n) => env
            .get(n)
            .cloned()
            .ok_or_else(|| format!("unbound variable %{}", n)),
        Expr::Builtin(b, inner) => {
            let t = infer(inner, env)?;
            builtin_ty(*b, t)
        }
        Expr::Map { var, list, body } => {
            let list_ty = infer(list, env)?;
            let elem = match list_ty {
                Ty::ListInt => Ty::Int,
                Ty::ListF64 => Ty::F64,
                other => return Err(format!("map over {}", other.name())),
            };
            let mut scoped = env.clone();
            scoped.insert(var.clone(), elem);
            let body_ty = infer(body, &scoped)?;
            Ok(if matches!(body_ty, Ty::F64) { Ty::ListF64 } else { Ty::ListInt })
        }
        Expr::Builtin2(b, l, r) => infer_builtin2(*b, l, r, env),
        Expr::ListCons(elems) => {
            if elems.is_empty() {
                return Ok(Ty::ListInt); // empty list defaults to Int
            }
            let first = infer(&elems[0], env)?;
            let elem_ty = match first {
                Ty::Int | Ty::F64 => first,
                other => return Err(format!("list-cons element {}", other.name())),
            };
            for e in &elems[1..] {
                let t = infer(e, env)?;
                match (t.clone(), elem_ty.clone()) {
                    (Ty::Int, Ty::F64) | (Ty::F64, Ty::Int) | (Ty::F64, Ty::F64) => {}
                    (Ty::Int, Ty::Int) => {}
                    _ => return Err(format!(
                        "list-cons type mismatch: {} vs {}",
                        t.name(), elem_ty.name()
                    )),
                }
            }
            Ok(if elem_ty == Ty::Int { Ty::ListInt } else { Ty::ListF64 })
        }
        Expr::UnOp(UnOp::Neg, inner) => {
            // Interp defines float negation (interp handles -f); Int-only
            // here silently forced models through 0.0-x contortions.
            let t = infer(inner, env)?;
            match t {
                Ty::Int | Ty::F64 => Ok(t),
                other => Err(format!("neg on {}", other.name())),
            }
        }
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
            ref until,
            ref aux,
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
            let mut want = vec![init_ty.clone()];
            for (n, ie) in aux {
                let t = infer(ie, env)?;
                scoped.insert(n.clone(), t.clone());
                want.push(t);
            }
            if aux.is_empty() {
                expect_ty_in(body, &scoped, &init_ty)?;
            } else {
                // Plain-path tuple check via per-component expect.
                match &**body {
                    Expr::Tuple(items) => {
                        if items.len() != want.len() {
                            return Err(format!(
                                "fold body yields {} values, accumulators want {}",
                                items.len(),
                                want.len()
                            ));
                        }
                        for (it, w) in items.iter().zip(want.iter()) {
                            expect_ty_in(it, &scoped, w)?;
                        }
                    }
                    _ => return Err(
                        "multi-accumulator fold bodies must be tuple expressions `(a, b, ...)`"
                            .to_string(),
                    ),
                }
            }
            if let Some(u) = until {
                expect_ty_in(u, &scoped, &Ty::Bool)?;
            }
            Ok(init_ty)
        }
        Expr::BinOp(op, l, r) => {
            let no_deps = DepSigs::new();
            infer_binop(*op, l, r, env, &no_deps)
        }
        Expr::Tuple(_) => Err(
            "tuple expressions are only valid as multi-accumulator fold bodies".to_string(),
        ),
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

/// Typecheck binary builtins.
fn infer_builtin2(b: Builtin, l: &Expr, r: &Expr, env: &HashMap<String, Ty>) -> Result<Ty, String> {
    let lt = infer(l, env)?;
    let rt = infer(r, env)?;
    match b {
        Builtin::Index => {
            let elem = match lt {
                Ty::ListInt => Ty::Int,
                Ty::ListF64 => Ty::F64,
                other => return Err(format!("index of {}", other.name())),
            };
            if rt != Ty::Int {
                return Err(format!("index position must be Int, got {}", rt.name()));
            }
            Ok(elem)
        }
        Builtin::MinEl | Builtin::MaxEl => {
            if !matches!(lt, Ty::Int | Ty::F64) || !matches!(rt, Ty::Int | Ty::F64) {
                return Err(format!(
                    "{:?} needs two scalars, got {} vs {}",
                    b,
                    lt.name(),
                    rt.name()
                ));
            }
            Ok(if matches!(lt, Ty::F64) || matches!(rt, Ty::F64) {
                Ty::F64
            } else {
                Ty::Int
            })
        }
        _ => Err(format!("builtin {:?} is unary", b)),
    }
}

fn infer_binop(
    op: BinOp,
    l: &Expr,
    r: &Expr,
    env: &HashMap<String, Ty>,
    deps: &DepSigs,
) -> Result<Ty, String> {
    match op {
        BinOp::Concat => {
            let lt = infer_dep(l, env, deps)?;
            let rt = infer_dep(r, env, deps)?;
            match (&lt, &rt) {
                (Ty::ListInt, Ty::ListInt) => Ok(Ty::ListInt),
                (Ty::ListF64, _) | (_, Ty::ListF64)
                    if matches!(lt, Ty::ListF64 | Ty::F64 | Ty::Int) && matches!(rt, Ty::ListF64 | Ty::F64 | Ty::Int) =>
                    Ok(Ty::ListF64),
                _ => Err(format!("concat on {} vs {}", lt.name(), rt.name())),
            }
        }
        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
            let lt = infer_dep(l, env, deps)?;
            let rt = infer_dep(r, env, deps)?;
            // Numeric promotion: mixing Int with F64 widens to F64
            // (research-language convention, documented in AGENTS.md).
            // Broadcasting: list op scalar (or same-kind lists) maps
            // elementwise; Int scalars widen into F64 lists.
            match (&lt, &rt) {
                (Ty::Int, Ty::Int) => Ok(Ty::Int),
                (Ty::F64, Ty::Int) | (Ty::Int, Ty::F64) | (Ty::F64, Ty::F64) => Ok(Ty::F64),
                (Ty::ListInt, Ty::ListInt) => Ok(Ty::ListInt),
                (Ty::ListInt, Ty::Int) | (Ty::Int, Ty::ListInt) => Ok(Ty::ListInt),
                (Ty::ListF64, Ty::F64) | (Ty::F64, Ty::ListF64)
                | (Ty::ListF64, Ty::Int) | (Ty::Int, Ty::ListF64)
                | (Ty::ListF64, Ty::ListF64) => Ok(Ty::ListF64),
                (Ty::ListInt, Ty::F64) | (Ty::F64, Ty::ListInt) => Ok(Ty::ListF64),
                _ => Err(format!("arith on {} vs {}", lt.name(), rt.name())),
            }
        }
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
            let lt = infer_dep(l, env, deps)?;
            let rt = infer_dep(r, env, deps)?;
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
        // Equality over matching scalars; Int==F64 promotes like other
        // numeric ops (documented convention).
        BinOp::Eq | BinOp::Ne => {
            let lt = infer_dep(l, env, deps)?;
            let rt = infer_dep(r, env, deps)?;
            match (&lt, &rt) {
                (Ty::Int | Ty::F64, Ty::Int | Ty::F64) | (Ty::Bool, Ty::Bool) => Ok(Ty::Bool),
                _ => Err(format!(
                    "== needs equal scalar types, got {} vs {}",
                    lt.name(),
                    rt.name()
                )),
            }
        }
    }
}

/// Validate gen invariants against the signature environment (`%params` +
/// `%res`). A gen whose invariants do not typecheck is malformed spec, not a
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
    fn test_expression_list_parses_and_typechecks() {
        // With ListCons support, [%x] parses and typechecks.
        let c = sketch::parse("fn @t(%items: List<Int>) -> Int { len([1, 2]) }").unwrap();
        assert!(check(&c).is_ok());
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
