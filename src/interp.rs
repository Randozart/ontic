//! The oracle: tree-walking evaluator defining sketch semantics.
//! If this and the MLIR lowering disagree, this module wins (AGENTS.md rule 6).

use crate::sketch::{BinOp, Builtin, Expr, UnOp};
use crate::gen::Value;
use std::collections::HashMap;

/// Evaluation failure — any of these kills a candidate at S3–S5.
#[derive(Debug, Clone, PartialEq)]
pub enum EvalError {
    DivByZero,
    IndexOutOfBounds(i64),
    ModByZero,
    Overflow,
    TypeError(String),
    Unbound(String),
}

impl std::fmt::Display for EvalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EvalError::DivByZero => write!(f, "division by zero"),
            EvalError::IndexOutOfBounds(p) => write!(f, "index {} out of bounds", p),
            EvalError::ModByZero => write!(f, "modulo by zero"),
            EvalError::Overflow => write!(f, "integer overflow"),
            EvalError::TypeError(m) => write!(f, "type error: {}", m),
            EvalError::Unbound(n) => write!(f, "unbound variable %{}", n),
        }
    }
}

pub type Env = HashMap<String, Value>;

/// Evaluation semantics tier. `wrapping` mirrors a gen's declared wrapping
/// clause: arithmetic wraps mod 2^64 and only division/modulo-by-zero are
/// errors. Default (checked) kills candidates on any overflow so probes can
/// expose unguarded paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tier {
    pub wrapping: bool,
}

impl Tier {
    pub fn checked() -> Self {
        Tier { wrapping: false }
    }
    pub fn wrapping() -> Self {
        Tier { wrapping: true }
    }
}

/// A resolvable vault dependency: its parsed candidate plus its own tier.
#[derive(Debug, Clone)]
pub struct DepFn {
    pub cand: crate::sketch::Candidate,
    pub tier: Tier,
}

pub type DepMap = std::collections::HashMap<String, DepFn>;

/// Runtime context: semantics tier + resolvable vault dependencies.
#[derive(Debug, Clone)]
pub struct Ctx {
    pub tier: Tier,
    pub deps: std::sync::Arc<DepMap>,
}

impl Ctx {
    pub fn checked() -> Self {
        Ctx {
            tier: Tier::checked(),
            deps: std::sync::Arc::new(DepMap::new()),
        }
    }
    pub fn wrapping() -> Self {
        Ctx {
            tier: Tier::wrapping(),
            deps: std::sync::Arc::new(DepMap::new()),
        }
    }
    /// Tier-only constructor sharing the empty dep table.
    pub fn of(tier: Tier) -> Self {
        Ctx {
            tier,
            deps: std::sync::Arc::new(DepMap::new()),
        }
    }
    pub fn is_wrapping(&self) -> bool {
        self.tier.wrapping
    }
}

fn float_of(v: &Value) -> Result<f64, EvalError> {
    match v {
        Value::Float(x) => Ok(*x),
        other => Err(EvalError::TypeError(format!("expected F64, got {}", other))),
    }
}

/// Some(F64 value) when this operand participates in promotion: floats pass
/// through; ints widen. Bools/lists never promote.
fn as_promotable(v: &Value) -> Option<f64> {
    match v {
        Value::Float(f) => Some(*f),
        Value::Int(i) => Some(*i as f64),
        _ => None,
    }
}


