//! The oracle: tree-walking evaluator defining sketch semantics.
//! If this and the MLIR lowering disagree, this module wins (AGENTS.md rule 6).

use crate::sketch::{BinOp, Expr, UnOp};
use crate::wish::Value;
use std::collections::HashMap;

/// Evaluation failure — any of these kills a candidate at S3–S5.
#[derive(Debug, Clone, PartialEq)]
pub enum EvalError {
    DivByZero,
    ModByZero,
    Overflow,
    TypeError(String),
    Unbound(String),
}

impl std::fmt::Display for EvalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EvalError::DivByZero => write!(f, "division by zero"),
            EvalError::ModByZero => write!(f, "modulo by zero"),
            EvalError::Overflow => write!(f, "integer overflow"),
            EvalError::TypeError(m) => write!(f, "type error: {}", m),
            EvalError::Unbound(n) => write!(f, "unbound variable %{}", n),
        }
    }
}

pub type Env = HashMap<String, Value>;

/// Evaluation semantics tier. `wrapping` mirrors a wish's declared wrapping
/// clause: arithmetic wraps mod 2^64 and only division/modulo-by-zero are
/// errors. Default (checked) kills candidates on any overflow so probes can
/// expose unguarded paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ctx {
    pub wrapping: bool,
}

impl Ctx {
    pub fn checked() -> Self {
        Ctx { wrapping: false }
    }
    pub fn wrapping() -> Self {
        Ctx { wrapping: true }
    }
}

fn float_of(v: &Value) -> Result<f64, EvalError> {
    match v {
        Value::Float(x) => Ok(*x),
        other => Err(EvalError::TypeError(format!("expected F64, got {}", other))),
    }
}

fn int_of(v: &Value) -> Result<i64, EvalError> {
    match v {
        Value::Int(i) => Ok(*i),
        other => Err(EvalError::TypeError(format!("expected Int, got {}", other))),
    }
}

/// Evaluate `expr` under `env` in the default (checked) tier.
pub fn eval(expr: &Expr, env: &Env) -> Result<Value, EvalError> {
    eval_ctx(expr, env, Ctx::checked())
}

/// Evaluate `expr` under `env` with explicit semantics tier.
pub fn eval_ctx(expr: &Expr, env: &Env, ctx: Ctx) -> Result<Value, EvalError> {
    match expr {
        Expr::IntLit(v) => Ok(Value::Int(*v)),
        // IEEE semantics: inf/NaN propagate; only div/mod by zero on INTEGERS errors.
        Expr::FloatLit(v) => Ok(Value::Float(*v)),
        Expr::BoolLit(b) => Ok(Value::Bool(*b)),
        Expr::Var(n) => env
            .get(n)
            .cloned()
            .ok_or_else(|| EvalError::Unbound(n.clone())),
        Expr::ListLit(items) => Ok(Value::List(items.clone())),
        Expr::Len(inner) => match eval_ctx(inner, env, ctx)? {
            Value::List(vs) => Ok(Value::Int(vs.len() as i64)),
            Value::FloatList(vs) => Ok(Value::Int(vs.len() as i64)),
            other => Err(EvalError::TypeError(format!("len of non-list {}", other))),
        },
        Expr::UnOp(UnOp::Neg, inner) => {
            let inner_v = eval_ctx(inner, env, ctx)?;
            if let Value::Float(f) = inner_v {
                return Ok(Value::Float(-f));
            }
            let v = int_of(&inner_v)?;
            if ctx.wrapping {
                Ok(Value::Int(v.wrapping_neg()))
            } else {
                v.checked_neg().map(Value::Int).ok_or(EvalError::Overflow)
            }
        }
        Expr::UnOp(UnOp::Not, inner) => match eval_ctx(inner, env, ctx)? {
            Value::Bool(b) => Ok(Value::Bool(!b)),
            other => Err(EvalError::TypeError(format!("! on {}", other))),
        },
        Expr::If(c, t, e) => match eval_ctx(c, env, ctx)? {
            Value::Bool(true) => eval_ctx(t, env, ctx),
            Value::Bool(false) => eval_ctx(e, env, ctx),
            other => Err(EvalError::TypeError(format!("if cond is {}", other))),
        },
        Expr::Let(n, value, body) => {
            let v = eval_ctx(value, env, ctx)?;
            let mut scoped = env.clone();
            scoped.insert(n.clone(), v);
            eval_ctx(body, &scoped, ctx)
        }
        Expr::Fold {
            var,
            acc,
            list,
            init,
            body,
        } => eval_fold(var, acc, list, init, body, env, ctx),
        Expr::BinOp(op, l, r) => eval_binop(*op, l, r, env, ctx),
    }
}

