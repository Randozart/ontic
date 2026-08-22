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

use crate::sketch::{BinOp, Expr, Ty};

/// Pretty-print an expression back to sketch surface syntax.
/// Used by canonical wish serialization and sieve diagnostics.
pub fn expr_display(e: &Expr) -> String {
    match e {
        Expr::IntLit(v) => v.to_string(),
        Expr::BoolLit(b) => b.to_string(),
        Expr::Var(n) => format!("%{}", n),
        Expr::ListLit(items) => {
            let inner: Vec<String> = items.iter().map(|v| v.to_string()).collect();
            format!("[{}]", inner.join(", "))
        }
        Expr::Len(i) => format!("len({})", expr_display(i)),
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
}

impl Emitter {
    fn new() -> Self {
        Emitter {
            out: String::new(),
            counter: 0,
            indent: 0,
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
        Ty::ListInt => "memref<?xi64>",
    }
}

fn mlir_ret_type(ty: &Ty) -> Result<&'static str, String> {
    match ty {
        Ty::Int | Ty::Bool => Ok("i64"),
        // v0 functions return scalars; list-returning wishes are a planned M2 extension.
        Ty::ListInt => Err("v0 lowering: list-returning functions not supported".to_string()),
    }
}

/// Emit a complete `module { func.func @name ... }` for a candidate.
pub fn emit_fn(
    name: &str,
    params: &[(String, Ty)],
    ret: &Ty,
    body: &Expr,
) -> Result<String, String> {
    let out_ty = mlir_ret_type(ret)?;
    let mut em = Emitter::new();

    let sig: Vec<String> = params
        .iter()
        .map(|(n, t)| format!("%{}: {}", n, mlir_param_type(t)))
        .collect();

    em.line("module {");
    em.indent += 1;
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

    let result = emit_expr(body, &mut env, &mut em)?;
    em.line(&format!("return {} : i64", result));
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
fn emit_expr(e: &Expr, env: &mut Vec<Binding>, em: &mut Emitter) -> Result<String, String> {
    match e {
        Expr::IntLit(v) => Ok(em.const_i64(*v)),
        Expr::BoolLit(b) => Ok(em.const_i64(if *b { 1 } else { 0 })),
        Expr::Var(n) => Ok(lookup(env, n)?.ssa.clone()),
        Expr::ListLit(items) => emit_list_lit(items, em),
        Expr::Len(inner) => {
            let m = emit_expr(inner, env, em)?;
            let idx0 = em.const_index(0);
            let dim = em.fresh("dim");
            let mty = "memref<?xi64>";
            // Generic op syntax: Ubuntu LLVM 18.1.3's mlir-opt rejects the
            // custom memref.dim assembly ("expected operation name in
            // quotes") regardless of shape; the generic form parses cleanly
            // everywhere. See ISSUES.md 2026-08-22.
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
        Expr::UnOp(crate::sketch::UnOp::Neg, inner) => {
            let x = emit_expr(inner, env, em)?;
            let z = em.const_i64(0);
            let r = em.fresh("neg");
            em.line(&format!("{} = arith.subi {}, {} : i64", r, z, x));
            Ok(r)
        }
        Expr::UnOp(crate::sketch::UnOp::Not, inner) => {
            let b = emit_expr(inner, env, em)?;
            let one = em.const_i64(1);
            let r = em.fresh("not");
            em.line(&format!("{} = arith.xori {}, {} : i64", r, b, one));
            Ok(r)
        }
        Expr::If(c, t, f) => emit_if(c, t, f, env, em),
        Expr::Let(n, value, body) => {
            let v = emit_expr(value, env, em)?;
            env.push(Binding {
                name: n.clone(),
                ssa: v,
            });
            emit_expr(body, env, em)
        }
        Expr::Fold {
            var,
            acc,
            list,
            init,
            body,
        } => emit_fold(var, acc, list, init, body, env, em),
        Expr::BinOp(op, l, r) => emit_binop(*op, l, r, env, em),
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

fn emit_if(
    c: &Expr,
    t: &Expr,
    f: &Expr,
    env: &mut Vec<Binding>,
    em: &mut Emitter,
) -> Result<String, String> {
    let cv = emit_expr(c, env, em)?;
    let cond = em.fresh("cond");
    em.line(&format!(
        "{} = arith.trunci {} : i64 to i1",
        cond, cv
    ));
    let result = em.fresh("ifres");
    em.line(&format!("{} = scf.if {} -> (i64) {{", result, cond));
    em.indent += 1;
    let tv = emit_expr(t, env, em)?;
    em.line(&format!("scf.yield {} : i64", tv));
    em.indent -= 1;
    em.line("} else {");
    em.indent += 1;
    let fv = emit_expr(f, env, em)?;
    em.line(&format!("scf.yield {} : i64", fv));
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
    em: &mut Emitter,
) -> Result<String, String> {
    let init_v = emit_expr(init, env, em)?;
    let m = emit_expr(list, env, em)?;
    let idx0 = em.const_index(0);
    let step = em.const_index(1);
    let dim = em.fresh("dim");
    let mty = "memref<?xi64>";
    // Generic op syntax — see Len arm note re Ubuntu mlir-opt memref.dim.
    em.line(&format!(
        "{} = \"memref.dim\"({}, {}) : ({}, index) -> index",
        dim, m, idx0, mty
    ));

    let iv = em.fresh("i");
    let acc_ssa = em.fresh("acc");
    em.line(&format!(
        "{} = scf.for {} = {} to {} step {} iter_args({} = {}) -> (i64) {{",
        acc_ssa, iv, idx0, dim, step, acc_ssa, init_v
    ));
    em.indent += 1;

    let elem = em.fresh("x");
    em.line(&format!("{} = memref.load {}[{}] : {}", elem, m, iv, mty));

    env.push(Binding {
        name: var.to_string(),
        ssa: elem.clone(),
    });
    env.push(Binding {
        name: acc.to_string(),
        ssa: acc_ssa.clone(),
    });
    let body_v = emit_expr(body, env, em)?;
    em.line(&format!("scf.yield {} : i64", body_v));
    em.indent -= 1;
    em.line("}");
    Ok(acc_ssa)
}

fn emit_binop(
    op: BinOp,
    l: &Expr,
    r: &Expr,
    env: &mut Vec<Binding>,
    em: &mut Emitter,
) -> Result<String, String> {
    let lv = emit_expr(l, env, em)?;
    let rv = emit_expr(r, env, em)?;
    if is_comparison(op) {
        return emit_cmp(op, &lv, &rv, em);
    }
    let out = em.fresh("op");
    let stmt = match op {
        BinOp::Add => Some(format!("{} = arith.addi {}, {} : i64", out, lv, rv)),
        BinOp::Sub => Some(format!("{} = arith.subi {}, {} : i64", out, lv, rv)),
        BinOp::Mul => Some(format!("{} = arith.muli {}, {} : i64", out, lv, rv)),
        BinOp::Div => Some(format!("{} = arith.divsi {}, {} : i64", out, lv, rv)),
        BinOp::Mod => Some(format!("{} = arith.remsi {}, {} : i64", out, lv, rv)),
        BinOp::And => Some(format!("{} = arith.andi {}, {} : i64", out, lv, rv)),
        BinOp::Or => Some(format!("{} = arith.ori {}, {} : i64", out, lv, rv)),
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

fn is_comparison(op: BinOp) -> bool {
    matches!(
        op,
        BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge
    )
}

/// Comparisons produce i1 then widen to the i64 boolean ABI.
fn emit_cmp(op: BinOp, lv: &str, rv: &str, em: &mut Emitter) -> Result<String, String> {
    let pred = match op {
        BinOp::Eq => "eq",
        BinOp::Ne => "ne",
        BinOp::Lt => "slt",
        BinOp::Le => "sle",
        BinOp::Gt => "sgt",
        _ => "sge",
    };
    let bit = em.fresh("cmp");
    em.line(&format!(
        "{} = arith.cmpi {}, {}, {} : i64",
        bit, pred, lv, rv
    ));
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
        emit_fn(&c.name, &c.params, &c.ret, &c.body).expect("lowers")
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