/// Unary builtin semantics. sum/max/min over lists; numeric transforms are
/// IEEE F64 (Int args promote). max/min on empty lists are errors — there is
/// no honest answer; probes expose them like division by zero.
fn eval_builtin(b: Builtin, inner: &Expr, env: &Env, ctx: &Ctx) -> Result<Value, EvalError> {
    let v = eval_ctx(inner, env, ctx)?;
    match b {
        Builtin::Range => match v {
            Value::Int(n) => {
                if n < 0 || n > 10_000_000 {
                    return Err(EvalError::Overflow);
                }
                Ok(Value::List((0..n).collect()))
            }
            other => Err(EvalError::TypeError(format!("range of {}", other))),
        },
        // Unary evaluation never sees Index (binary).
        Builtin::Index => Err(EvalError::TypeError("internal: index".to_string())),
        Builtin::Len => match v {
            Value::List(vs) => Ok(Value::Int(vs.len() as i64)),
            Value::FloatList(vs) => Ok(Value::Int(vs.len() as i64)),
            other => Err(EvalError::TypeError(format!("len of non-list {}", other))),
        },
        Builtin::Sum => match v {
            Value::List(vs) => Ok(Value::Int(vs.iter().sum())),
            Value::FloatList(vs) => Ok(Value::Float(vs.iter().sum())),
            other => Err(EvalError::TypeError(format!("sum of {}", other))),
        },
        Builtin::Max | Builtin::Min => {
            let take = |items: Vec<i64>| -> Result<Value, EvalError> {
                if items.is_empty() {
                    return Err(EvalError::TypeError("max/min of empty list".to_string()));
                }
                let m = if matches!(b, Builtin::Max) {
                    items.iter().max().copied()
                } else {
                    items.iter().min().copied()
                };
                m.map(Value::Int).ok_or_else(|| EvalError::Overflow)
            };
            match v {
                Value::List(vs) => take(vs),
                Value::FloatList(vs) => {
                    if vs.is_empty() {
                        return Err(EvalError::TypeError(
                            "max/min of empty list".to_string(),
                        ));
                    }
                    let m = if matches!(b, Builtin::Max) {
                        vs.iter().cloned().fold(f64::NEG_INFINITY, f64::max)
                    } else {
                        vs.iter().cloned().fold(f64::INFINITY, f64::min)
                    };
                    Ok(Value::Float(m))
                }
                other => Err(EvalError::TypeError(format!("max/min of {}", other))),
            }
        }
        Builtin::Sqrt | Builtin::Exp | Builtin::Log | Builtin::Abs => {
            let x = match v {
                Value::Float(f) => f,
                Value::Int(i) => i as f64,
                other => {
                    return Err(EvalError::TypeError(format!(
                        "numeric builtin on {}",
                        other
                    )))
                }
            };
            let out = match b {
                Builtin::Sqrt => x.sqrt(),
                Builtin::Exp => x.exp(),
                Builtin::Log => x.ln(),
                _ => x.abs(),
            };
            Ok(Value::Float(out))
        }
    }
}


