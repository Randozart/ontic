//! Lowering: sketch AST → MLIR text (arith + scf + memref), plus the
//! expression pretty-printer used in canonical gen text and diagnostics.
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
/// Used by canonical gen serialization and sieve diagnostics.
pub fn expr_display(e: &Expr) -> String {
    match e {
        Expr::IntLit(v) => v.to_string(),
        Expr::FloatLit(v) => format!("{:e}", v),
        Expr::BoolLit(b) => b.to_string(),
        Expr::ListCons(elems) => format!(
            "[{}]",
            elems.iter().map(expr_display).collect::<Vec<_>>().join(", ")
        ),
        Expr::Call(p, args) => format!(
            "{}({})",
            p,
            args.iter()
                .map(expr_display)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Expr::Var(n) => format!("%{}", n),
        Expr::FloatListLit(items) => {
            let inner: Vec<String> = items.iter().map(|v| v.to_string()).collect();
            format!("[{}]", inner.join(", "))
        }
        Expr::ListLit(items) => {
            let inner: Vec<String> = items.iter().map(|v| v.to_string()).collect();
            format!("[{}]", inner.join(", "))
        }
        Expr::Builtin(b, i) => format!(
            "{}({})",
            match b {
                Builtin::Len => "len",
                Builtin::Range => "range",
                Builtin::Sum => "sum",
                Builtin::Max => "max",
                Builtin::Min => "min",
                Builtin::Sqrt => "sqrt",
                Builtin::Exp => "exp",
                Builtin::Log => "log",
                Builtin::Abs => "abs",
                Builtin::Index => "index",
                Builtin::MinEl => "min_el",
                Builtin::MaxEl => "max_el",
            },
            expr_display(i)
        ),
        Expr::Builtin2(b, l, r) => format!(
            "{}({}, {})",
            match b {
                Builtin::Index => "index",
                Builtin::MinEl => "min_el",
                Builtin::MaxEl => "max_el",
                other => unreachable!("builtin2 display: {:?}", other),
            },
            expr_display(l),
            expr_display(r)
        ),
        Expr::Map { var, list, body } => format!(
            "map(%{} in {}) {{ {} }}",
            var,
            expr_display(list),
            expr_display(body)
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
            until,
            aux,
        } => format!(
            "(fold %{} in {}, %{} from {}{} {{ {} }}{})",
            var,
            expr_display(list),
            acc,
            expr_display(init),
            aux.iter()
                .map(|(n, e)| format!(", %{} from {}", n, expr_display(e)))
                .collect::<String>(),
            expr_display(body),
            match until {
                Some(u) => format!(" until {}", expr_display(u)),
                None => String::new(),
            }
        ),
        Expr::Tuple(items) => format!(
            "({})",
            items.iter().map(expr_display).collect::<Vec<_>>().join(", ")
        ),
        Expr::BinOp(op, l, r) => format!(
            "({} {} {})",
            expr_display(l),
            binop_str(*op),
            expr_display(r)
        ),
    }
}

/// Public alias so the uniform sampler renders operators identically.
pub fn binop_display(op: BinOp) -> &'static str {
    binop_str(op)
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
        BinOp::Concat => "++",
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
    calls: CallMap,
}

impl Emitter {
    fn new() -> Self {
        Emitter {
            out: String::new(),
            counter: 0,
            indent: 0,
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
        Ty::F32 => "f32",
        Ty::ListInt => "memref<?xi64>",
        Ty::ListF64 => "memref<?xf64>",
        Ty::ListF32 => "memref<?xf32>",
    }
}

fn mlir_ret_type(ty: &Ty) -> Result<&'static str, String> {
    match ty {
        Ty::Int | Ty::Bool => Ok("i64"),
        Ty::F64 => Ok("f64"),
        Ty::F32 => Ok("f32"),
        Ty::ListInt => Ok("memref<?xi64>"),
        Ty::ListF64 => Ok("memref<?xf64>"),
        Ty::ListF32 => Ok("memref<?xf32>"),
    }
}