/// Fold evaluates `init` once, then folds `list` left-to-right binding
/// `%var` (element) and `%acc` (running value) per step.
fn eval_fold(
    var: &str,
    acc: &str,
    list: &Expr,
    init: &Expr,
    body: &Expr,
    env: &Env,
    ctx: Ctx,
) -> Result<Value, EvalError> {
    let mut running = eval_ctx(init, env, ctx)?;
    let step = |env: &Env, item: Value, running: Value| -> Result<Value, EvalError> {
        let mut scoped = env.clone();
        scoped.insert(var.to_string(), item);
        scoped.insert(acc.to_string(), running);
        eval_ctx(body, &scoped, ctx)
    };
    match eval_ctx(list, env, ctx)? {
        Value::List(vs) => {
            for item in vs {
                running = step(env, Value::Int(item), running)?;
            }
        }
        Value::FloatList(vs) => {
            for item in vs {
                running = step(env, Value::Float(item), running)?;
            }
        }
        other => return Err(EvalError::TypeError(format!("fold over {}", other))),
    }
    Ok(running)
}

fn eval_binop(
    op: BinOp,
    l: &Expr,
    r: &Expr,
    env: &Env,
    ctx: Ctx,
) -> Result<Value, EvalError> {
    // Short-circuit booleans first.
    if matches!(op, BinOp::And | BinOp::Or) {
        let lv = match eval_ctx(l, env, ctx)? {
            Value::Bool(b) => b,
            other => return Err(EvalError::TypeError(format!("{} operand {}", op_str(op), other))),
        };
        if matches!(op, BinOp::And) && !lv {
            return Ok(Value::Bool(false));
        }
        if matches!(op, BinOp::Or) && lv {
            return Ok(Value::Bool(true));
        }
        return match eval_ctx(r, env, ctx)? {
            Value::Bool(b) => Ok(Value::Bool(b)),
            other => Err(EvalError::TypeError(format!("{} operand {}", op_str(op), other))),
        };
    }
    let lv = eval_ctx(l, env, ctx)?;
    let rv = eval_ctx(r, env, ctx)?;
    // IEEE fast path for float pairs: no overflow kills, div-by-zero -> inf/nan.
    if matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod)
        && matches!(lv, Value::Float(_))
        && matches!(rv, Value::Float(_))
    {
        let a = float_of(&lv)?;
        let b = float_of(&rv)?;
        let out = match op {
            BinOp::Add => a + b,
            BinOp::Sub => a - b,
            BinOp::Mul => a * b,
            BinOp::Div => a / b,
            _ => a % b,
        };
        return Ok(Value::Float(out));
    }
    match op {
        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
            let a = int_of(&lv)?;
            let b = int_of(&rv)?;
            let out = match (op, ctx.wrapping) {
                (BinOp::Add, true) => Some(a.wrapping_add(b)),
                (BinOp::Sub, true) => Some(a.wrapping_sub(b)),
                (BinOp::Mul, true) => Some(a.wrapping_mul(b)),
                (BinOp::Add, false) => a.checked_add(b),
                (BinOp::Sub, false) => a.checked_sub(b),
                (BinOp::Mul, false) => a.checked_mul(b),
                (_, _) if matches!(op, BinOp::Div) => {
                    if b == 0 {
                        return Err(EvalError::DivByZero);
                    }
                    a.checked_div(b)
                }
                (_, _) => {
                    if b == 0 {
                        return Err(EvalError::ModByZero);
                    }
                    // Wrapping remainder is the sign-of-dividend semantics
                    // matching x86 idiv — same as checked for in-domain values.
                    a.checked_rem(b)
                }
            };
            out.map(Value::Int).ok_or(EvalError::Overflow)
        }
        BinOp::Eq | BinOp::Ne => {
            let eq = match (&lv, &rv) {
                (Value::Int(a), Value::Int(b)) => a == b,
                (Value::Bool(a), Value::Bool(b)) => a == b,
                _ => {
                    return Err(EvalError::TypeError(format!(
                        "{} on {} vs {}",
                        op_str(op),
                        lv,
                        rv
                    )))
                }
            };
            Ok(Value::Bool(if matches!(op, BinOp::Eq) { eq } else { !eq }))
        }
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
            let (a, b) = (int_of(&lv)?, int_of(&rv)?);
            let out = match op {
                BinOp::Lt => a < b,
                BinOp::Le => a <= b,
                BinOp::Gt => a > b,
                _ => a >= b,
            };
            Ok(Value::Bool(out))
        }
        BinOp::And | BinOp::Or => Err(unreachable("short-circuit handled above")),
    }
}

