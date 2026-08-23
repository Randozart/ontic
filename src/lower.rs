//! Lowering: sketch AST → MLIR text (arith + scf + memref), plus the
//! expression pretty-printer used in canonical wish text and diagnostics.
//!
//! Semantics rule: the interpreter (`interp.rs`) is the oracle. This module
//! projects exactly those semantics; differential tests pin them.
//!
//! Known projection note: `&&`/`||` lower to bitwise `andi`/`ori` (booleans
//! are 0/1 i64), not short-circuit control flow. Sketch programs have no side
//! effects, so divergence is only observable when a subexpression *errors*
//! (division/modulo by zero) — and any candidate whose errors are reachable
//! by probes is already rejected before lowering ever runs.

use crate::sketch::{BinOp, Builtin, Expr, Ty};
use std::collections::HashMap;

/// How to emit a call to one declared dependency.
#[derive(Debug, Clone)]
pub struct CallTarget {
    /// The function symbol inside the dependency's compiled module.
    pub symbol: String,
    pub params: Vec<Ty>,
    pub ret: Ty,
}

pub type CallMap = HashMap<String, CallTarget>;

/// Pretty-print an expression back to sketch surface syntax.
/// Used by canonical wish serialization and sieve diagnostics.
pub fn expr_display(e: &Expr) -> String {
    match e {
        Expr::IntLit(v) => v.to_string(),
        Expr::FloatLit(v) => format!("{:e}", v),
        Expr::BoolLit(b) => b.to_string(),
        Expr::Call(p, args) => format!(
            "{}({})",
            p,
            args.iter()
                .map(expr_display)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Expr::Var(n) => format!("%{}", n),
        Expr::ListLit(items) => {
            let inner: Vec<String> = items.iter().map(|v| v.to_string()).collect();
            format!("[{}]", inner.join(", "))
        }
        Expr::Builtin(b, i) => format!(
            "{}({})",
            match b {
                Builtin::Len => "len",
                Builtin::Sum => "sum",
                Builtin::Max => "max",
                Builtin::Min => "min",
                Builtin::Sqrt => "sqrt",
                Builtin::Exp => "exp",
                Builtin::Log => "log",
                Builtin::Abs => "abs",
            },
            expr_display(i)
        ),
        Expr::UnOp(crate::sketch::UnOp::Neg, i) => format!("(-{})", expr_display(i)),
        Expr::UnOp(crate::sketch::UnOp::Not, i) => format!("!{}", expr_display(i)),
        Expr::If(c, t, f) => format!(
            "if {} {{ {} }} else {{ {} }}",
            expr_display(c),
            expr_display(t),
            expr_display(f)
        ),
        Expr::Let(n, v, b) => format!("let %{} = {}; {}", n, expr_display(v), expr_display(b)),
        Expr::Fold {
            var,
            acc,
            list,
            init,
            body,
        } => format!(
            "(fold %{} in {}, %{} from {} {{ {} }})",
            var,
            expr_display(list),
            acc,
            expr_display(init),
            expr_display(body)
        ),
        Expr::BinOp(op, l, r) => format!(
            "({} {} {})",
            expr_display(l),
            binop_str(*op),
            expr_display(r)
        ),
    }
}

fn binop_str(op: BinOp) -> &'static str {
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

// ---------------------------------------------------------------------------
// MLIR emission
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Binding {
    name: String,
    ssa: String,
}

/// One-shot MLIR text emitter for a single candidate function.
struct Emitter {
    out: String,
    counter: usize,
    indent: usize,
    /// Wrapping tier: plain ops. Checked tier: widen-check-narrow expansion.
    wrapping: bool,
    calls: CallMap,
}

impl Emitter {
    fn new() -> Self {
        Emitter {
            out: String::new(),
            counter: 0,
            indent: 0,
            wrapping: false,
            calls: CallMap::new(),
        }
    }

    fn fresh(&mut self, prefix: &str) -> String {
        self.counter += 1;
        format!("%{}_{}", prefix, self.counter)
    }

    fn line(&mut self, text: &str) {
        for _ in 0..self.indent {
            self.out.push_str("  ");
        }
        self.out.push_str(text);
        self.out.push('\n');
    }

    /// Emit an i64 constant and return its SSA name.
    fn const_i64(&mut self, v: i64) -> String {
        let c = self.fresh("c");
        self.line(&format!("{} = arith.constant {} : i64", c, v));
        c
    }

    /// Emit an index constant and return its SSA name.
    fn const_index(&mut self, v: usize) -> String {
        let c = self.fresh("ci");
        self.line(&format!("{} = arith.constant {} : index", c, v));
        c
    }
}

fn mlir_param_type(ty: &Ty) -> &'static str {
    match ty {
        Ty::Int | Ty::Bool => "i64",
        Ty::F64 => "f64",
        Ty::ListInt => "memref<?xi64>",
        Ty::ListF64 => "memref<?xf64>",
    }
}

fn mlir_ret_type(ty: &Ty) -> Result<&'static str, String> {
    match ty {
        Ty::Int | Ty::Bool => Ok("i64"),
        Ty::F64 => Ok("f64"),
        Ty::ListInt => Ok("memref<?xi64>"),
        Ty::ListF64 => Ok("memref<?xf64>"),
    }
}