/// Elementwise broadcast arithmetic. Scalar operands apply to every element;
/// both-list forms zip (length mismatch is an error). Int elements widen
/// inside F64 broadcasts.
fn eval_broadcast(
    op: BinOp,
    l: &Value,
    r: &Value,
) -> Option<Result<Value, EvalError>> {
    let f = |a: f64, b: f64| -> f64 {
        match op {
            BinOp::Add => a + b,
            BinOp::Sub => a - b,
            BinOp::Mul => a * b,
            BinOp::Div => a / b,
            _ => a % b,
        }
    };
    let i = |a: i64, b: i64| -> Result<i64, EvalError> {
        match op {
            BinOp::Add => a.checked_add(b).ok_or(EvalError::Overflow),
            BinOp::Sub => a.checked_sub(b).ok_or(EvalError::Overflow),
            BinOp::Mul => a.checked_mul(b).ok_or(EvalError::Overflow),
            BinOp::Div => {
                if b == 0 {
                    Err(EvalError::DivByZero)
                } else {
                    a.checked_div(b).ok_or(EvalError::Overflow)
                }
            }
            _ => {
                if b == 0 {
                    Err(EvalError::ModByZero)
                } else {
                    a.checked_rem(b).ok_or(EvalError::Overflow)
                }
            }
        }
    };
    let widen = |xs: &[i64]| -> Vec<f64> { xs.iter().map(|x| *x as f64).collect() };
    match (l, r) {
        (Value::List(a), Value::List(b)) => {
            if a.len() != b.len() {
                return Some(Err(EvalError::TypeError(format!(
                    "broadcast length mismatch: {} vs {}",
                    a.len(),
                    b.len()
                ))));
            }
            let out: Result<Vec<i64>, EvalError> =
                a.iter().zip(b.iter()).map(|(x, y)| i(*x, *y)).collect();
            Some(out.map(Value::List))
        }
        (Value::FloatList(a), Value::FloatList(b)) => {
            if a.len() != b.len() {
                return Some(Err(EvalError::TypeError(format!(
                    "broadcast length mismatch: {} vs {}",
                    a.len(),
                    b.len()
                ))));
            }
            Some(Ok(Value::FloatList(
                a.iter().zip(b.iter()).map(|(x, y)| f(*x, *y)).collect(),
            )))
        }
        (Value::List(a), scalar) | (scalar, Value::List(a))
            if matches!(scalar, Value::Int(_) | Value::Float(_)) =>
        {
            match (op, scalar) {
                (_, Value::Int(s)) => {
                    let out: Result<Vec<i64>, EvalError> =
                        a.iter().map(|x| i(*x, *s)).collect();
                    Some(out.map(Value::List))
                }
                (_, Value::Float(s)) => {
                    let xf = widen(a);
                    Some(Ok(Value::FloatList(
                        xf.iter().map(|x| f(*x, *s)).collect(),
                    )))
                }
                _ => None,
            }
        }
        (Value::FloatList(a), scalar) | (scalar, Value::FloatList(a))
            if matches!(scalar, Value::Int(_) | Value::Float(_)) =>
        {
            let s = match scalar {
                Value::Int(x) => *x as f64,
                Value::Float(x) => *x,
                _ => return None,
            };
            Some(Ok(Value::FloatList(
                a.iter().map(|x| f(*x, s)).collect(),
            )))
        }
        _ => None,
    }
}

/// Resolve a vault call: bind args against the dep's parameters, evaluate
/// the dep's stored candidate under the DEP'S OWN tier.
fn eval_call(
    path: &str,
    args: &[Expr],
    env: &Env,
    ctx: &Ctx,
) -> Result<Value, EvalError> {
    let dep = ctx
        .deps
        .get(path)
        .ok_or_else(|| EvalError::TypeError(format!("undeclared dependency `{}`", path)))?;
    let params: Vec<Value> = args
        .iter()
        .map(|a| eval_ctx(a, env, ctx))
        .collect::<Result<_, _>>()?;
    if params.len() != dep.cand.params.len() {
        return Err(EvalError::TypeError(format!(
            "`{}` arity {} != {}",
            path,
            params.len(),
            dep.cand.params.len()
        )));
    }
    let mut dep_env: Env = Env::new();
    for ((n, _), v) in dep.cand.params.iter().zip(params.iter()) {
        dep_env.insert(n.clone(), v.clone());
    }
    eval_ctx(&dep.cand.body, &dep_env, ctx)
}