fn unreachable(msg: &str) -> EvalError {
    EvalError::TypeError(format!("internal: {}", msg))
}

fn op_str(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Mod => "%",
        BinOp::Eq => "==",
        BinOp::Ne => "!=",
        BinOp::Lt => "<",
        BinOp::Le => "<=",
        BinOp::Gt => ">",
        BinOp::Ge => ">=",
        BinOp::And => "&&",
        BinOp::Or => "||",
    }
}

/// Convenience: evaluate a candidate body against positional inputs.
pub fn eval_candidate(
    candidate: &crate::sketch::Candidate,
    inputs: &[Value],
    ctx: Ctx,
) -> Result<Value, EvalError> {
    let mut env: Env = HashMap::new();
    for ((name, _), v) in candidate.params.iter().zip(inputs.iter()) {
        env.insert(name.clone(), v.clone());
    }
    eval_ctx(&candidate.body, &env, ctx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch;

    fn run(src: &str, inputs: &[Value]) -> Result<Value, EvalError> {
        let c = sketch::parse(src).expect("candidate parses");
        eval_candidate(&c, inputs, Ctx::checked())
    }

    #[test]
    fn test_fold_sum_semantics() {
        let v = run(
            "fn @t(%items: List<Int>) -> Int { fold %x in %items, %acc from 0 { %acc + %x } }",
            &[Value::List(vec![1, 2, 3])],
        )
        .expect("evals");
        assert_eq!(v, Value::Int(6));
    }

    #[test]
    fn test_fold_empty_returns_init() {
        let v = run(
            "fn @t(%items: List<Int>) -> Int { fold %x in %items, %acc from 42 { %acc + %x } }",
            &[Value::List(vec![])],
        )
        .expect("evals");
        assert_eq!(v, Value::Int(42));
    }

    #[test]
    fn test_div_by_zero_is_error_not_panic() {
        assert_eq!(
            run("fn @d(%a: Int) -> Int { %a / 0 }", &[Value::Int(7)]),
            Err(EvalError::DivByZero)
        );
    }

    #[test]
    fn test_overflow_is_error() {
        assert_eq!(
            run("fn @o(%a: Int) -> Int { %a * %a }", &[Value::Int(4_000_000_000)]),
            Err(EvalError::Overflow)
        );
    }

    #[test]
    fn test_short_circuit_and() {
        // LHS is false (%a != 0); RHS would divide by zero but must not evaluate.
        let v = run(
            "fn @s(%a: Int) -> Bool { %a == 0 && 1 / %a > 0 }",
            &[Value::Int(5)],
        )
        .expect("evals");
        assert_eq!(v, Value::Bool(false));
    }

    #[test]
    fn test_let_binding_scopes() {
        let v = run(
            "fn @l(%a: Int) -> Int { let %b = %a + 1; %b * %b }",
            &[Value::Int(3)],
        )
        .expect("evals");
        assert_eq!(v, Value::Int(16));
    }

    #[test]
    fn test_unbound_variable_detected() {
        assert!(matches!(
            run("fn @u(%a: Int) -> Int { %zz }", &[Value::Int(1)]),
            Err(EvalError::Unbound(_))
        ));
    }
}

#[cfg(test)]
mod float_tests {
    use super::*;
    use crate::sketch;

    #[test]
    fn test_ieee_division_propagates_inf() {
        // IEEE semantics: division by zero yields inf, not an error.
        let c = sketch::parse("fn @d(%a: F64) -> F64 { %a / 0.0 }").unwrap();
        let v = eval_candidate(&c, &[Value::Float(1.0)], Ctx::checked()).expect("evals");
        assert!(matches!(v, Value::Float(f) if f.is_infinite()));
    }

    #[test]
    fn test_int_div_zero_still_errors() {
        let c = sketch::parse("fn @i(%a: Int) -> Int { %a / 0 }").unwrap();
        assert_eq!(
            eval_candidate(&c, &[Value::Int(1)], Ctx::checked()),
            Err(EvalError::DivByZero)
        );
    }
}