/// Emit a complete `module { func.func @name ... }` for a candidate.
pub fn emit_fn(
    name: &str,
    params: &[(String, Ty)],
    ret: &Ty,
    body: &Expr,
    wrapping: bool,
    calls: &CallMap,
) -> Result<String, String> {
    let out_ty = mlir_ret_type(ret)?;
    let mut em = Emitter::new();
    em.wrapping = wrapping;
    em.calls = calls.clone();

    let sig: Vec<String> = params
        .iter()
        .map(|(n, t)| format!("%{}: {}", n, mlir_param_type(t)))
        .collect();

    em.line("module {");
    em.indent += 1;
    // Trap target declaration: used by checked-tier arithmetic AND by the
    // broadcast length guard (a structural error in EVERY tier). Unused
    // declarations are valid MLIR; harnesses link abort().
    em.line("func.func private @ontic_trap() -> i64");
    em.line(&format!(
        "func.func @{}({}) -> {} {{",
        name,
        sig.join(", "),
        out_ty
    ));
    em.indent += 1;

    let mut env: Vec<Binding> = params
        .iter()
        .map(|(n, _t)| Binding {
            name: n.clone(),
            ssa: format!("%{}", n),
        })
        .collect();
    let mut tyenv0: HashMap<String, Ty> =
        params.iter().map(|(n, t)| (n.clone(), t.clone())).collect();
    // Dep call results are typed by their targets.
    for (p, t) in calls {
        tyenv0.insert(p.clone(), t.ret);
    }

    let result = emit_expr(body, &mut env, &mut tyenv0, &mut em)?;
    em.line(&format!("return {} : {}", result, out_ty));
    em.indent -= 1;
    em.line("}");
    em.indent -= 1;
    em.line("}");
    Ok(em.out)
}

fn lookup<'a>(env: &'a [Binding], name: &str) -> Result<&'a Binding, String> {
    env.iter()
        .rev()
        .find(|b| b.name == name)
        .ok_or_else(|| format!("lowering: unbound variable %{}", name))
}

/// Emit one expression; returns the SSA value carrying its result.
fn emit_expr(
    e: &Expr,
    env: &mut Vec<Binding>,
    tyenv: &mut HashMap<String, Ty>,
    em: &mut Emitter,
) -> Result<String, String> {
    match e {
        Expr::IntLit(v) => Ok(em.const_i64(*v)),
        Expr::FloatLit(v) => {
            let c = em.fresh("cf");
            em.line(&format!(
                "{} = arith.constant {} : f64",
                c,
                mlir_float(*v)
            ));
            Ok(c)
        }
        Expr::BoolLit(b) => Ok(em.const_i64(if *b { 1 } else { 0 })),
        Expr::Var(n) => Ok(lookup(env, n)?.ssa.clone()),
        Expr::ListLit(items) => emit_list_lit(items, em),
        Expr::Call(p, args) => emit_call(p, args, env, tyenv, em),
        Expr::Builtin(b, inner) => emit_builtin(*b, inner, env, tyenv, em),
        Expr::UnOp(crate::sketch::UnOp::Neg, inner) => {
            let x = emit_expr(inner, env, tyenv, em)?;
            let z = em.const_i64(0);
            let r = em.fresh("neg");
            em.line(&format!("{} = arith.subi {}, {} : i64", r, z, x));
            Ok(r)
        }
        Expr::UnOp(crate::sketch::UnOp::Not, inner) => {
            let b = emit_expr(inner, env, tyenv, em)?;
            let one = em.const_i64(1);
            let r = em.fresh("not");
            em.line(&format!("{} = arith.xori {}, {} : i64", r, b, one));
            Ok(r)
        }
        Expr::If(c, t, f) => emit_if(c, t, f, env, tyenv, em),
        Expr::Let(n, value, body) => {
            let v_ty = expr_ty(value, tyenv);
            let v = emit_expr(value, env, tyenv, em)?;
            tyenv.insert(n.clone(), v_ty);
            env.push(Binding {
                name: n.clone(),
                ssa: v,
            });
            emit_expr(body, env, tyenv, em)
        }
        Expr::Fold {
            var,
            acc,
            list,
            init,
            body,
        } => emit_fold(var, acc, list, init, body, env, tyenv, em),
        Expr::BinOp(op, l, r) => emit_binop(*op, l, r, env, tyenv, em),
    }
}

/// `[a,b,c]` allocates an anonymous memref and stores each literal.
fn emit_list_lit(items: &[i64], em: &mut Emitter) -> Result<String, String> {
    let len = em.const_index(items.len());
    let m = em.fresh("list");
    em.line(&format!(
        "{} = memref.alloc({}) : memref<?xi64>",
        m, len
    ));
    for (i, item) in items.iter().enumerate() {
        let v = em.const_i64(*item);
        let idx = em.const_index(i);
        em.line(&format!(
            "memref.store {}, {}[{}] : memref<?xi64>",
            v, m, idx
        ));
    }
    Ok(m)
}