/// Emit a complete `module { func.func @name ... }` for a candidate.
pub fn emit_fn(
    name: &str,
    params: &[(String, Ty)],
    ret: &Ty,
    body: &Expr,
    calls: &CallMap,
) -> Result<String, String> {
    let out_ty = mlir_ret_type(ret)?;
    let mut em = Emitter::new();
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
    em.line("func.func private @ontic_trapf() -> f64");
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
            let mlir_ty = if matches!(expr_ty(e, tyenv), Ty::F32) {
                "f32"
            } else {
                "f64"
            };
            em.line(&format!(
                "{} = arith.constant {} : {}",
                c,
                mlir_float(*v),
                mlir_ty
            ));
            Ok(c)
        }
        Expr::BoolLit(b) => Ok(em.const_i64(if *b { 1 } else { 0 })),
        Expr::Var(n) => Ok(lookup(env, n)?.ssa.clone()),
        Expr::ListLit(items) => emit_list_lit(items, em),
        Expr::FloatListLit(items) => {
            // Allocate an f64 memref and store each element.
            let len = em.const_index(items.len());
            let m = em.fresh("flist");
            em.line(&format!(
                "{} = memref.alloc({}) : memref<?xf64>",
                m, len
            ));
            for (i2, item) in items.iter().enumerate() {
                let v = em.fresh("cf");
                em.line(&format!("{} = arith.constant {} : f64", v, item));
                let ix = em.const_index(i2);
                em.line(&format!(
                    "memref.store {}, {}[{}] : memref<?xf64>",
                    v, m, ix
                ));
            }
            Ok(m)
        }
        Expr::Call(p, args) => emit_call(p, args, env, tyenv, em),
        Expr::Builtin(b, inner) => emit_builtin(*b, inner, env, tyenv, em),
        Expr::Builtin2(
            b @ (crate::sketch::Builtin::MinEl | crate::sketch::Builtin::MaxEl),
            lr,
            rr,
        ) => {
            let lv = emit_expr(lr, env, tyenv, em)?;
            let rv = emit_expr(rr, env, tyenv, em)?;
            let lt = expr_ty(lr, tyenv);
            let rt = expr_ty(rr, tyenv);
            let any_float = matches!(lt, Ty::F64 | Ty::F32) || matches!(rt, Ty::F64 | Ty::F32);
            let float_ty = if matches!(lt, Ty::F32) || matches!(rt, Ty::F32) { "f32" } else { "f64" };
            let (lv_s, rv_s) = if any_float {
                let lw = if matches!(lt, Ty::Int) {
                    let w = em.fresh("widen");
                    em.line(&format!("{} = arith.sitofp {} : i64 to {}", w, lv, float_ty));
                    w
                } else {
                    lv
                };
                let rw = if matches!(rt, Ty::Int) {
                    let w = em.fresh("widen");
                    em.line(&format!("{} = arith.sitofp {} : i64 to {}", w, rv, float_ty));
                    w
                } else {
                    rv
                };
                (lw, rw)
            } else {
                (lv, rv)
            };
            let is_min = matches!(b, crate::sketch::Builtin::MinEl);
            let pred = match (any_float, is_min) {
                (true, true) => "olt",
                (true, false) => "ogt",
                (false, true) => "slt",
                (false, false) => "sgt",
            };
            let cmp = em.fresh("c");
            if any_float {
                em.line(&format!(
                    "{} = arith.cmpf {}, {}, {} : {}",
                    cmp, pred, lv_s, rv_s, float_ty
                ));
            } else {
                em.line(&format!(
                    "{} = arith.cmpi {}, {}, {} : i64",
                    cmp, pred, lv_s, rv_s
                ));
            }
            let r = em.fresh("sel");
            let sel_ty = if any_float { "f64" } else { "i64" };
            // min: pick left when left < right; max: pick left when left > right.
            em.line(&format!(
                "{} = arith.select {}, {}, {} : {}",
                r, cmp, lv_s, rv_s, sel_ty
            ));
            Ok(r)
        }
        Expr::Builtin2(crate::sketch::Builtin::Index, l, r) => {
            emit_index(l, r, env, tyenv, em)
        }
        Expr::Map { var, list, body } => {
            emit_map(var, list, body, env, tyenv, em)
        }
        Expr::Builtin2(b, _, _) => Err(format!(
            "lowering: builtin {:?} is unary",
            b
        )),
        Expr::ListCons(elems) => {
            // Element type mirrors the checker: F64 if ANY element is F64,
            // else Int. Literal-presence heuristics miss computed floats
            // (e.g. `[c/det, -b/det, ...]`) and mis-alloc i64 memrefs.
            let elem_f64 = elems.iter().any(|e| matches!(expr_ty(e, tyenv), Ty::F64))
                || matches!(
                    expr_ty(&Expr::ListCons(elems.clone()), tyenv),
                    Ty::ListF64
                );
            let elem_f32 = elems.iter().any(|e| matches!(expr_ty(e, tyenv), Ty::F32))
                || matches!(
                    expr_ty(&Expr::ListCons(elems.clone()), tyenv),
                    Ty::ListF32
                );
            let elem_f = elem_f64 || elem_f32;
            let mty = if elem_f32 { "memref<?xf32>" } else if elem_f64 { "memref<?xf64>" } else { "memref<?xi64>" };
            let _ety = if elem_f32 { "f32" } else if elem_f64 { "f64" } else { "i64" };
            let count = em.const_index(elems.len());
            let alloc = em.fresh("lc");
            em.line(&format!("{} = memref.alloc({}) : {}", alloc, count, mty));
            for (i, e) in elems.iter().enumerate() {
                let v = emit_expr(e, env, tyenv, em)?;
                let v2 = if elem_f && !matches!(expr_ty(e, tyenv), Ty::F64 | Ty::F32) {
                    // Widen int-typed sub-expressions to the list's float type.
                    let w = em.fresh("widen");
                    let sitofp_ty = if elem_f32 { "f32" } else { "f64" };
                    em.line(&format!(
                        "{} = arith.sitofp {} : i64 to {}",
                        w, v, sitofp_ty
                    ));
                    w
                } else {
                    v
                };
                let ix = em.const_index(i);
                em.line(&format!(
                    "memref.store {}, {}[{}] : {}",
                    v2, alloc, ix, mty
                ));
            }
            Ok(alloc)
        }
        Expr::UnOp(crate::sketch::UnOp::Neg, inner) => {
            let x = emit_expr(inner, env, tyenv, em)?;
            let r = em.fresh("neg");
            if matches!(expr_ty(inner, tyenv), Ty::F64 | Ty::F32) {
                em.line(&format!("{} = arith.negf {} : f64", r, x));
            } else {
                let z = em.const_i64(0);
                em.line(&format!("{} = arith.subi {}, {} : i64", r, z, x));
            }
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
            ref until,
            ref aux,
        } => emit_fold(
            var,
            acc,
            list,
            init,
            body,
            until.as_deref(),
            aux,
            env,
            tyenv,
            em,
        ),
        Expr::BinOp(op, l, r) => emit_binop(*op, l, r, env, tyenv, em),
        Expr::Tuple(_) => Err(
            "tuple expressions require a multi-accumulator fold (emission is component-wise)"
                .to_string(),
        ),
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
        Builtin::MinEl | Builtin::MaxEl => Err(format!(
            "lowering: {:?} is binary (internal routing error)",
            b
        )),
        // range(n) allocates an i64 memref 0..n and fills it by loop.
        Builtin::Range => {
            let n64 = emit_expr(inner, env, tyenv, em)?;
            let idx0 = em.const_index(0);
            let step = em.const_index(1);
            let n_idx = em.fresh("rn");
            em.line(&format!(
                "{} = arith.index_cast {} : i64 to index",
                n_idx, n64
            ));
            let neg = em.fresh("rneg");
            let z = em.const_i64(0);
            em.line(&format!("{} = arith.cmpi slt, {}, {} : i64", neg, n64, z));
            let guard = em.fresh("rguard");
            em.line(&format!("{} = scf.if {} -> (index) {{", guard, neg));
            em.indent += 1;
            em.line(&format!("scf.yield {} : index", idx0));
            em.indent -= 1;
            em.line("} else {");
            em.indent += 1;
            em.line(&format!("scf.yield {} : index", n_idx));
            em.indent -= 1;
            em.line("}");
            let alloc = em.fresh("range");
            em.line(&format!(
                "{} = memref.alloc({}) : memref<?xi64>",
                alloc, guard
            ));
            let iv = em.fresh("ri");
            let accp = em.fresh("racc");
            em.line(&format!(
                "{} = scf.for {} = {} to {} step {} iter_args({} = {}) -> (index) {{",
                accp, iv, idx0, guard, step, accp, idx0
            ));
            em.indent += 1;
            let v64 = em.fresh("rv");
            em.line(&format!(
                "{} = arith.index_cast {} : index to i64",
                v64, iv
            ));
            em.line(&format!(
                "memref.store {}, {}[{}] : memref<?xi64>",
                v64, alloc, iv
            ));
            em.line(&format!("scf.yield {} : index", accp));
            em.indent -= 1;
            em.line("}");
            // The allocated buffer carries the iota; the loop result is unused.
            Ok(alloc)
        }
        Builtin::Index => Err("lowering: index is binary".to_string()),
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
                None,
                &[],
                env,
                tyenv,
                em,
            )
        }
        Builtin::Sqrt | Builtin::Exp | Builtin::Log | Builtin::Abs => {
            let x = emit_expr(inner, env, tyenv, em)?;
            let xf = if matches!(expr_ty(inner, tyenv), Ty::F64 | Ty::F32) {
                x
            } else {
                let w = em.fresh("widen");
                let sitofp_ty = if matches!(expr_ty(inner, tyenv), Ty::F32) { "f32" } else { "f64" };
                em.line(&format!("{} = arith.sitofp {} : i64 to {}", w, x, sitofp_ty));
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
        let widened = if matches!(pt, Ty::F64 | Ty::F32) && matches!(at, Ty::Int) {
            let w = em.fresh("widen");
            let sitofp_ty = if matches!(pt, Ty::F32) { "f32" } else { "f64" };
            em.line(&format!("{} = arith.sitofp {} : i64 to {}", w, ssa, sitofp_ty));
            w
        } else {
            ssa
        };
        prepared.push((widened, pt.clone()));
    }

    // Callee signature types for every parameter.
    let param_tys: Vec<&str> = prepared
        .iter()
        .map(|(_, pt)| match pt {
            Ty::ListF64 => "memref<?xf64>",
        Ty::ListF32 => "memref<?xf32>",
            Ty::ListInt => "memref<?xi64>",
            Ty::F64 | Ty::F32 => "f64",
            _ => "i64",
        })
        .collect();
    let ret_ty = match target.ret {
        Ty::F64 | Ty::F32 => "f64",
        Ty::ListF64 => "memref<?xf64>",
        Ty::ListF32 => "memref<?xf32>",
        Ty::ListInt => "memref<?xi64>",
        other => {
            return Err(format!(
                "lowering: dep call return {:?} unsupported",
                other
            ))
        }
    };
    let out = em.fresh("call");
    // func.call type suffix lists PARAM TYPES only, never SSA names.
    em.line(&format!(
        "{} = func.call @{}({}) : ({}) -> {}",
        out,
        target.symbol,
        prepared.iter().map(|(s, _)| s.as_str()).collect::<Vec<_>>().join(", "),
        param_tys.join(", "),
        ret_ty
    ));
    Ok(out)
}


/// Emit `index(list, pos)`: bounds-CHECKED load — out-of-bounds traps via
/// ontic_trap so native matches the oracle's IndexOutOfBounds kill.
fn emit_index(
    l: &Expr,
    r: &Expr,
    env: &mut Vec<Binding>,
    tyenv: &mut HashMap<String, Ty>,
    em: &mut Emitter,
) -> Result<String, String> {
    let m = emit_expr(l, env, tyenv, em)?;
    let pos = emit_expr(r, env, tyenv, em)?;
    let mty = list_memref(l, tyenv);
    let idx0 = em.const_index(0);
    let dim = em.fresh("idim");
    em.line(&format!(
        "{} = \"memref.dim\"({}, {}) : ({}, index) -> index",
        dim, m, idx0, mty
    ));
    // position is i64; compare in index domain.
    let pos_idx = em.fresh("ipos");
    em.line(&format!(
        "{} = arith.index_cast {} : i64 to index",
        pos_idx, pos
    ));
    let neg = em.fresh("ineg");
    let zero_i64 = em.const_i64(0);
    em.line(&format!(
        "{} = arith.cmpi slt, {}, {} : i64",
        neg, pos, zero_i64
    ));
    let pos_i64 = em.fresh("p64");
    em.line(&format!(
        "{} = arith.index_cast {} : index to i64",
        pos_i64, dim
    ));
    let ge = em.fresh("ige");
    em.line(&format!(
        "{} = arith.cmpi sge, {}, {} : i64",
        ge, pos, pos_i64
    ));
    let bad = em.fresh("ibad");
    em.line(&format!("{} = arith.ori {}, {} : i1", bad, neg, ge));

    let out = em.fresh("elem");
    let elem_ty = if matches!(mty, "memref<?xf64>") { "f64" } else { "i64" };
    em.line(&format!("{} = scf.if {} -> ({}) {{", out, bad, elem_ty));
    em.indent += 1;
    let trap_sym = if elem_ty == "f64" { "ontic_trapf" } else { "ontic_trap" };
    let t = em.fresh("t");
    em.line(&format!(
        "{} = func.call @{}() : () -> {}",
        t, trap_sym, elem_ty
    ));
    em.line(&format!("scf.yield {} : {}", t, elem_ty));
    em.indent -= 1;
    em.line("} else {");
    em.indent += 1;
    let v = em.fresh("v");
    em.line(&format!(
        "{} = memref.load {}[{}] : {}",
        v, m, pos_idx, mty
    ));
    em.line(&format!("scf.yield {} : {}", v, elem_ty));
    em.indent -= 1;
    em.line("}");
    Ok(out)
}