/// Binary builtin semantics. Index is bounds-CHECKED: out-of-bounds is an
/// error exactly like division by zero, so probes expose it and native
/// traps match (parity rule).
fn eval_builtin2(
    b: Builtin,
    l: &Expr,
    r: &Expr,
    env: &Env,
    ctx: &Ctx,
) -> Result<Value, EvalError> {
    match b {
        Builtin::Index => {
            let list = eval_ctx(l, env, ctx)?;
            let pos = int_of(&eval_ctx(r, env, ctx)?)?;
            match list {
                Value::List(vs) => vs
                    .get(pos as usize)
                    .map(|v| Value::Int(*v))
                    .ok_or(EvalError::IndexOutOfBounds(pos)),
                Value::FloatList(vs) => vs
                    .get(pos as usize)
                    .map(|v| Value::Float(*v))
                    .ok_or(EvalError::IndexOutOfBounds(pos)),
                other => Err(EvalError::TypeError(format!("index of {}", other))),
            }
        }
        _ => Err(EvalError::TypeError("binary builtin".to_string())),
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
    eval_ctx(expr, env, &Ctx::checked())
}

/// Evaluate `expr` under `env` with explicit semantics tier + dep table.
pub fn eval_ctx(expr: &Expr, env: &Env, ctx: &Ctx) -> Result<Value, EvalError> {
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
        Expr::FloatListLit(items) => Ok(Value::FloatList(items.clone())),
        Expr::Builtin(b, inner) => eval_builtin(*b, inner, env, ctx),
        Expr::Builtin2(b, l, r) => eval_builtin2(*b, l, r, env, ctx),
        Expr::Call(p, args) => eval_call(p, args, env, ctx),
        Expr::UnOp(UnOp::Neg, inner) => {
            let inner_v = eval_ctx(inner, env, ctx)?;
            if let Value::Float(f) = inner_v {
                return Ok(Value::Float(-f));
            }
            let v = int_of(&inner_v)?;
            if ctx.is_wrapping() {
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
    ctx: &Ctx,
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
    ctx: &Ctx,
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
    // Numeric promotion: Int operands widen into any F64 operation or
    // ordered comparison (language convention, matches check.rs).
    let lv_f = as_promotable(&lv);
    let rv_f = as_promotable(&rv);
    if let (Some(a), Some(b)) = (lv_f, rv_f) {
        // Float semantics fire whenever ANY operand is F64 (mixed widens,
        // pure-float compares/arithmetic are IEEE). Pure-Int stays exact.
        let any_float =
            matches!(lv, Value::Float(_)) || matches!(rv, Value::Float(_));
        if any_float {
            return match op {
                BinOp::Add => Ok(Value::Float(a + b)),
                BinOp::Sub => Ok(Value::Float(a - b)),
                BinOp::Mul => Ok(Value::Float(a * b)),
                BinOp::Div => Ok(Value::Float(a / b)),
                BinOp::Mod => Ok(Value::Float(a % b)),
                BinOp::Lt => Ok(Value::Bool(a < b)),
                BinOp::Le => Ok(Value::Bool(a <= b)),
                BinOp::Gt => Ok(Value::Bool(a > b)),
                BinOp::Ge => Ok(Value::Bool(a >= b)),
                BinOp::Eq => Ok(Value::Bool(a == b)),
                BinOp::Ne => Ok(Value::Bool(a != b)),
                _ => Err(EvalError::TypeError(format!(
                    "{} on {} vs {}",
                    op_str(op),
                    lv,
                    rv
                ))),
            };
        }
    }
    if matches!(
        op,
        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod
    ) {
        if let Some(res) = eval_broadcast(op, &lv, &rv) {
            return res;
        }
    }
    match op {
        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
            let a = int_of(&lv)?;
            let b = int_of(&rv)?;
            let out = match (op, ctx.tier.wrapping) {
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
    ctx: &Ctx,
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
        eval_candidate(&c, inputs, &Ctx::checked())
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
        let v = eval_candidate(&c, &[Value::Float(1.0)], &Ctx::checked()).expect("evals");
        assert!(matches!(v, Value::Float(f) if f.is_infinite()));
    }

    #[test]
    fn test_int_div_zero_still_errors() {
        let c = sketch::parse("fn @i(%a: Int) -> Int { %a / 0 }").unwrap();
        assert_eq!(
            eval_candidate(&c, &[Value::Int(1)], &Ctx::checked()),
            Err(EvalError::DivByZero)
        );
    }
}

#[cfg(test)]
mod broadcast_tests {
    use super::*;
    use crate::sketch;

    #[test]
    fn test_list_plus_scalar_broadcast() {
        let c = sketch::parse("fn @b(%xs: List<Int>) -> List<Int> { %xs + 1 }").unwrap();
        let v = eval_candidate(&c, &[Value::List(vec![1, 2, 3])], &Ctx::checked())
            .unwrap();
        assert_eq!(v, Value::List(vec![2, 3, 4]));
    }

    #[test]
    fn test_float_list_times_int_scalar_widens() {
        let c = sketch::parse("fn @c(%xs: List<F64>) -> List<F64> { %xs * 2 }").unwrap();
        let v = eval_candidate(&c, &[Value::FloatList(vec![1.5, 2.5])], &Ctx::checked())
            .unwrap();
        assert_eq!(v, Value::FloatList(vec![3.0, 5.0]));
    }

    #[test]
    fn test_zip_broadcast_and_mismatch() {
        let c =
            sketch::parse("fn @z(%a: List<F64>, %b: List<F64>) -> List<F64> { %a - %b }").unwrap();
        let ok = eval_candidate(
            &c,
            &[Value::FloatList(vec![3.0, 4.0]), Value::FloatList(vec![1.0, 0.5])],
            &Ctx::checked(),
        )
        .unwrap();
        assert_eq!(ok, Value::FloatList(vec![2.0, 3.5]));
        assert!(eval_candidate(
            &c,
            &[Value::FloatList(vec![1.0]), Value::FloatList(vec![1.0, 2.0])],
            &Ctx::checked(),
        )
        .is_err());
    }

    #[test]
    fn test_sum_sqrt_builtins_semantics() {
        let c = sketch::parse("fn @s(%xs: List<F64>) -> F64 { sqrt(sum(%xs)) }").unwrap();
        let v = eval_candidate(
            &c,
            &[Value::FloatList(vec![4.0, 9.0, 16.0])],
            &Ctx::checked(),
        )
        .unwrap();
        assert_eq!(v, Value::Float(29.0f64.sqrt()));
    }
}

#[cfg(test)]
mod pr0_tests {
    use super::*;
    use crate::{check, sketch};

    #[test]
    fn test_index_in_bounds_and_oob() {
        let c = sketch::parse("fn @g(%xs: List<Int>, %i: Int) -> Int { index(%xs, %i) }").unwrap();
        let ok = eval_candidate(
            &c,
            &[Value::List(vec![7, 8]), Value::Int(1)],
            &Ctx::checked(),
        )
        .unwrap();
        assert_eq!(ok, Value::Int(8));
        let oob = eval_candidate(
            &c,
            &[Value::List(vec![7, 8]), Value::Int(5)],
            &Ctx::checked(),
        );
        assert!(matches!(oob, Err(EvalError::IndexOutOfBounds(5))));
    }

    #[test]
    fn test_range_builds_iota() {
        let c = sketch::parse("fn @r(%n: Int) -> List<Int> { range(%n) }").unwrap();
        let v = eval_candidate(&c, &[Value::Int(4)], &Ctx::checked()).unwrap();
        assert_eq!(v, Value::List(vec![0, 1, 2, 3]));
        // Negative ranges are rejected (probe-exposed like div-by-zero).
        assert!(eval_candidate(&c, &[Value::Int(-1)], &Ctx::checked()).is_err());
    }

    #[test]
    fn test_dot_via_range_index() {
        // The canonical D-track kernel shape.
        let c = sketch::parse(
            "fn @dot(%a: List<F64>, %b: List<F64>) -> F64 { fold %i in range(len(%a)), %acc from 0.0 { %acc + index(%a, %i) * index(%b, %i) } }",
        )
        .unwrap();
        check::check(&c).unwrap();
        let v = eval_candidate(
            &c,
            &[Value::FloatList(vec![1.0, 2.0]), Value::FloatList(vec![3.0, 4.0])],
            &Ctx::checked(),
        )
        .unwrap();
        assert_eq!(v, Value::Float(11.0));
    }
}