/// Emit unary builtins. Len reads the memref dim; sum/max/min lower to
/// synthesized folds (`arith.maxsi`/`maximumf` style sentinels); numeric
/// transforms call the math dialect after implicit Int->F64 promotion.
fn emit_builtin(
    b: Builtin,
    inner: &Expr,
    env: &mut Vec<Binding>,
    tyenv: &mut HashMap<String, Ty>,
    em: &mut Emitter,
) -> Result<String, String> {
    match b {
        Builtin::Len => {
            let m = emit_expr(inner, env, tyenv, em)?;
            let idx0 = em.const_index(0);
            let dim = em.fresh("dim");
            let mty = list_memref(inner, tyenv);
            em.line(&format!(
                "{} = \"memref.dim\"({}, {}) : ({}, index) -> index",
                dim, m, idx0, mty
            ));
            let cast = em.fresh("len");
            em.line(&format!(
                "{} = arith.index_cast {} : index to i64",
                cast, dim
            ));
            Ok(cast)
        }
        Builtin::Sum | Builtin::Max | Builtin::Min => {
            let is_f = matches!(expr_ty(inner, tyenv), Ty::ListF64);
            let tag = em.fresh(match b {
                Builtin::Sum => "sum",
                Builtin::Max => "max",
                _ => "min",
            });
            let var = format!("e{}", em.counter);
            let acc = format!("a{}", em.counter);
            let init = match (b, is_f) {
                (Builtin::Sum, false) => Expr::IntLit(0),
                (Builtin::Sum, true) => Expr::FloatLit(0.0),
                (_, false) => Expr::IntLit(if matches!(b, Builtin::Max) { i64::MIN } else { i64::MAX }),
                (_, true) => Expr::FloatLit(if matches!(b, Builtin::Max) {
                    f64::NEG_INFINITY
                } else {
                    f64::INFINITY
                }),
            };
            let elem = Expr::Var(var.clone());
            let body = match b {
                Builtin::Sum => Expr::BinOp(
                    if is_f { BinOp::Add } else { BinOp::Add },
                    Box::new(Expr::Var(acc.clone())),
                    Box::new(elem),
                ),
                Builtin::Max => Expr::If(
                    Box::new(Expr::BinOp(
                        if is_f { BinOp::Gt } else { BinOp::Gt },
                        Box::new(elem.clone()),
                        Box::new(Expr::Var(acc.clone())),
                    )),
                    Box::new(elem),
                    Box::new(Expr::Var(acc.clone())),
                ),
                _ => Expr::If(
                    Box::new(Expr::BinOp(
                        BinOp::Lt,
                        Box::new(elem.clone()),
                        Box::new(Expr::Var(acc.clone())),
                    )),
                    Box::new(elem),
                    Box::new(Expr::Var(acc.clone())),
                ),
            };
            emit_fold(
                &var,
                &acc,
                inner,
                &Box::new(init),
                &Box::new(body),
                env,
                tyenv,
                em,
            )
        }
        Builtin::Sqrt | Builtin::Exp | Builtin::Log | Builtin::Abs => {
            let x = emit_expr(inner, env, tyenv, em)?;
            let xf = if expr_ty(inner, tyenv) == Ty::F64 {
                x
            } else {
                let w = em.fresh("widen");
                em.line(&format!("{} = arith.sitofp {} : i64 to f64", w, x));
                w
            };
            let out = em.fresh("math");
            let op = match b {
                Builtin::Sqrt => "math.sqrt",
                Builtin::Exp => "math.exp",
                Builtin::Log => "math.log",
                _ => "math.absf",
            };
            em.line(&format!("{} = {} {} : f64", out, op, xf));
            Ok(out)
        }
    }
}


/// Emit a vault call: evaluate args, widen numerics into F64 params, then
/// `func.call` the dependency's compiled symbol.
fn emit_call(
    path: &str,
    args: &[Expr],
    env: &mut Vec<Binding>,
    tyenv: &mut HashMap<String, Ty>,
    em: &mut Emitter,
) -> Result<String, String> {
    let target = em
        .calls
        .get(path)
        .cloned()
        .ok_or_else(|| format!("lowering: no call target for `{}`", path))?;
    if args.len() != target.params.len() {
        return Err(format!(
            "call `{}` arity {} != {}",
            path,
            args.len(),
            target.params.len()
        ));
    }
    // Widen Int arguments headed into F64 params (matches checker promotion).
    let mut prepared: Vec<(String, Ty)> = Vec::new();
    for (a, pt) in args.iter().zip(target.params.iter()) {
        let ssa = emit_expr(a, env, tyenv, em)?;
        let at = expr_ty(a, tyenv);
        let widened = if matches!(pt, Ty::F64) && matches!(at, Ty::Int) {
            let w = em.fresh("widen");
            em.line(&format!("{} = arith.sitofp {} : i64 to f64", w, ssa));
            w
        } else {
            ssa
        };
        prepared.push((widened, pt.clone()));
    }

    match target.ret {
        Ty::F64 => {
            let mut parts = Vec::new();
            for ((ssa, pt)) in &prepared {
                match pt {
                    Ty::ListF64 | Ty::ListInt => parts.push(format!(
                        "{}: {}",
                        ssa,
                        if matches!(pt, Ty::ListF64) { "memref<?xf64>" } else { "memref<?xi64>" }
                    )),
                    Ty::F64 => parts.push(format!("{}: f64", ssa)),
                    _ => parts.push(format!("{}: i64", ssa)),
                }
            }
            let out = em.fresh("call");
            // func.call type suffix lists PARAM TYPES only, never SSA names.
            let param_tys: Vec<&str> = prepared
                .iter()
                .map(|(_, pt)| match pt {
                    Ty::ListF64 => "memref<?xf64>",
                    Ty::ListInt => "memref<?xi64>",
                    Ty::F64 => "f64",
                    _ => "i64",
                })
                .collect();
            em.line(&format!(
                "{} = func.call @{}({}) : ({}) -> f64",
                out,
                target.symbol,
                prepared.iter().map(|(s, _)| s.as_str()).collect::<Vec<_>>().join(", "),
                param_tys.join(", ")
            ));
            Ok(out)
        }
        _ => Err("lowering: only F64-returning dep calls supported".to_string()),
    }
}