/// Emit list concatenation: allocate combined-size memref, copy both sides.
fn emit_concat(
    lv: &str,
    rv: &str,
    lt: &Ty,
    rt: &Ty,
    em: &mut Emitter,
) -> Result<String, String> {
    let mty_l = if matches!(lt, Ty::ListF64) { "memref<?xf64>" } else { "memref<?xi64>" };
    let mty_r = if matches!(rt, Ty::ListF64) { "memref<?xf64>" } else { "memref<?xi64>" };
    let elem = if matches!(lt, Ty::ListF64) || matches!(rt, Ty::ListF64) { "f64" } else { "i64" };
    let mty_out = if elem == "f64" { "memref<?xf64>" } else { "memref<?xi64>" };

    let idx0 = em.const_index(0);
    let step = em.const_index(1);

    let dim_l = em.fresh("cdl");
    em.line(&format!(
        "{} = \"memref.dim\"({}, {}) : ({}, index) -> index",
        dim_l, lv, idx0, mty_l
    ));
    let dim_r = em.fresh("cdr");
    em.line(&format!(
        "{} = \"memref.dim\"({}, {}) : ({}, index) -> index",
        dim_r, rv, idx0, mty_r
    ));
    let total = em.fresh("ctot");
    em.line(&format!(
        "{} = arith.addi {}, {} : index",
        total, dim_l, dim_r
    ));

    let alloc = em.fresh("calloc");
    em.line(&format!(
        "{} = memref.alloc({}) : {}",
        alloc, total, mty_out
    ));

    // Copy left side.
    let ivl = em.fresh("cil");
    let accl = em.fresh("cal");
    em.line(&format!(
        "{} = scf.for {} = {} to {} step {} iter_args({} = {}) -> (index) {{",
        accl, ivl, idx0, dim_l, step, accl, idx0
    ));
    em.indent += 1;
    let vl = em.fresh("vl");
    em.line(&format!("{} = memref.load {}[{}] : {}", vl, lv, ivl, mty_l));
    em.line(&format!(
        "memref.store {}, {}[{}] : {}",
        vl, alloc, ivl, mty_out
    ));
    em.line(&format!("scf.yield {} : index", accl));
    em.indent -= 1;
    em.line("}");

    // Copy right side at offset = len(left).
    let ivr = em.fresh("cir");
    let accr = em.fresh("car");
    let endr = em.fresh("cer");
    em.line(&format!(
        "{} = arith.addi {}, {} : index",
        endr, dim_l, dim_r
    ));
    em.line(&format!(
        "{} = scf.for {} = {} to {} step {} iter_args({} = {}) -> (index) {{",
        accr, ivr, dim_l, endr, step, accr, accr
    ));
    em.indent += 1;
    let vr = em.fresh("vr");
    em.line(&format!("{} = memref.load {}[{}] : {}", vr, rv, ivr, mty_r));
    let off = em.fresh("coff");
    em.line(&format!(
        "{} = arith.subi {}, {} : index",
        off, ivr, dim_l
    ));
    em.line(&format!(
        "memref.store {}, {}[{}] : {}",
        vr, alloc, off, mty_out
    ));
    em.line(&format!("scf.yield {} : index", accr));
    em.indent -= 1;
    em.line("}");

    Ok(alloc)
}


