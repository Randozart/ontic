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

fn int_of(v: &Value) -> Result<i64, EvalError> {
    match v {
        Value::Int(i) => Ok(*i),
        other => Err(EvalError::TypeError(format!("expected Int, got {}", other))),
    }
}

/// Evaluate `expr` under `env`. Checked arithmetic: overflow/div-zero are
/// hard errors so probes can expose candidates with unguarded paths.
pub fn eval(expr: &Expr, env: &Env) -> Result<Value, EvalError> {
    match expr {
        Expr::IntLit(v) => Ok(Value::Int(*v)),
        Expr::BoolLit(b) => Ok(Value::Bool(*b)),
        Expr::Var(n) => env
            .get(n)
            .cloned()
            .ok_or_else(|| EvalError::Unbound(n.clone())),
        Expr::ListLit(items) => Ok(Value::List(items.clone())),
        Expr::Len(inner) => match eval(inner, env)? {
            Value::List(vs) => Ok(Value::Int(vs.len() as i64)),
            other => Err(EvalError::TypeError(format!("len of non-list {}", other))),
        },
        Expr::UnOp(UnOp::Neg, inner) => int_of(&eval(inner, env)?)
            .and_then(|v| v.checked_neg().ok_or(EvalError::Overflow))
            .map(Value::Int),
        Expr::UnOp(UnOp::Not, inner) => match eval(inner, env)? {
            Value::Bool(b) => Ok(Value::Bool(!b)),
            other => Err(EvalError::TypeError(format!("! on {}", other))),
        },
        Expr::If(c, t, e) => match eval(c, env)? {
            Value::Bool(true) => eval(t, env),
            Value::Bool(false) => eval(e, env),
            other => Err(EvalError::TypeError(format!("if cond is {}", other))),
        },
        Expr::Let(n, value, body) => {
            let v = eval(value, env)?;
            let mut scoped = env.clone();
            scoped.insert(n.clone(), v);
            eval(body, &scoped)
        }
        Expr::Fold {
            var,
            acc,
            list,
            init,
            body,
        } => eval_fold(var, acc, list, init, body, env),
        Expr::BinOp(op, l, r) => eval_binop(*op, l, r, env),
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
) -> Result<Value, EvalError> {
    let items = match eval(list, env)? {
        Value::List(vs) => vs,
        other => return Err(EvalError::TypeError(format!("fold over {}", other))),
    };
    let mut running = eval(init, env)?;
    for item in items {
        let mut scoped = env.clone();
        scoped.insert(var.to_string(), Value::Int(item));
        scoped.insert(acc.to_string(), running);
        running = eval(body, &scoped)?;
    }
    Ok(running)
}

fn eval_binop(op: BinOp, l: &Expr, r: &Expr, env: &Env) -> Result<Value, EvalError> {
    // Short-circuit booleans first.
    if matches!(op, BinOp::And | BinOp::Or) {
        let lv = match eval(l, env)? {
            Value::Bool(b) => b,
            other => return Err(EvalError::TypeError(format!("{} operand {}", op_str(op), other))),
        };
        if matches!(op, BinOp::And) && !lv {
            return Ok(Value::Bool(false));
        }
        if matches!(op, BinOp::Or) && lv {
            return Ok(Value::Bool(true));
        }
        return match eval(r, env)? {
            Value::Bool(b) => Ok(Value::Bool(b)),
            other => Err(EvalError::TypeError(format!("{} operand {}", op_str(op), other))),
        };
    }
    let lv = eval(l, env)?;
    let rv = eval(r, env)?;
    match op {
        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
            let a = int_of(&lv)?;
            let b = int_of(&rv)?;
            let out = match op {
                BinOp::Add => a.checked_add(b),
                BinOp::Sub => a.checked_sub(b),
                BinOp::Mul => a.checked_mul(b),
                BinOp::Div => {
                    if b == 0 {
                        return Err(EvalError::DivByZero);
                    }
                    a.checked_div(b)
                }
                _ => {
                    if b == 0 {
                        return Err(EvalError::ModByZero);
                    }
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
) -> Result<Value, EvalError> {
    let mut env: Env = HashMap::new();
    for ((name, _), v) in candidate.params.iter().zip(inputs.iter()) {
        env.insert(name.clone(), v.clone());
    }
    eval(&candidate.body, &env)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch;

    fn run(src: &str, inputs: &[Value]) -> Result<Value, EvalError> {
        let c = sketch::parse(src).expect("candidate parses");
        eval_candidate(&c, inputs)
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