fn emit_if(
    c: &Expr,
    t: &Expr,
    f: &Expr,
    env: &mut Vec<Binding>,
    tyenv: &mut HashMap<String, Ty>,
    em: &mut Emitter,
) -> Result<String, String> {
    let cv = emit_expr(c, env, tyenv, em)?;
    let cond = em.fresh("cond");
    em.line(&format!(
        "{} = arith.trunci {} : i64 to i1",
        cond, cv
    ));
    let ty_str = if expr_ty(t, tyenv) == Ty::F64 { "f64" } else { "i64" };
    let result = em.fresh("ifres");
    em.line(&format!("{} = scf.if {} -> ({}) {{", result, cond, ty_str));
    em.indent += 1;
    let tv = emit_expr(t, env, tyenv, em)?;
    em.line(&format!("scf.yield {} : {}", tv, ty_str));
    em.indent -= 1;
    em.line("} else {");
    em.indent += 1;
    let fv = emit_expr(f, env, tyenv, em)?;
    em.line(&format!("scf.yield {} : {}", fv, ty_str));
    em.indent -= 1;
    em.line("}");
    Ok(result)
}

/// Fold lowers to `scf.for` with `iter_args(%acc_ssa = init)` over the list
/// memref; `%var_ssa` is the loaded element each iteration.
#[allow(clippy::too_many_arguments)]
fn emit_fold(
    var: &str,
    acc: &str,
    list: &Expr,
    init: &Expr,
    body: &Expr,
    env: &mut Vec<Binding>,
    tyenv: &mut HashMap<String, Ty>,
    em: &mut Emitter,
) -> Result<String, String> {
    let init_v = emit_expr(init, env, tyenv, em)?;
    let m = emit_expr(list, env, tyenv, em)?;
    let idx0 = em.const_index(0);
    let step = em.const_index(1);
    let dim = em.fresh("dim");
    let mty = list_memref(list, tyenv);
    // Generic op syntax — see Len arm note re Ubuntu mlir-opt memref.dim.
    em.line(&format!(
        "{} = \"memref.dim\"({}, {}) : ({}, index) -> index",
        dim, m, idx0, mty
    ));

    let iv = em.fresh("i");
    let acc_ssa = em.fresh("acc");
    let ty_str = if expr_ty(init, tyenv) == Ty::F64 { "f64" } else { "i64" };
    em.line(&format!(
        "{} = scf.for {} = {} to {} step {} iter_args({} = {}) -> ({}) {{",
        acc_ssa, iv, idx0, dim, step, acc_ssa, init_v, ty_str
    ));
    em.indent += 1;

    // Load type follows the folded list's element kind.
    let elem = em.fresh("x");
    if expr_ty(list, tyenv) == Ty::ListF64 {
        em.line(&format!(
            "{} = memref.load {}[{}] : memref<?xf64>",
            elem, m, iv
        ));
    } else {
        em.line(&format!("{} = memref.load {}[{}] : {}", elem, m, iv, mty));
    }

    tyenv.insert(var.to_string(), if matches!(expr_ty(list, tyenv), Ty::ListF64) { Ty::F64 } else { Ty::Int });
    tyenv.insert(acc.to_string(), if ty_str == "f64" { Ty::F64 } else { Ty::Int });
    env.push(Binding {
        name: var.to_string(),
        ssa: elem.clone(),
    });
    env.push(Binding {
        name: acc.to_string(),
        ssa: acc_ssa.clone(),
    });
    let body_v = emit_expr(body, env, tyenv, em)?;
    em.line(&format!("scf.yield {} : {}", body_v, ty_str));
    em.indent -= 1;
    em.line("}");
    Ok(acc_ssa)
}