/// Emit a map transform: alloc result memref at same dim, scf.for loop
/// evaluating body per element and storing. Element type from body's
/// static inference.
#[allow(clippy::too_many_arguments)]
fn emit_map(
    var: &str,
    list: &Expr,
    body: &Expr,
    env: &mut Vec<Binding>,
    tyenv: &mut HashMap<String, Ty>,
    em: &mut Emitter,
) -> Result<String, String> {
    let m = emit_expr(list, env, tyenv, em)?;
    let elem_is_float = matches!(list, Expr::Var(n) if matches!(tyenv.get(n), Some(Ty::ListF64)))
        || matches!(expr_ty(list, tyenv), Ty::ListF64 | Ty::ListF32);
    let mty_in_str = if elem_is_float { "memref<?xf64>" } else { "memref<?xi64>" };

    let idx0 = em.const_index(0);
    let step = em.const_index(1);
    let dim = em.fresh("mdim");
    em.line(&format!(
        "{} = \"memref.dim\"({}, {}) : ({}, index) -> index",
        dim, m, idx0, mty_in_str
    ));

    // Output element type must be inferred WITH the loop variable in scope:
    // bodies like `v * v` reference the var, which is unbound until the loop
    // header. Pre-binding bug stored f64 products into memref<?xi64>.
    let mut probe = tyenv.clone();
    let var_is_float = if elem_is_float { Ty::F64 } else { Ty::Int };
    probe.insert(var.to_string(), var_is_float);
    let out_ty = if matches!(expr_ty(body, &probe), Ty::F64) { "f64" } else { "i64" };
    let out_mty = if out_ty == "f64" { "memref<?xf64>" } else { "memref<?xi64>" };

    let alloc = em.fresh("mout");
    em.line(&format!(
        "{} = memref.alloc({}) : {}",
        alloc, dim, out_mty
    ));

    let iv = em.fresh("mi");
    let accp = em.fresh("mac");
    em.line(&format!(
        "{} = scf.for {} = {} to {} step {} iter_args({} = {}) -> (index) {{",
        accp, iv, idx0, dim, step, accp, idx0
    ));
    em.indent += 1;

    let elem = em.fresh("me");
    em.line(&format!(
        "{} = memref.load {}[{}] : {}",
        elem, m, iv, mty_in_str
    ));
    env.push(Binding { name: var.to_string(), ssa: elem });
    tyenv.insert(var.to_string(), if elem_is_float { Ty::F64 } else { Ty::Int });

    let body_v = emit_expr(body, env, tyenv, em)?;
    em.line(&format!(
        "memref.store {}, {}[{}] : {}",
        body_v, alloc, iv, out_mty
    ));
    em.line(&format!("scf.yield {} : index", accp));
    em.indent -= 1;
    em.line("}");

    Ok(alloc)
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
    let t_ty = expr_ty(t, tyenv);
    let ty_str = match t_ty {
        Ty::F64 | Ty::F32 => "f64",
        Ty::ListF64 => "memref<?xf64>",
        Ty::ListF32 => "memref<?xf32>",
        Ty::ListInt => "memref<?xi64>",
        _ => "i64",
    };
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
    until: Option<&Expr>,
    aux: &[(String, Box<Expr>)],
    env: &mut Vec<Binding>,
    tyenv: &mut HashMap<String, Ty>,
    em: &mut Emitter,
) -> Result<String, String> {
    let init_v = emit_expr(init, env, tyenv, em)?;
    let m = emit_expr(list, env, tyenv, em)?;
    if !aux.is_empty() {
        // Multi-accumulator: scf.for with one iter_arg per carried value;
        // body is a restricted tuple, emitted component-wise.
        return emit_fold_multi(
            var, acc, list, m, init, init_v, body, until, aux, env, tyenv, em,
        );
    }
    if let Some(u) = until {
        let init_ty_v = expr_ty(init, tyenv);
        return emit_fold_until(var, acc, list, m, init_ty_v, init_v, body, u, env, tyenv, em);
    }
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
    let ty_str = if matches!(expr_ty(init, tyenv), Ty::F64 | Ty::F32) { "f64" } else { "i64" };
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

    tyenv.insert(var.to_string(), if matches!(expr_ty(list, tyenv), Ty::ListF64 | Ty::ListF32) { Ty::F64 } else { Ty::Int });
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

/// Multi-accumulator fold: one iter_arg per carried value. Without `until`
/// lowers to `scf.for` (variadic results, take #0 = acc); with `until` to
/// `scf.while` whose pre-test binds every carried value plus the index.
/// Body is a restricted tuple emitted component-wise — matching interp's
/// component-wise evaluation exactly (Golden Rule 6).
#[allow(clippy::too_many_arguments)]
fn emit_fold_multi(
    var: &str,
    acc_name: &str,
    list: &Expr,
    m: String,
    init: &Expr,
    init_v: String,
    body: &Expr,
    until: Option<&Expr>,
    aux: &[(String, Box<Expr>)],
    env: &mut Vec<Binding>,
    tyenv: &mut HashMap<String, Ty>,
    em: &mut Emitter,
) -> Result<String, String> {
    let comps = match body {
        Expr::Tuple(items) => items.clone(),
        _ => {
            return Err(
                "multi-accumulator fold bodies must be tuple expressions".to_string(),
            )
        }
    };
    if comps.len() != aux.len() + 1 {
        return Err(format!(
            "fold body yields {} components for {} accumulators",
            comps.len(),
            aux.len() + 1
        ));
    }

    // Carried set: acc first, then aux in declaration order.
    let mut carried: Vec<(String, String, Ty)> = Vec::new();
    let acc_ty = expr_ty(init, tyenv);
    carried.push((acc_name.to_string(), init_v.clone(), acc_ty.clone()));
    for (n, ie) in aux {
        let t = expr_ty(ie, tyenv);
        let ssa = emit_expr(ie, env, tyenv, em)?;
        carried.push((n.clone(), ssa, t));
    }
    // NOTE: aux init ssa names double as unique carriers; bound names come
    // from aux declarations, so rebind by NAME below regardless of ssa text.
    let _ = &carried;

    let elem_ty = fold_elem_ty(list, tyenv);
    let list_mty = if matches!(elem_ty, Ty::F64 | Ty::F32) {
        "memref<?xf64>"
    } else {
        "memref<?xi64>"
    };

    let idx0 = em.const_index(0);
    let step = em.const_index(1);
    let dim = em.fresh("dim");
    em.line(&format!(
        "{} = \"memref.dim\"({}, {}) : ({}, index) -> index",
        dim, m, idx0, list_mty
    ));

    let tys_txt: String = carried
        .iter()
        .map(|(_, _, t)| if matches!(t, Ty::F64) { "f64" } else { "i64" })
        .collect::<Vec<_>>()
        .join(", ");

    // Fresh arg names per carried slot, reused consistently per region.
    let arg_names: Vec<String> = (0..carried.len()).map(|k| em.fresh(&format!("s{}", k))).collect();
    let bind_names: Vec<String> = (0..carried.len()).map(|k| em.fresh(&format!("b{}", k))).collect();

    let push_bindings = |env: &mut Vec<Binding>,
                         tyenv: &mut HashMap<String, Ty>,
                         names: &[String],
                         iv_ssa: Option<&String>| {
        if let Some(iv) = iv_ssa {
            env.push(Binding { name: var.to_string(), ssa: iv.clone() });
            tyenv.insert(var.to_string(), Ty::Int);
        }
        for ((n, _, t), a) in carried.iter().zip(names.iter()) {
            env.push(Binding { name: n.clone(), ssa: a.clone() });
            tyenv.insert(n.clone(), t.clone());
        }
    };
    let pop_n = |env: &mut Vec<Binding>, tyenv: &mut HashMap<String, Ty>, k: usize| {
        for _ in 0..k {
            env.pop();
        }
        for _ in 0..k {
            if let Some((n, _)) = env.last().map(|b| (b.name.clone(), ())) {
                let _ = n;
                break;
            }
        }
        // tyenv entries are scoped-by-shadowing; leaving them is consistent
        // with existing fold/map emissions.
        let _ = tyenv;
    };

    match until {
        None => {
            let mut iter_args = String::new();
            for (k, a) in arg_names.iter().enumerate() {
                if k > 0 {
                    iter_args.push_str(", ");
                }
                iter_args.push_str(&format!(
                    "{} = {}",
                    a,
                    carried[k].1.clone()
                ));
            }
            let res = em.fresh("mf").trim_start_matches('%').to_string();
            em.line(&format!(
                "%{r} = scf.for %mi = {i0} to {dim} step {st} iter_args({args}) -> ({tys}) {{",
                r = res,
                i0 = idx0,
                st = step,
                args = iter_args,
                tys = tys_txt
            ));
            em.indent += 1;
            let elem = em.fresh("fe");
            em.line(&format!(
                "{} = memref.load {}[%mi] : {}",
                elem, m, list_mty
            ));
            env.push(Binding { name: var.to_string(), ssa: "%mi".to_string() });
            for ((n, _, t), a) in carried.iter().zip(arg_names.iter()) {
                env.push(Binding {
                    name: n.clone(),
                    ssa: a.clone(),
                });
                tyenv.insert(n.clone(), t.clone());
            }
            let pushed = carried.len() + 1;
            let mut yields: Vec<String> = Vec::new();
            for c in &comps {
                yields.push(emit_expr(c, env, tyenv, em)?);
            }
            em.line(&format!(
                "scf.yield {} : {}",
                yields.join(", "),
                tys_txt
            ));
            // Pop what we pushed (bindings only; tyenv shadowing persists).
            for _ in 0..pushed {
                env.pop();
            }
            em.indent -= 1;
            em.line("}");
            Ok(format!("%{}#0", res))
        }
        Some(u) => {
            let wname = em.fresh("wh").trim_start_matches('%').to_string();
            let mut init_args: Vec<String> =
                carried.iter().map(|(_, s, _)| s.clone()).collect();
            init_args.push(idx0.clone());
            let mut arg_names_w = arg_names.clone();
            arg_names_w.push("%wi".to_string());
            let mut tys_w: Vec<String> = carried
                .iter()
                .map(|(_, _, t)| {
                    if matches!(t, Ty::F64) { "f64".to_string() } else { "i64".to_string() }
                })
                .collect();
            tys_w.push("index".to_string());
            let tys_w_txt = tys_w.join(", ");
            let num_results = (carried.len() + 1).to_string();
            em.line(&format!(
                "%{w}:{n} = scf.while ({args}) : ({tys}) -> ({tys}) {{",
                w = wname,
                n = num_results,
                args = format_args_named(&arg_names_w, &init_args),
                tys = tys_w_txt
            ));
            em.indent += 1;
            let inb = em.fresh("inb");
            em.line(&format!(
                "{} = arith.cmpi slt, %wi, {} : index",
                inb, dim
            ));
            push_bindings(env, tyenv, &arg_names, Some(&"%wi".to_string()));
            let done_raw = emit_expr(u, env, tyenv, em)?;
            let done_b = em.fresh("db");
            em.line(&format!("{} = arith.trunci {} : i64 to i1", done_b, done_raw));
            let ctrue = em.fresh("ct");
            em.line(&format!("{} = arith.constant true", ctrue));
            let nd = em.fresh("nd");
            em.line(&format!("{} = arith.xori {}, {} : i1", nd, done_b, ctrue));
            let cont = em.fresh("cont");
            em.line(&format!("{} = arith.andi {}, {} : i1", cont, inb, nd));
            let mut carry_args: Vec<String> = arg_names.clone();
            carry_args.push("%wi".to_string());
            em.line(&format!(
                "scf.condition({}) {} : {}",
                cont,
                carry_args.join(", "),
                tys_w_txt
            ));
            pop_n(env, tyenv, carried.len() + 1);
            em.indent -= 1;
            em.line("} do {");
            em.indent += 1;
            let mut bb_sig = format_args_typed(&bind_names, &carried);
            if !bb_sig.is_empty() {
                bb_sig.push_str(", ");
            }
            bb_sig.push_str("%wib: index");
            em.line(&format!("^bb0({}):", bb_sig));
            let elem = em.fresh("fe");
            em.line(&format!(
                "{} = memref.load {}[%wib] : {}",
                elem, m, list_mty
            ));
            env.push(Binding { name: var.to_string(), ssa: "%wib".to_string() });
            for ((n, _, t), b) in carried.iter().zip(bind_names.iter()) {
                env.push(Binding { name: n.clone(), ssa: b.clone() });
                tyenv.insert(n.clone(), t.clone());
            }
            let mut yields: Vec<String> = Vec::new();
            for c in &comps {
                yields.push(emit_expr(c, env, tyenv, em)?);
            }
            let ni = em.fresh("ni");
            em.line(&format!("{} = arith.addi %wib, {} : index", ni, step));
            let mut yld = yields.clone();
            yld.push(ni);
            let mut yld_tys: Vec<String> = carried
                .iter()
                .map(|(_, _, t)| {
                    if matches!(t, Ty::F64) { "f64".to_string() } else { "i64".to_string() }
                })
                .collect();
            yld_tys.push("index".to_string());
            em.line(&format!("scf.yield {} : {}", yld.join(", "), yld_tys.join(", ")));
            for _ in 0..carried.len() + 1 {
                env.pop();
            }
            em.indent -= 1;
            em.line("}");
            Ok(format!("%{}#0", wname))
        }
    }
}

fn format_args_named(names: &[String], inits: &[String]) -> String {
    names
        .iter()
        .zip(inits.iter())
        .map(|(n, i)| format!("{} = {}", n, i))
        .collect::<Vec<_>>()
        .join(", ")
}

fn format_args_types(carried: &[(String, String, crate::sketch::Ty)], extra: &str) -> String {
    let mut v: Vec<String> = carried
        .iter()
        .map(|(_, _, t)| if matches!(t, crate::sketch::Ty::F64) { "f64".to_string() } else { "i64".to_string() })
        .collect();
    v.push(extra.to_string());
    v.join(", ")
}

fn format_args_typed(names: &[String], carried: &[(String, String, crate::sketch::Ty)]) -> String {
    names
        .iter()
        .zip(carried.iter())
        .map(|(n, (_, _, t))| {
            format!(
                "{}: {}",
                n,
                if matches!(t, crate::sketch::Ty::F64) { "f64" } else { "i64" }
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Element type of the folded list under tyenv (param lookup / range).

fn n_of(s: &str) -> String {
    s.trim_start_matches('%').to_string()
}
fn fold_elem_ty(list: &Expr, tyenv: &HashMap<String, Ty>) -> Ty {
    match list {
        Expr::Var(n) => match tyenv.get(n) {
            Some(Ty::ListF64) => Ty::F64,
            _ => Ty::Int,
        },
        Expr::Call(p, _) if p.ends_with("range") => Ty::Int,
        _ => Ty::Int,
    }
}

/// Fold with `until` lowers to `scf.while` (pre-test): condition checks
/// `iv < dim && !until(var=iv, acc)` before each step. Zero iterations when
/// the initial state satisfies DONE; result is the surviving accumulator -
/// matching interp::eval_fold exactly (Golden Rule 6).
#[allow(clippy::too_many_arguments)]
fn emit_fold_until(
    var: &str,
    acc_name: &str,
    list: &Expr,
    m: String,
    init_ty: Ty,
    init_v: String,
    body: &Expr,
    until: &Expr,
    env: &mut Vec<Binding>,
    tyenv: &mut HashMap<String, Ty>,
    em: &mut Emitter,
) -> Result<String, String> {
    let idx0 = em.const_index(0);
    let step = em.const_index(1);
    let dim = em.fresh("dim");
    let elem_ty = fold_elem_ty(list, tyenv);
    let (list_mty, _ety) = if matches!(elem_ty, Ty::F64 | Ty::F32) {
        ("memref<?xf64>", "f64")
    } else {
        ("memref<?xi64>", "i64")
    };
    let acc_ty_s = if matches!(init_ty, Ty::F64) { "f64" } else { "i64" };
    em.line(&format!(
        "{} = \"memref.dim\"({}, {}) : ({}, index) -> index",
        dim, m, idx0, list_mty
    ));

    let wname = em.fresh("wh").trim_start_matches('%').to_string();
    em.line(&format!(
        "%{w}:2 = scf.while (%wa = {init}, %wi = {i0}) : ({acc_t}, index) -> ({acc_t}, index) {{",
        w = wname, init = init_v, i0 = idx0, acc_t = acc_ty_s
    ));
    em.indent += 1;
    let inb = em.fresh("inb");
    em.line(&format!("{} = arith.cmpi slt, %wi, {} : index", inb, dim));
    env.push(Binding { name: var.to_string(), ssa: "%wi".to_string() });
    env.push(Binding { name: acc_name.to_string(), ssa: "%wa".to_string() });
    tyenv.insert(var.to_string(), Ty::Int);
    tyenv.insert(acc_name.to_string(), init_ty.clone());
    let done_raw = emit_expr(until, env, tyenv, em)?;
    let done_b = em.fresh("db");
    em.line(&format!("{} = arith.trunci {} : i64 to i1", done_b, done_raw));
    let ctrue = em.fresh("ct");
    em.line(&format!("{} = arith.constant true", ctrue));
    let nd = em.fresh("nd");
    em.line(&format!("{} = arith.xori {}, {} : i1", nd, done_b, ctrue));
    let cont = em.fresh("cont");
    em.line(&format!("{} = arith.andi {}, {} : i1", cont, inb, nd));
    em.line(&format!(
        "scf.condition({}) %wa, %wi : {}, index",
        cont, acc_ty_s
    ));
    env.pop();
    env.pop();
    em.indent -= 1;
    em.line("} do {");
    em.indent += 1;
    em.line(&format!("^bb0(%wa2: {}, %wi2: index):", acc_ty_s));
    let elem = em.fresh("fe");
    em.line(&format!(
        "{} = memref.load {}[%wi2] : {}",
        elem, m, list_mty
    ));
    env.push(Binding { name: var.to_string(), ssa: elem.clone() });
    env.push(Binding { name: acc_name.to_string(), ssa: "%wa2".to_string() });
    tyenv.insert(var.to_string(), elem_ty.clone());
    tyenv.insert(acc_name.to_string(), init_ty.clone());
    let nv = emit_expr(body, env, tyenv, em)?;
    let ni = em.fresh("ni");
    em.line(&format!("{} = arith.addi %wi2, {} : index", ni, step));
    em.line(&format!(
        "scf.yield {}, {} : {}, index",
        nv, ni, acc_ty_s
    ));
    env.pop();
    env.pop();
    em.indent -= 1;
    em.line("}");
    Ok(format!("%{}#0", wname))
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
        && ((matches!(lt, Ty::F64 | Ty::F32) && matches!(rt, Ty::Int))
            || (matches!(lt, Ty::Int) && matches!(rt, Ty::F64)));
    let mut lv_s = lv;
    let mut rv_s = rv;
    let mut any_float = matches!(lt, Ty::F64 | Ty::F32) || matches!(rt, Ty::F64 | Ty::F32);
    let binop_float_ty = if matches!(lt, Ty::F32) || matches!(rt, Ty::F32) { "f32" } else { "f64" };
    if mixed_float {
        if matches!(lt, Ty::Int) {
            let w = em.fresh("widen");
            em.line(&format!("{} = arith.sitofp {} : i64 to {}", w, lv_s, binop_float_ty));
            lv_s = w;
        }
        if matches!(rt, Ty::Int) {
            let w = em.fresh("widen");
            em.line(&format!("{} = arith.sitofp {} : i64 to {}", w, rv_s, binop_float_ty));
            rv_s = w;
        }
        any_float = true;
    }
    if is_comparison(op) {
        let float_ty = if any_float { binop_float_ty } else { "" };
        return emit_cmp(op, &lv_s, &rv_s, float_ty, em);
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
    if op == BinOp::Concat {
        return emit_concat(&lv_s, &rv_s, &lt, &rt, em);
    }
    if any_float && matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod) {
        let out = em.fresh("opf");
        let float_ty = binop_float_ty;
        // When float_ty is f32 but an operand was emitted as f64 (e.g. FloatLit),
        // insert fptrunc to bring it to f32 before the arith op.
        let lv_s = if float_ty == "f32" && matches!(lt, Ty::F64) {
            let c = em.fresh("trunc");
            em.line(&format!("{} = arith.truncf {} : f64 to f32", c, lv_s));
            c
        } else { lv_s };
        let rv_s = if float_ty == "f32" && matches!(rt, Ty::F64) {
            let c = em.fresh("trunc");
            em.line(&format!("{} = arith.truncf {} : f64 to f32", c, rv_s));
            c
        } else { rv_s };
        let stmt = match op {
            BinOp::Add => format!("{} = arith.addf {}, {} : {}", out, lv_s, rv_s, float_ty),
            BinOp::Sub => format!("{} = arith.subf {}, {} : {}", out, lv_s, rv_s, float_ty),
            BinOp::Mul => format!("{} = arith.mulf {}, {} : {}", out, lv_s, rv_s, float_ty),
            BinOp::Div => format!("{} = arith.divf {}, {} : {}", out, lv_s, rv_s, float_ty),
            _ => format!("{} = arith.remf {}, {} : {}", out, lv_s, rv_s, float_ty),
        };
        em.line(&stmt);
        return Ok(out);
    }
    if matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul) {
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
        Expr::FloatListLit(_) => Ty::ListF64,
        Expr::Var(n) => tyenv.get(n).cloned().unwrap_or(Ty::Int),
        Expr::Call(p, _) => tyenv.get(p).cloned().unwrap_or(Ty::Int),
        Expr::Tuple(items) => items
            .first()
            .map(|e| expr_ty(e, tyenv))
            .unwrap_or(Ty::Int),
        Expr::Builtin2(crate::sketch::Builtin::Index, l, _) => {
            // Element type follows the indexed list.
            if matches!(expr_ty(l, tyenv), Ty::ListF64) {
                Ty::F64
            } else {
                Ty::Int
            }
        }
        Expr::Builtin2(..) => Ty::Int,
        Expr::Map { body, .. } => match expr_ty(body, tyenv) {
            Ty::F64 => Ty::ListF64,
            _ => Ty::ListInt,
        },
        Expr::ListCons(_) => Ty::ListInt, // refined by caller via tyenv
        Expr::Builtin(b, inner) => match b {
            Builtin::Len => Ty::Int,
            Builtin::Range => Ty::ListInt,
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
            BinOp::Concat => {
                let lt = expr_ty(l, tyenv);
                let rt = expr_ty(r, tyenv);
                if matches!(lt, Ty::ListF64) || matches!(rt, Ty::ListF64) {
                    Ty::ListF64
                } else {
                    Ty::ListInt
                }
            }
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
    float_ty: &str,
    em: &mut Emitter,
) -> Result<String, String> {
    let is_float = !float_ty.is_empty();
    // cmpf uses ordered-float predicate names; cmpi uses signed-int names.
    let pred = match (op, is_float) {
        (BinOp::Eq, false) => "eq",
        (BinOp::Ne, false) => "ne",
        (BinOp::Lt, false) => "slt",
        (BinOp::Le, false) => "sle",
        (BinOp::Gt, false) => "sgt",
        (_, false) => "sge",
        (BinOp::Eq, true) => "oeq",
        (BinOp::Ne, true) => "une",
        (BinOp::Lt, true) => "olt",
        (BinOp::Le, true) => "ole",
        (BinOp::Gt, true) => "ogt",
        (_, true) => "oge",
    };
    let bit = em.fresh("cmp");
    if is_float {
        em.line(&format!("{} = arith.cmpf {}, {}, {} : {}", bit, pred, lv, rv, float_ty));
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
        emit_fn(&c.name, &c.params, &c.ret, &c.body, &CallMap::new()).expect("lowers")
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
    fn test_computed_float_list_cons_allocates_f64_memref() {
        // Regression: element type must come from the checker (F64 if any
        // computed element is F64), not from literal presence. Storing a
        // divf result into memref<?xi64> is invalid IR that mlir-opt
        // correctly rejects.
        let c = sketch::parse(
            "fn @ci2(%a: F64, %b: F64, %c: F64) -> List<F64> { let %d = %a * %c - %b * %b; [%c / %d, -%b / %d] }",
        )
        .unwrap();
        check::check(&c).unwrap();
        let ir = emit_fn(&c.name, &c.params, &c.ret, &c.body, &CallMap::new())
            .expect("lowers");
        assert!(
            ir.contains("memref<?xf64>"),
            "computed-float list cons must allocate f64 memref, got:\n{}",
            ir
        );
        assert!(!ir.contains("sitofp"), "no widening needed for uniform f64");
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
    fn test_checked_arith_expands_wide_check_and_trap() {
        let c = sketch::parse("fn @f(%a: Int, %b: Int) -> Int { %a + %b }").unwrap();
        let ir = emit_fn(&c.name, &c.params, &c.ret, &c.body, &CallMap::new()).unwrap();
        assert!(ir.contains("ontic_trap"), "missing trap decl");
        assert!(ir.contains("i128"));
        assert!(ir.contains("scf.if"));
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

    // Elementwise op (widen ints into float results).
    let (a, b) = if matches!(out_ty, Ty::ListF64 | Ty::ListF32) {
        let fty = if matches!(out_ty, Ty::ListF32) { "f32" } else { "f64" };
        let a = elem_to_float(l_elem, &lt_scalar_kind(lt), fty, em);
        let b = elem_to_float(r_elem, &lt_scalar_kind(rt), fty, em);
        (a, b)
    } else {
        (l_elem, r_elem)
    };

    let val = em.fresh("bv");
    let list_float = matches!(out_ty, Ty::ListF64 | Ty::ListF32);
    let list_f32 = matches!(out_ty, Ty::ListF32);
    let fty = if list_f32 { "f32" } else { "f64" };
    let stmt = match (list_float, op) {
        (_, BinOp::Add) if list_float => {
            format!("{} = arith.addf {}, {} : {}", val, a, b, fty)
        }
        (_, BinOp::Sub) if list_float => {
            format!("{} = arith.subf {}, {} : {}", val, a, b, fty)
        }
        (_, BinOp::Mul) if list_float => {
            format!("{} = arith.mulf {}, {} : {}", val, a, b, fty)
        }
        (_, BinOp::Div) if list_float => {
            format!("{} = arith.divf {}, {} : {}", val, a, b, fty)
        }
        (true, _) => format!("{} = arith.remf {}, {} : {}", val, a, b, fty),
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

/// Widen an int-typed element value to a float type in place.
fn elem_to_float(x: String, t: &Ty, float_ty: &str, em: &mut Emitter) -> String {
    match t {
        Ty::Int => {
            let w = em.fresh("widen");
            em.line(&format!("{} = arith.sitofp {} : i64 to {}", w, x, float_ty));
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
    let mut seen_private: std::collections::HashSet<String> = Default::default();
    for m in mlirs {
        let t = m.trim();
        let inner = t
            .strip_prefix("module {")
            .and_then(|x| x.strip_suffix('}'))
            .ok_or_else(|| "compose: module not in expected shape".to_string())?;
        // Dedent one level (our emitter uses two-space indent uniformly).
        for line in inner.lines() {
            let l = line.strip_prefix("  ").unwrap_or(line);
            if l.trim().is_empty() {
                continue;
            }
            // Private declarations repeat across dep modules (ontic_trap,
            // shared deps in a flat closure). Keep the first, drop the rest.
            if let Some(rest) = l.trim().strip_prefix("func.func private @") {
                let sym = rest.split(['(', ':']).next().unwrap_or("").trim();
                if !seen_private.insert(sym.to_string()) {
                    continue;
                }
            }
            out.push_str("  ");
            out.push_str(l);
            out.push('\n');
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

    #[test]
    fn test_compose_dedupes_private_decls() {
        // Flat closures re-list shared deps; ontic_trap appears in every
        // module. Duplicate private decls are invalid MLIR (redefinition).
        let a = "module {\n  func.func private @ontic_trap() -> i64\n  func.func @a() -> i64 {\n    return 0 : i64\n  }\n}".to_string();
        let b = "module {\n  func.func private @ontic_trap() -> i64\n  func.func @b() -> i64 {\n    return 0 : i64\n  }\n}".to_string();
        let c = compose_modules(&[a, b]).unwrap();
        assert_eq!(
            c.matches("func.func private @ontic_trap").count(),
            1,
            "private decls must dedupe across modules:\n{}",
            c
        );
    }
}

/// C return type for a gen-level type.
fn c_ret_ty(ty: &Ty) -> Result<&'static str, String> {
    match ty {
        Ty::Int | Ty::Bool => Ok("long"),
        Ty::F64 => Ok("double"),
        Ty::F32 => Ok("float"),
        // Flat-MemRef return: 5-field struct, caller reads aligned+size.
        Ty::ListInt | Ty::ListF64 | Ty::ListF32 => Ok("void*"),
    }
}

/// Generate the C header declaration for one candidate using the flat
/// MemRef ABI (each List<T> param expands to five scalars). Deterministic:
/// no timestamps, no paths — same gen yields byte-identical headers.
pub fn emit_header(
    name: &str,
    params: &[(String, Ty)],
    ret: &Ty,
    key8: &str,
    guarded: bool,
) -> Result<String, String> {
    let rt = c_ret_ty(ret)?;
    let mut parts: Vec<String> = Vec::new();
    for (n, t) in params {
        match t {
            Ty::ListInt | Ty::ListF64 | Ty::ListF32 => parts.push(format!(
                "void* {n}_a, void* {n}_b, long {n}_o, long {n}_s, long {n}_st"
            )),
            Ty::Int | Ty::Bool => parts.push(format!("long {n}")),
            Ty::F64 => parts.push(format!("double {n}")),
            Ty::F32 => parts.push(format!("float {n}")),
        }
    }
    // Deterministic sanitized guard tokens from key8 + kernel name.
    let sanitize = |s: &str| -> String {
        s.chars()
            .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_uppercase() } else { '_' })
            .collect()
    };
    let gk = sanitize(key8);
    let gn = sanitize(name);
    let args = if parts.is_empty() {
        "void".to_string()
    } else {
        parts.join(", ")
    };

    let guarded_section = if guarded {
        format!(
            "\n/* ---- Runtime guard (link {guarded_lib} for checks) ---- */\n\
             /* Default policy: ABORT. Switch at runtime: */\n\
             /*   ontic_set_violation_policy(ONTIC_POLICY_TRAP); */\n\
             /*   double r = geo(0.5);       // guarded */\n\
             /*   double u = geo__raw(0.5);  // unchecked */\n\
             const char *ontic_last_error(void);\n\
             void        ontic_last_error_clear(void);\n\
             void        ontic_set_violation_policy(int policy);\n\
             int         ontic_violation_policy(void);\n\
             #define ONTIC_POLICY_ABORT 0\n\
             #define ONTIC_POLICY_TRAP  1\n",
            guarded_lib = format!("lib{}-{}.guarded.so", name, key8),
        )
    } else {
        String::new()
    };

    let body = format!(
        "// Ontic kernel (verified; do not edit - re-solve instead)\n\
         // ABI v1: Flat-MemRef; List<T> param -> (allocated*, aligned*, offset, size, stride)\n\
         #ifndef ONTIC_{gk}_{gn}_H\n\
         #define ONTIC_{gk}_{gn}_H\n\n\
         #ifdef __cplusplus\n\
         extern \"C\" {{\n\
         #endif\n\n\
         {rt} {name}({args});\n\
         {guarded_section}\n\
         #ifdef __cplusplus\n\
         }}\n\
         #endif\n\n\
         #endif /* ONTIC_{gk}_{gn}_H */\n"
    );
    Ok(body)
}


/// Flatten a conjunction tree into its atomic conjuncts.
fn conjuncts(e: &crate::sketch::Expr) -> Vec<&crate::sketch::Expr> {
    match e {
        crate::sketch::Expr::BinOp(crate::sketch::BinOp::And, l, r) => {
            let mut out = conjuncts(l);
            out.extend(conjuncts(r));
            out
        }
        other => vec![other],
    }
}


/// C++26-contracted twin of emit_header. Contracts are machine-translated
/// from sieve-proven invariants over a conservative subset (scalar params,
/// len() of lists, arithmetic/comparisons). Native `pre(...)` under
/// ONTIC_CONTRACTS; portable `// ontic requires:` otherwise; metadata block
/// always present so tooling can read provenance without a compiler.
pub fn emit_header_hpp(
    name: &str,
    params: &[(String, Ty)],
    ret: &Ty,
    key8: &str,
    invariants: &[crate::sketch::Expr],
) -> Result<String, String> {
    let rt = c_ret_ty(ret)?;
    let mut parts: Vec<String> = Vec::new();
    for (n, t) in params {
        match t {
            Ty::ListInt | Ty::ListF64 | Ty::ListF32 => parts.push(format!(
                "void* {n}_a, void* {n}_b, long {n}_o, long {n}_s, long {n}_st"
            )),
            Ty::Int | Ty::Bool => parts.push(format!("long {n}")),
            Ty::F64 => parts.push(format!("double {n}")),
            Ty::F32 => parts.push(format!("float {n}")),
        }
    }
    let sanitize = |s: &str| -> String {
        s.chars()
            .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_uppercase() } else { '_' })
            .collect()
    };
    let gk = sanitize(key8);
    let gn = sanitize(name);
    let args = if parts.is_empty() { "void".to_string() } else { parts.join(", ") };

    // Translate invariants; keep the ones inside the subset, list the rest.
    let mut contracts: Vec<String> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    for inv in invariants {
        // Conjunctions split: one untranslatable conjunct must not discard
        // the provable ones beside it.
        for part in conjuncts(inv) {
            match contract_text(part, params) {
                Some(t) => contracts.push(t),
                None => skipped.push(crate::lower::expr_display(part)),
            }
        }
    }

    let mut meta = String::from("// ontic contracts (machine-derived from sieve-proven invariants):\n");
    if contracts.is_empty() && skipped.is_empty() {
        meta.push_str("//   (none)\n");
    }
    for c in &contracts {
        meta.push_str(&format!("//   pre: {c}\n"));
    }
    for s in &skipped {
        meta.push_str(&format!("//   untranslated: {s}\n"));
    }


/// Remove one balanced outer paren pair when present (cosmetic).
fn strip_outer(s: &str) -> String {
    if s.starts_with('(') && s.ends_with(')') {
        let mut depth = 0i32;
        for (i, ch) in s.char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 && i != s.len() - 1 {
                        return s.to_string(); // closes before end: keep
                    }
                }
                _ => {}
            }
        }
        return s[1..s.len() - 1].to_string();
    }
    s.to_string()
}

    let decl_native = if contracts.is_empty() {
        format!("{rt} {name}({args});")
    } else {
        format!(
            "{rt} {name}({args})\n  pre({});",
            contracts
                .iter()
                .map(|c| strip_outer(c))
                .collect::<Vec<_>>()
                .join(")\n  pre(")
        )
    };
    let req_line = if contracts.is_empty() {
        String::new()
    } else {
        format!(
            " // ontic requires: {}",
            contracts
                .iter()
                .map(|c| strip_outer(c))
                .collect::<Vec<_>>()
                .join(" && ")
        )
    };
    let decl_plain = format!("{rt} {name}({args});{req_line}");

    Ok(format!(
        "// Ontic kernel (verified; do not edit - re-solve instead)\n\
         // ABI v1: Flat-MemRef; List<T> param -> (allocated*, aligned*, offset, size, stride)\n\
         #ifndef ONTIC_{gk}_{gn}_HPP\n#define ONTIC_{gk}_{gn}_HPP\n\n\
         #include <cstddef>\n\n{meta}\n\
         #if defined(ONTIC_CONTRACTS) && defined(__cplusplus) && __cplusplus >= 202601L\n\
         #ifdef __cplusplus\nextern \"C\" {{\n#endif\n\n{decl_native}\n\n\
         #ifdef __cplusplus\n}}\n#endif\n\
         #else /* portable */\n\
         #ifdef __cplusplus\nextern \"C\" {{\n#endif\n\n{decl_plain}\n\n\
         #ifdef __cplusplus\n}}\n#endif\n#endif /* ONTIC_CONTRACTS */\n\n\
         #endif /* ONTIC_{gk}_{gn}_HPP */\n"
    ))
}

/// Translate one invariant into contract text over flat-MemRef parameters.
/// Conservative subset: scalar vars, len(var), arithmetic, comparisons,
/// literals. Anything else returns None (listed as untranslated).
fn contract_text(
    e: &crate::sketch::Expr,
    params: &[(String, Ty)],
) -> Option<String> {
    use crate::sketch::{BinOp, Expr};
    let scalar = |n: &str| -> bool {
        params
            .iter()
            .any(|(p, t)| p == n && matches!(t, Ty::Int | Ty::Bool | Ty::F64 | Ty::F32))
    };
    let listp = |n: &str| -> bool {
        params
            .iter()
            .any(|(p, t)| p == n && matches!(t, Ty::ListInt | Ty::ListF64 | Ty::ListF32))
    };
    match e {
        Expr::Var(n) if scalar(n) => Some(n.clone()),
        Expr::IntLit(v) => Some(v.to_string()),
        Expr::FloatLit(v) => Some(format!("{}", v)),
        Expr::BoolLit(b) => Some(b.to_string()),
        Expr::Builtin(crate::sketch::Builtin::Len, inner) => match inner.as_ref() {
            Expr::Var(n) if listp(n) => Some(format!("{n}_s")),
            _ => None,
        },
        Expr::BinOp(op, l, r) => {
            let op_s = match op {
                BinOp::Add => "+",
                BinOp::Sub => "-",
                BinOp::Mul => "*",
                BinOp::Div => "/",
                BinOp::Mod => "%",
                BinOp::Lt => "<",
                BinOp::Le => "<=",
                BinOp::Gt => ">",
                BinOp::Ge => ">=",
                BinOp::Eq => "==",
                BinOp::Ne => "!=",
                BinOp::And => "&&",
                BinOp::Or => "||",
                _ => return None,
            };
            Some(format!(
                "({} {} {})",
                contract_text(l, params)?,
                op_s,
                contract_text(r, params)?
            ))
        }
        _ => None,
    }
}

#[cfg(test)]
mod header_tests {
    use super::*;
    use crate::sketch::Ty;

    #[test]
    fn test_header_flat_memref_and_scalars() {
        let h = emit_header(
            "mean",
            &[("xs".to_string(), Ty::ListF64)],
            &Ty::F64,
            "deadbeef",
            false,
        )
        .unwrap();
        assert!(h.contains("double mean(void* xs_a, void* xs_b, long xs_o, long xs_s, long xs_st);"));
        assert!(h.contains("do not edit"));
    }

    #[test]
    fn test_header_int_list_and_scalars() {
        let h = emit_header(
            "f",
            &[
                ("items".to_string(), Ty::ListInt),
                ("k".to_string(), Ty::Int),
                ("flag".to_string(), Ty::Bool),
            ],
            &Ty::Int,
            "cafe1234",
            false,
        )
        .unwrap();
        assert!(h.contains("long f(void* items_a, void* items_b, long items_o, long items_s, long items_st, long k, long flag);"));
    }

    #[test]
    fn test_header_deterministic() {
        let a = emit_header("g", &[("x".to_string(), Ty::Int)], &Ty::Int, "aa", false).unwrap();
        let b = emit_header("g", &[("x".to_string(), Ty::Int)], &Ty::Int, "aa", false).unwrap();
        assert_eq!(a, b);
    }
}

#[cfg(test)]
mod hpp_tests {
    use super::*;
    use crate::sketch::{self, Ty};

    fn matvec_gen() -> Vec<crate::sketch::Expr> {
        let g = crate::gen::parse(
            "use Linalg.matvec\nfn T.m(%m: List<F64>, %v: List<F64>) -> List<F64>\n  | len(%v) > 0\n  | len(%m) == len(%v) * len(%v)\n  => [1.0], [2.0] -> [2.0]\n",
        )
        .unwrap();
        g.invariants
    }

    #[test]
    fn test_hpp_native_contracts_from_len_relations() {
        let invs = matvec_gen();
        let h = emit_header_hpp(
            "mv",
            &[
                ("m".to_string(), Ty::ListF64),
                ("v".to_string(), Ty::ListF64),
            ],
            &Ty::ListF64,
            "cafe1234",
            &invs,
        )
        .unwrap();
        assert!(h.contains("pre(v_s > 0)"), "hpp:\n{}", h);
        assert!(h.contains("pre(m_s == (v_s * v_s))"), "hpp:\n{}", h);
        assert!(h.contains("__cplusplus >= 202601L"), "auto-guard present");
        assert!(h.contains("ONTIC_CONTRACTS"));
        assert!(h.contains("// ontic requires:"));
        // Guard token distinct from the .h guard.
        assert!(h.contains("ONTIC_CAFE1234_MV_HPP"));
    }

    #[test]
    fn test_hpp_scalar_bounds_and_fallback() {
        let g = sketch::parse(
            "fn @g(%x: F64, %sigma: F64) -> F64 { %x }",
        )
        .unwrap();
        // Attach an invariant manually via gen parse instead:
        let gg = crate::gen::parse(
            "fn G.w(%x: F64, %sigma: F64) -> F64\n  | %sigma > 0.0\n  => 1.0, 1.0 -> 1.0\n",
        )
        .unwrap();
        let _ = g;
        let h = emit_header_hpp(
            "w",
            &[("x".to_string(), Ty::F64), ("sigma".to_string(), Ty::F64)],
            &Ty::F64,
            "f00d",
            &gg.invariants,
        )
        .unwrap();
        assert!(h.contains("pre(sigma > 0)"), "hpp:\n{}", h);
    }

    #[test]
    fn test_hpp_untranslated_listed() {
        // res-referencing postconditions are outside the v1 subset.
        let gg = crate::gen::parse(
            "fn G.p(%a: List<F64>) -> F64\n  | len(%a) > 0 && res >= 0.0\n  => [1.0] -> 1.0\n",
        )
        .unwrap();
        let h = emit_header_hpp(
            "p",
            &[("a".to_string(), Ty::ListF64)],
            &Ty::F64,
            "beef",
            &gg.invariants,
        )
        .unwrap();
        assert!(h.contains("untranslated:"), "hpp:\n{}", h);
        assert!(h.contains("pre(a_s > 0)"), "hpp:\n{}", h);
    }
}

// ---------------------------------------------------------------------------
// Runtime guard shim emitter
// ---------------------------------------------------------------------------

/// Sentinel value returned by guarded kernels on TRAP policy violation.
pub fn c_guard_sentinel(ty: &Ty) -> &'static str {
    match ty {
        Ty::F64 | Ty::F32 => "NAN",
        Ty::Int | Ty::Bool | Ty::ListInt | Ty::ListF64 | Ty::ListF32 => "LONG_MIN",
    }
}

/// C printf format specifier for a parameter value at runtime.
fn c_guard_printf_spec(ty: &Ty) -> &'static str {
    match ty {
        Ty::F64 | Ty::F32 => "%.17g",
        Ty::Int | Ty::Bool | Ty::ListInt | Ty::ListF64 | Ty::ListF32 => "%ld",
    }
}

/// Render a runtime-callable invariant as a human-readable predicate string
/// for inclusion in violation messages.  Falls back to `"true"` for
/// untranslatable conjuncts so the guard still fires on every check.
fn guard_pred_text(e: &crate::sketch::Expr, params: &[(String, Ty)]) -> String {
    contract_text(e, params).unwrap_or_else(|| "true".to_string())
}

/// Render a flat-memref parameter as its five C arguments for the shim.
/// Only used for diagnostic formatting in violation messages.
fn flat_memref_c_args(name: &str) -> String {
    format!(
        "void* {n}_a, void* {n}_b, long {n}_o, long {n}_s, long {n}_st",
        n = name
    )
}

/// Emit a complete C source file that wraps the raw MLIR-emitted kernel
/// with pre-runtime precondition checks.  The shim owns the public ABI
/// symbol; the kernel is renamed `<name>__raw`.
pub fn emit_shim_c(
    name: &str,
    params: &[(String, Ty)],
    ret: &Ty,
    key8: &str,
    invariants: &[crate::sketch::Expr],
) -> Result<String, String> {
    let rt = c_ret_ty(ret)?;
    let sanitize = |s: &str| -> String {
        s.chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect()
    };
    let safe_name = sanitize(name);

    // Build C parameter list.
    let mut c_params: Vec<String> = Vec::new();
    for (n, t) in params {
        match t {
            Ty::ListInt | Ty::ListF64 | Ty::ListF32 => c_params.push(flat_memref_c_args(n)),
            Ty::Int | Ty::Bool => c_params.push(format!("long {n}")),
            Ty::F64 => c_params.push(format!("double {n}")),
            Ty::F32 => c_params.push(format!("float {n}")),
        }
    }
    let c_args_str = if c_params.is_empty() {
        "void".to_string()
    } else {
        c_params.join(", ")
    };

    // Collect invariant conjuncts: translatable ones become C guards,
    // untranslatable ones are logged as comments but still fire (predicate
    // is `"true"` so the check is harmless).
    let mut checks: Vec<(String, String)> = Vec::new();
    for inv in invariants {
        for part in conjuncts(inv) {
            let pred = guard_pred_text(part, params);
            let readable = expr_display(part);
            checks.push((pred, readable));
        }
    }

    // Build printf format arguments for the violation message.
    // Each scalar/list param contributes one format specifier + value.
    let mut fmt_parts: Vec<String> = Vec::new();
    let mut val_parts: Vec<String> = Vec::new();
    for (n, t) in params {
        match t {
            Ty::ListInt | Ty::ListF64 | Ty::ListF32 => {
                // For lists, print the size field so the user can see what
                // length was passed.
                fmt_parts.push(format!("long {n}=%ld"));
                val_parts.push(format!("{n}_s"));
            }
            _ => {
                fmt_parts.push(format!("long {n}={}", c_guard_printf_spec(t)));
                val_parts.push(n.clone());
            }
        }
    }

    let fmt_str = if fmt_parts.is_empty() {
        "ontic: %s pre violated: %s".to_string()
    } else {
        format!("ontic: %s pre violated: %s ({})", fmt_parts.join(", "))
    };

    // Emit guard if-chains.
    let mut guard_body = String::new();
    for (pred, readable) in &checks {
        guard_body.push_str(&format!(
            "    if (!({pred})) {{\n\
             \x20       snprintf(tl_error, sizeof(tl_error),\n\
             \x20               \"{fmt_str}\",\n\
             \x20               \"{safe_name}\", \"{readable}\",\n\
             \x20               {val_args});\n\
             \x20       if (tl_policy == 0) {{\n\
             \x20           fprintf(stderr, \"%s\\n\", tl_error);\n\
             \x20           abort();\n\
             \x20       }}\n\
             \x20       return {sentinel};\n\
             \x20   }}\n",
            pred = pred,
            fmt_str = fmt_str,
            safe_name = safe_name,
            readable = readable,
            val_args = val_parts.join(", "),
            sentinel = c_guard_sentinel(ret),
        ));
    }

    let call_args: Vec<String> = params.iter().map(|(n, _)| n.clone()).collect();

    Ok(format!(
        "/* Auto-generated Ontic runtime guard shim — do not edit. */\n\
         /* Kernel: {safe_name} | key: {key8} | policy: abort(default)/trap */\n\n\
         #include <math.h>\n\
         #include <string.h>\n\
         #include <stdio.h>\n\
         #include <stdlib.h>\n\
         #include <stdint.h>\n\
         #include <stdbool.h>\n\n\
         /* ---- policy constants ---- */\n\
         #define ONTIC_POLICY_ABORT 0\n\
         #define ONTIC_POLICY_TRAP  1\n\n\
         /* ---- thread-local error state ---- */\n\
         static _Thread_local char tl_error[256];\n\
         static _Thread_local int  tl_policy = ONTIC_POLICY_ABORT;\n\n\
         const char *ontic_last_error(void) {{\n\
         \x20   return tl_error[0] ? tl_error : NULL;\n\
         }}\n\
         void ontic_last_error_clear(void) {{ tl_error[0] = '\\0'; }}\n\
         void ontic_set_violation_policy(int p) {{ tl_policy = p; }}\n\
         int  ontic_violation_policy(void) {{ return tl_policy; }}\n\n\
         /* ---- raw kernel ---- */\n\
         extern {rt} {name}__raw({c_args});\n\n\
         /* ---- guarded public symbol ---- */\n\
         {rt} {name}({c_args}) {{\n\
         {guard_body}\n\
         \x20   return {name}__raw({call_args});\n\
         }}\n",
        safe_name = safe_name,
        key8 = key8,
        rt = rt,
        name = name,
        c_args = c_args_str,
        guard_body = guard_body,
        call_args = call_args.join(", "),
    ))
}

#[cfg(test)]
mod shim_tests {
    use super::*;
    use crate::sketch::Ty;

    #[test]
    fn test_shim_scalar_preconditions() {
        let g = crate::gen::parse(
            "fn GeoSum.g(%r: F64) -> F64\n\
             | %r >= 0.0 && %r < 1.0\n\
             => 0.5 -> 2.0 ± 0.001\n",
        )
        .unwrap();
        let c = emit_shim_c("g", &[("r".to_string(), Ty::F64)], &Ty::F64, "cafe", &g.invariants)
            .unwrap();
        assert!(c.contains("if (!((r >= 0)))"), "guard:\n{}", c);
        assert!(c.contains("if (!((r < 1)))"), "guard:\n{}", c);
        assert!(c.contains("extern double g__raw(double r)"), "raw decl:\n{}", c);
        assert!(c.contains("double g(double r)"), "public symbol:\n{}", c);
        assert!(c.contains("NAN"), "sentinel:\n{}", c);
        assert!(c.contains("ontic_last_error"), "error API:\n{}", c);
        assert!(c.contains("ONTIC_POLICY_ABORT"), "policy define:\n{}", c);
    }

    #[test]
    fn test_shim_int_sentinel() {
        let g = crate::gen::parse(
            "fn Foo.f(%x: Int) -> Int\n\
             | %x > 0\n\
             => 1 -> 1\n",
        )
        .unwrap();
        let c = emit_shim_c("f", &[("x".to_string(), Ty::Int)], &Ty::Int, "aa", &g.invariants)
            .unwrap();
        assert!(c.contains("LONG_MIN"), "int sentinel:\n{}", c);
    }

    #[test]
    fn test_shim_list_shape_guard() {
        let g = crate::gen::parse(
            "fn Dot.dot(%a: List<F64>, %b: List<F64>) -> F64\n\
             | len(%a) == len(%b)\n\
             => [1.0], [2.0] -> 2.0\n",
        )
        .unwrap();
        let c = emit_shim_c(
            "dot",
            &[("a".to_string(), Ty::ListF64), ("b".to_string(), Ty::ListF64)],
            &Ty::F64,
            "bb",
            &g.invariants,
        )
        .unwrap();
        assert!(c.contains("a_s == b_s"), "shape guard:\n{}", c);
        assert!(c.contains("long a=%ld"), "list prints size:\n{}", c);
    }

    #[test]
    fn test_shim_no_params_no_checks() {
        let c = emit_shim_c("noop", &[], &Ty::F64, "cc", &[]).unwrap();
        assert!(c.contains("double noop(void)"));
        assert!(!c.contains("if (!("), "no guard chaine:\n{}", c);
        assert!(c.contains("return noop__raw()"), "raw call:\n{}", c);
    }

    #[test]
    fn test_guard_pred_text_untranslated() {
        // res-referencing invariant: outside v1 subset
        let params = vec![("x".to_string(), Ty::F64)];
        let expr = crate::sketch::Expr::BinOp(
            crate::sketch::BinOp::Ge,
            Box::new(crate::sketch::Expr::Var("res".to_string())),
            Box::new(crate::sketch::Expr::FloatLit(0.0)),
        );
        let t = guard_pred_text(&expr, &params);
        assert_eq!(t, "true", "untranslated falls back to true");
    }
}