fn emit_binop(
    op: BinOp,
    l: &Expr,
    r: &Expr,
    env: &mut Vec<Binding>,
    tyenv: &mut HashMap<String, Ty>,
    em: &mut Emitter,
) -> Result<String, String> {
    let lv = emit_expr(l, env, tyenv, em)?;
    let rv = emit_expr(r, env, tyenv, em)?;
    // Numeric promotion: widen the Int side of a mixed numeric op via
    // arith.sitofp (matches interp/check convention).
    let lt = expr_ty(l, tyenv);
    let rt = expr_ty(r, tyenv);
    let mixed_float = matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge)
        && ((matches!(lt, Ty::F64) && matches!(rt, Ty::Int))
            || (matches!(lt, Ty::Int) && matches!(rt, Ty::F64)));
    let mut lv_s = lv;
    let mut rv_s = rv;
    let mut any_float = matches!(lt, Ty::F64) || matches!(rt, Ty::F64);
    if mixed_float {
        if matches!(lt, Ty::Int) {
            let w = em.fresh("widen");
            em.line(&format!("{} = arith.sitofp {} : i64 to f64", w, lv_s));
            lv_s = w;
        }
        if matches!(rt, Ty::Int) {
            let w = em.fresh("widen");
            em.line(&format!("{} = arith.sitofp {} : i64 to f64", w, rv_s));
            rv_s = w;
        }
        any_float = true;
    }
    if is_comparison(op) {
        return emit_cmp(op, &lv_s, &rv_s, any_float, em);
    }
    // Broadcasting: either operand a list -> elementwise loop over a fresh
    // result memref. Scalar operands are loaded per-iteration.
    let l_listy = matches!(lt, Ty::ListInt | Ty::ListF64);
    let r_listy = matches!(rt, Ty::ListInt | Ty::ListF64);
    if (l_listy || r_listy)
        && matches!(
            op,
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod
        )
    {
        return emit_broadcast(op, l, r, &lv_s, &rv_s, &lt, &rt, tyenv, em);
    }
    if any_float && matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod) {
        let out = em.fresh("opf");
        let stmt = match op {
            BinOp::Add => format!("{} = arith.addf {}, {} : f64", out, lv_s, rv_s),
            BinOp::Sub => format!("{} = arith.subf {}, {} : f64", out, lv_s, rv_s),
            BinOp::Mul => format!("{} = arith.mulf {}, {} : f64", out, lv_s, rv_s),
            BinOp::Div => format!("{} = arith.divf {}, {} : f64", out, lv_s, rv_s),
            _ => format!("{} = arith.remf {}, {} : f64", out, lv_s, rv_s),
        };
        em.line(&stmt);
        return Ok(out);
    }
    if matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul) && !em.wrapping {
        return emit_checked_arith(op, &lv_s, &rv_s, em);
    }
    let out = em.fresh("op");
    let stmt = match op {
        BinOp::Add => Some(format!("{} = arith.addi {}, {} : i64", out, lv_s, rv_s)),
        BinOp::Sub => Some(format!("{} = arith.subi {}, {} : i64", out, lv_s, rv_s)),
        BinOp::Mul => Some(format!("{} = arith.muli {}, {} : i64", out, lv_s, rv_s)),
        BinOp::Div => Some(format!("{} = arith.divsi {}, {} : i64", out, lv_s, rv_s)),
        BinOp::Mod => Some(format!("{} = arith.remsi {}, {} : i64", out, lv_s, rv_s)),
        BinOp::And => Some(format!("{} = arith.andi {}, {} : i64", out, lv_s, rv_s)),
        BinOp::Or => Some(format!("{} = arith.ori {}, {} : i64", out, lv_s, rv_s)),
        _ => None,
    };
    match stmt {
        Some(s) => {
            em.line(&s);
            Ok(out)
        }
        None => Err(format!("lowering: unhandled binop {:?}", op)),
    }
}

/// Checked-tier arithmetic: compute in i128 (cannot overflow for any i64
/// operand pair), range-check against i64, narrow — or trap via extern
/// `ontic_trap`. Matches interpreter kill semantics exactly.
fn emit_checked_arith(
    op: BinOp,
    lv: &str,
    rv: &str,
    em: &mut Emitter,
) -> Result<String, String> {
    let wide_op = match op {
        BinOp::Add => "arith.addi",
        BinOp::Sub => "arith.subi",
        BinOp::Mul => "arith.muli",
        _ => return Err(format!("checked expansion unsupported for {:?}", op)),
    };
    let a128 = em.fresh("wa");
    em.line(&format!("{} = arith.extsi {} : i64 to i128", a128, lv));
    let b128 = em.fresh("wb");
    em.line(&format!("{} = arith.extsi {} : i64 to i128", b128, rv));
    let s128 = em.fresh("ws");
    em.line(&format!("{} = {} {}, {} : i128", s128, wide_op, a128, b128));
    let cmin = em.fresh("cmin");
    em.line(&format!(
        "{} = arith.constant -9223372036854775808 : i128",
        cmin
    ));
    let cmax = em.fresh("cmax");
    em.line(&format!(
        "{} = arith.constant 9223372036854775807 : i128",
        cmax
    ));
    let ge = em.fresh("ge");
    em.line(&format!("{} = arith.cmpi sge, {}, {} : i128", ge, s128, cmin));
    let le = em.fresh("le");
    em.line(&format!("{} = arith.cmpi sle, {}, {} : i128", le, s128, cmax));
    let ok = em.fresh("ok");
    em.line(&format!("{} = arith.andi {}, {} : i1", ok, ge, le));

    let res = em.fresh("chk");
    em.line(&format!("{} = scf.if {} -> (i64) {{", res, ok));
    em.indent += 1;
    let narrow = em.fresh("narrow");
    em.line(&format!(
        "{} = arith.trunci {} : i128 to i64",
        narrow, s128
    ));
    em.line(&format!("scf.yield {} : i64", narrow));
    em.indent -= 1;
    em.line("} else {");
    em.indent += 1;
    let trapped = em.fresh("trap");
    em.line(&format!(
        "{} = func.call @ontic_trap() : () -> i64",
        trapped
    ));
    em.line(&format!("scf.yield {} : i64", trapped));
    em.indent -= 1;
    em.line("}");
    Ok(res)
}


/// Format an f64 as an MLIR float literal: scientific notation whose
/// mantissa ALWAYS carries a decimal point (`0e0` parses as op name `e0`).
fn mlir_float(v: f64) -> String {
    let s = format!("{:e}", v);
    match s.split_once('e') {
        Some((mantissa, exp)) => {
            let m = if mantissa.contains('.') {
                mantissa.to_string()
            } else {
                format!("{}.0", mantissa)
            };
            format!("{}e{}", m, exp)
        }
        None => s,
    }
}

/// Static type of an expression for emission decisions (mirrors check::infer
/// for the subset the emitter needs; candidates were typechecked already).
fn expr_ty(e: &Expr, tyenv: &HashMap<String, Ty>) -> Ty {
    match e {
        Expr::IntLit(_) => Ty::Int,
        Expr::FloatLit(_) => Ty::F64,
        Expr::BoolLit(_) => Ty::Bool,
        Expr::ListLit(_) => Ty::ListInt,
        Expr::Var(n) => tyenv.get(n).cloned().unwrap_or(Ty::Int),
        Expr::Call(p, _) => tyenv.get(p).cloned().unwrap_or(Ty::Int),
        Expr::Builtin(b, inner) => match b {
            Builtin::Len => Ty::Int,
            // Reductions follow their list's element type.
            Builtin::Sum | Builtin::Max | Builtin::Min => {
                if matches!(expr_ty(inner, tyenv), Ty::ListF64) {
                    Ty::F64
                } else {
                    Ty::Int
                }
            }
            // Numeric transforms are always F64.
            _ => Ty::F64,
        },
        Expr::UnOp(crate::sketch::UnOp::Not, _) => Ty::Bool,
        Expr::UnOp(crate::sketch::UnOp::Neg, i) => expr_ty(i, tyenv),
        Expr::If(_, t, _) => expr_ty(t, tyenv),
        Expr::Let(_, _, b) => expr_ty(b, tyenv),
        Expr::Fold { init, .. } => expr_ty(init, tyenv),
        Expr::BinOp(op, l, r) => match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
                let lt = expr_ty(l, tyenv);
                let rt = expr_ty(r, tyenv);
                broadcast_result_ty(lt, rt)
            }
            _ => Ty::Bool,
        },
    }
}

/// Result type of arithmetic over two operand types (broadcast rules,
/// mirroring check.rs).
fn broadcast_result_ty(lt: Ty, rt: Ty) -> Ty {
    use Ty::*;
    let num = |t: &Ty| matches!(t, Int | F64);
    let any_f = matches!(lt, F64 | ListF64) || matches!(rt, F64 | ListF64);
    match (&lt, &rt) {
        (ListInt, ListInt) => ListInt,
        (ListInt, t) | (t, ListInt) if num(t) && any_f => ListF64,
        (ListInt, t) | (t, ListInt) if num(t) => ListInt,
        (ListF64, _) | (_, ListF64) if num(&lt) || num(&rt) || matches!(lt, ListF64) || matches!(rt, ListF64) => {
            ListF64
        }
        _ => if any_f { F64 } else { Int },
    }
}

/// MemRef type string for a list-valued expression.
fn list_memref(e: &Expr, tyenv: &HashMap<String, Ty>) -> &'static str {
    if matches!(expr_ty(e, tyenv), Ty::ListF64) {
        "memref<?xf64>"
    } else {
        "memref<?xi64>"
    }
}

fn is_comparison(op: BinOp) -> bool {
    matches!(
        op,
        BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge
    )
}

/// Comparisons produce i1 then widen to the boolean ABI. Floats use cmpf.
fn emit_cmp(
    op: BinOp,
    lv: &str,
    rv: &str,
    is_float: bool,
    em: &mut Emitter,
) -> Result<String, String> {
    let pred = match op {
        BinOp::Eq => "eq",
        BinOp::Ne => "ne",
        BinOp::Lt => "slt",
        BinOp::Le => "sle",
        BinOp::Gt => "sgt",
        _ => "sge",
    };
    let bit = em.fresh("cmp");
    if is_float {
        em.line(&format!("{} = arith.cmpf {}, {}, {} : f64", bit, pred, lv, rv));
    } else {
        em.line(&format!("{} = arith.cmpi {}, {}, {} : i64", bit, pred, lv, rv));
    }
    let out = em.fresh("bool");
    em.line(&format!("{} = arith.extui {} : i1 to i64", out, bit));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{check, sketch};

    fn lower(src: &str) -> String {
        let c = sketch::parse(src).unwrap();
        check::check(&c).unwrap();
        emit_fn(&c.name, &c.params, &c.ret, &c.body, true, &CallMap::new()).expect("lowers")
    }

    #[test]
    fn test_fold_lowers_to_scf_for_with_iter_args() {
        let ir = lower(
            "fn @total(%items: List<Int>) -> Int { fold %x in %items, %acc from 0 { %acc + %x } }",
        );
        assert!(ir.contains("scf.for"));
        assert!(ir.contains("iter_args"));
        assert!(ir.contains("arith.addi"));
        assert!(ir.contains("memref.load"));
        assert!(ir.contains("func.func @total(%items: memref<?xi64>) -> i64"));
    }

    #[test]
    fn test_if_lowers_to_scf_if() {
        let ir = lower("fn @g(%n: Int) -> Int { if %n > 0 { 1 } else { 0 - %n } }");
        assert!(ir.contains("scf.if"));
        assert!(ir.contains("arith.cmpi sgt"));
        assert!(ir.contains("arith.extui"));
        assert!(ir.contains("arith.trunci"));
    }

    #[test]
    fn test_len_and_list_literal_lowering() {
        let ir = lower("fn @h() -> Int { len([7, 8, 9]) }");
        assert!(ir.contains("memref.alloc"));
        assert!(ir.contains("arith.constant 7 : i64"));
        assert!(ir.contains("memref.dim"));
        assert!(ir.contains("arith.index_cast"));
    }

    #[test]
    fn test_division_uses_signed_ops() {
        let ir = lower("fn @d(%a: Int, %b: Int) -> Int { (%a / %b) + (%a % %b) }");
        assert!(ir.contains("divsi"));
        assert!(ir.contains("remsi"));
    }

    #[test]
    fn test_display_round_trip_shape() {
        let e = crate::sketch::parse_expr_str("%acc + %x").unwrap();
        assert_eq!(expr_display(&e), "(%acc + %x)");
    }

    #[test]
    fn test_bool_params_are_i64_abi() {
        let ir = lower("fn @b(%p: Bool) -> Int { if %p { 1 } else { 2 } }");
        assert!(ir.contains("func.func @b(%p: i64) -> i64"));
    }
}

#[cfg(test)]
mod tier_tests {
    use super::*;
    use crate::sketch;

    #[test]
    fn test_checked_tier_expands_wide_check_and_trap() {
        let c = sketch::parse("fn @f(%a: Int, %b: Int) -> Int { %a + %b }").unwrap();
        let ir = emit_fn(&c.name, &c.params, &c.ret, &c.body, false, &CallMap::new()).unwrap();
        assert!(ir.contains("ontic_trap"), "missing trap decl");
        assert!(ir.contains("i128"));
        assert!(ir.contains("scf.if"));
        // Wrapping tier of the same body stays plain.
        let plain = emit_fn(&c.name, &c.params, &c.ret, &c.body, true, &CallMap::new()).unwrap();
        assert!(!plain.contains("i128"));
        assert!(plain.contains("arith.addi"));
    }
}

/// Broadcasting lowering: guard equal lengths (trap on mismatch — the oracle
/// errors there), allocate a result memref, elementwise loop with stores.
/// Scalar operands are re-loaded per iteration; Int elements widen via
/// arith.sitofp when the result is F64.
#[allow(clippy::too_many_arguments)]
fn emit_broadcast(
    op: BinOp,
    l: &Expr,
    r: &Expr,
    lv_ssa: &str,
    rv_ssa: &str,
    lt: &Ty,
    rt: &Ty,
    tyenv: &mut HashMap<String, Ty>,
    em: &mut Emitter,
) -> Result<String, String> {
    let out_ty = broadcast_result_ty(lt.clone(), rt.clone());
    let elem_ty = if matches!(out_ty, Ty::ListF64) { "f64" } else { "i64" };
    let mty_out = if matches!(out_ty, Ty::ListF64) {
        "memref<?xf64>"
    } else {
        "memref<?xi64>"
    };

    // Size source: whichever operand is a list.
    let (size_ssa, mty_in) = if matches!(lt, Ty::ListInt | Ty::ListF64) {
        (
            lv_ssa.to_string(),
            if matches!(lt, Ty::ListF64) {
                "memref<?xf64>"
            } else {
                "memref<?xi64>"
            },
        )
    } else {
        (
            rv_ssa.to_string(),
            if matches!(rt, Ty::ListF64) {
                "memref<?xf64>"
            } else {
                "memref<?xi64>"
            },
        )
    };
    let idx0 = em.const_index(0);
    let step = em.const_index(1);
    let dim = em.fresh("bdim");
    em.line(&format!(
        "{} = \"memref.dim\"({}, {}) : ({}, index) -> index",
        dim, size_ssa, idx0, mty_in
    ));

    // Length guard when both operands are lists: mismatch traps, matching
    // the oracle's zip-mismatch error.
    if matches!(lt, Ty::ListInt | Ty::ListF64) && matches!(rt, Ty::ListInt | Ty::ListF64) {
        let mty_r = if matches!(rt, Ty::ListF64) {
            "memref<?xf64>"
        } else {
            "memref<?xi64>"
        };
        let dim_r = em.fresh("brdim");
        em.line(&format!(
            "{} = \"memref.dim\"({}, {}) : ({}, index) -> index",
            dim_r, rv_ssa, idx0, mty_r
        ));
        let eq = em.fresh("beq");
        em.line(&format!(
            "{} = arith.cmpi eq, {}, {} : index",
            eq, dim, dim_r
        ));
        let guard = em.fresh("bguard");
        em.line(&format!("{} = scf.if {} -> (i64) {{", guard, eq));
        em.indent += 1;
        let zero = em.const_i64(0);
        em.line(&format!("scf.yield {} : i64", zero));
        em.indent -= 1;
        em.line("} else {");
        em.indent += 1;
        let trapped = em.fresh("trap");
        em.line(&format!(
            "{} = func.call @ontic_trap() : () -> i64",
            trapped
        ));
        em.line(&format!("scf.yield {} : i64", trapped));
        em.indent -= 1;
        em.line("}");
    }

    let alloc = em.fresh("balloc");
    em.line(&format!(
        "{} = memref.alloc({}) : {}",
        alloc, dim, mty_out
    ));

    let iv = em.fresh("bi");
    // No iter_args: results flow through the result memref stores.
    em.line(&format!("scf.for {} = {} to {} step {} {{", iv, idx0, dim, step));
    em.indent += 1;

    let l_elem = if matches!(lt, Ty::ListInt | Ty::ListF64) {
        let x = em.fresh("bx");
        em.line(&format!(
            "{} = memref.load {}[{}] : {}",
            x, lv_ssa, iv, list_memref(l, tyenv)
        ));
        x
    } else {
        lv_ssa.to_string()
    };
    let r_elem = if matches!(rt, Ty::ListInt | Ty::ListF64) {
        let y = em.fresh("by");
        em.line(&format!(
            "{} = memref.load {}[{}] : {}",
            y, rv_ssa, iv, list_memref(r, tyenv)
        ));
        y
    } else {
        rv_ssa.to_string()
    };

    // Elementwise op (widen ints into f64 results).
    let (a, b) = if matches!(out_ty, Ty::ListF64) {
        let a = elem_to_f64(l_elem, &lt_scalar_kind(lt), em);
        let b = elem_to_f64(r_elem, &lt_scalar_kind(rt), em);
        (a, b)
    } else {
        (l_elem, r_elem)
    };

    let val = em.fresh("bv");
    let stmt = match (matches!(out_ty, Ty::ListF64), op) {
        (_, BinOp::Add) if matches!(out_ty, Ty::ListF64) => {
            format!("{} = arith.addf {}, {} : f64", val, a, b)
        }
        (_, BinOp::Sub) if matches!(out_ty, Ty::ListF64) => {
            format!("{} = arith.subf {}, {} : f64", val, a, b)
        }
        (_, BinOp::Mul) if matches!(out_ty, Ty::ListF64) => {
            format!("{} = arith.mulf {}, {} : f64", val, a, b)
        }
        (_, BinOp::Div) if matches!(out_ty, Ty::ListF64) => {
            format!("{} = arith.divf {}, {} : f64", val, a, b)
        }
        (true, _) => format!("{} = arith.remf {}, {} : f64", val, a, b),
        (_, BinOp::Add) => format!("{} = arith.addi {}, {} : i64", val, a, b),
        (_, BinOp::Sub) => format!("{} = arith.subi {}, {} : i64", val, a, b),
        (_, BinOp::Mul) => format!("{} = arith.muli {}, {} : i64", val, a, b),
        (_, BinOp::Div) => format!("{} = arith.divsi {}, {} : i64", val, a, b),
        _ => format!("{} = arith.remsi {}, {} : i64", val, a, b),
    };
    em.line(&stmt);
    em.line(&format!(
        "memref.store {}, {}[{}] : {}",
        val, alloc, iv, mty_out
    ));
    // Bare scf.for has no yield; the alloc carries the result.
    em.indent -= 1;
    em.line("}");
    Ok(alloc)
}

/// Element kind fed to elem_to_f64: lists contribute their ELEMENT type;
/// scalars are already their own type.
fn lt_scalar_kind(t: &Ty) -> Ty {
    match t {
        Ty::ListF64 => Ty::F64,
        Ty::ListInt => Ty::Int,
        other => other.clone(),
    }
}

/// Widen an int-typed element value to f64 in place.
fn elem_to_f64(x: String, t: &Ty, em: &mut Emitter) -> String {
    match t {
        Ty::Int => {
            let w = em.fresh("widen");
            em.line(&format!("{} = arith.sitofp {} : i64 to f64", w, x));
            w
        }
        _ => x,
    }
}

/// Merge several emitted modules into one: inner `func.func`s concatenated
/// inside a single `module { ... }`. Used so candidates calling vault
/// functions validate AND link together with their dependencies.
pub fn compose_modules(mlirs: &[String]) -> Result<String, String> {
    if mlirs.is_empty() {
        return Err("no modules to compose".to_string());
    }
    let mut out = String::from("module {\n");
    for m in mlirs {
        let t = m.trim();
        let inner = t
            .strip_prefix("module {")
            .and_then(|x| x.strip_suffix('}'))
            .ok_or_else(|| "compose: module not in expected shape".to_string())?;
        // Dedent one level (our emitter uses two-space indent uniformly).
        for line in inner.lines() {
            let l = line.strip_prefix("  ").unwrap_or(line);
            if !l.trim().is_empty() {
                out.push_str("  ");
                out.push_str(l);
                out.push('\n');
            }
        }
    }
    out.push('}');
    Ok(out)
}

#[cfg(test)]
mod compose_tests {
    use super::*;

    #[test]
    fn test_compose_merges_func_decls() {
        let a = "module {\n  func.func @a(%\"x\": i64) -> i64 {\n    return %\"x\" : i64\n  }\n}".to_string();
        let b = "module {\n  func.func @b() -> i64 {\n    return 0 : i64\n  }\n}".to_string();
        let c = compose_modules(&[a, b]).unwrap();
        assert!(c.contains("func.func @a"));
        assert!(c.contains("func.func @b"));
        assert_eq!(c.matches("module {").count(), 1);
    }
}
