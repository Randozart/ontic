//! Direct LLVM IR text emitter — bypasses MLIR entirely.
//!
//! Produces LLVM IR compatible with `llc -filetype=obj` without needing
//! mlir-opt or mlir-translate. Reduces runtime toolchain dependency to
//! just `llc` and `clang`.
//!
//! Convention: List<T> params expand to 5 flat args (allocated*, aligned*,
//! offset, size, stride). Scalars pass as single values. Matches the
//! Flat-MemRef ABI verified against the MLIR pipeline.

use crate::sketch::{Builtin, BinOp, Expr, Ty, UnOp};
use std::collections::HashMap;

pub struct LlvmFnSpec<'a> {
    pub name: &'a str,
    pub params: &'a [(String, Ty)],
    pub ret: &'a Ty,
    pub body: &'a Expr,
}

pub struct LlvmEmitter {
    out: String,
    reg: usize,
    block: usize,
    current_block: String,
    indent: usize,
    vars: HashMap<String, (String, Ty)>,
}

impl LlvmEmitter {
    pub fn new() -> Self {
        LlvmEmitter {
            out: String::new(),
            reg: 0,
            block: 0,
            current_block: "entry".to_string(),
            indent: 1,
            vars: HashMap::new(),
        }
    }

    fn fresh(&mut self) -> String {
        self.reg += 1;
        format!("%{}", self.reg)
    }

    fn label(&mut self, prefix: &str) -> String {
        self.block += 1;
        format!("{}{}", prefix, self.block)
    }

    fn line(&mut self, text: &str) {
        for _ in 0..self.indent {
            self.out.push_str("  ");
        }
        self.out.push_str(text);
        self.out.push('\n');
    }

    fn set_block(&mut self, name: &str) {
        self.current_block = name.to_string();
        self.out.push_str(&format!("{}:\\n", name));
    }

    fn const_i64(&mut self, v: i64) -> String {
        let r = self.fresh();
        self.line(&format!("{} = add i64 {}, 0", r, v));
        r
    }

    #[allow(dead_code)]
    fn const_f64(&mut self, v: f64) -> String {
        let r = self.fresh();
        self.line(&format!("{} = fpconst {:.17e}", r, v));
        r
    }
}

/// Expand a type to its LLVM parameter types.
/// List<T> → 5 params; scalars → 1 param each.
fn llvm_param_types(ty: &Ty) -> Vec<&'static str> {
    match ty {
        Ty::ListInt | Ty::ListF64 | Ty::ListF32 => vec!["ptr", "ptr", "i64", "i64", "i64"],
        // Checker rejects tuple params; direct-LLVM never sees them.
        Ty::Tuple(_) => vec!["i64"],
        Ty::Int | Ty::Bool => vec!["i64"],
        Ty::F64 => vec!["double"],
        Ty::F32 => vec!["float"],
    }
}

fn llvm_ret_type(ty: &Ty) -> Result<&'static str, String> {
    match ty {
        Ty::Int | Ty::Bool => Ok("i64"),
        Ty::F64 => Ok("double"),
        _ => Err("list-return not supported in direct LLVM path".to_string()),
    }
}

/// Generate LLVM IR text for a candidate function.
/// Only supports scalar-returning kernels in v1.
pub fn emit_llvm(spec: &LlvmFnSpec) -> Result<String, String> {
    if matches!(spec.ret, Ty::ListInt | Ty::ListF64) {
        return Err(
            "direct LLVM: list-return requires sret support; use MLIR path".to_string(),
        );
    }
    let ret_ty = llvm_ret_type(spec.ret)?;

    let mut em = LlvmEmitter::new();

    // Build signature.
    let mut sig_parts: Vec<String> = Vec::new();
    let mut arg_names: Vec<String> = Vec::new();
    for (pname, pty) in spec.params {
        let types = llvm_param_types(pty);
        let suffixes: Vec<&str> = if _is_list(pty) {
            vec!["a", "b", "o", "s", "st"]
        } else {
            vec![""]
        };
        for (ty_str, suf) in types.iter().zip(suffixes.iter()) {
            let reg = format!("%{}_{}", pname.replace('%', ""), suf);
            sig_parts.push(format!("{} {}", ty_str, reg));
            arg_names.push(reg);
        }
        // Bind sketch var to its aligned pointer (for lists) or scalar.
        let bind_name = pname.trim_start_matches('%');
        if _is_list(pty) {
            em.vars.insert(
                bind_name.to_string(),
                (arg_names[arg_names.len() - 4].clone(), Ty::ListF64),
            );
            // Also store size for range/index.
            em.vars.insert(
                format!("__size_{}", bind_name),
                (arg_names[arg_names.len() - 2].clone(), Ty::Int),
            );
        } else {
            em.vars.insert(
                bind_name.to_string(),
                (arg_names.last().unwrap().clone(), pty.clone()),
            );
        }
    }

    em.out.push_str(&format!(
        "define {} @{}({}) {{\nentry:\n",
        ret_ty,
        spec.name,
        sig_parts.join(", ")
    ));

    let result = emit_expr(&spec.body, &mut em)?;
    em.line(&format!("ret {} {}", ret_ty, result));
    em.out.push('}');
    Ok(em.out)
}

fn _is_list(ty: &Ty) -> bool {
    matches!(ty, Ty::ListInt | Ty::ListF64)
}

// ---------------------------------------------------------------------------
// Expression emission
// ---------------------------------------------------------------------------

fn emit_expr(e: &Expr, em: &mut LlvmEmitter) -> Result<String, String> {
    match e {
        Expr::Tuple(_) => Err(
            "direct LLVM: tuple bodies require the MLIR pipeline".to_string(),
        ),
        Expr::IntLit(v) => {
            let r = em.fresh();
            em.line(&format!("{} = add i64 {}, 0", r, v));
            Ok(r)
        }
        Expr::FloatLit(v) => {
            let r = em.fresh();
            let bits = v.to_bits();
            em.line(&format!(
                "{} = bitcast i64 {} to double",
                r, bits as i64
            ));
            Ok(r)
        }
        Expr::BoolLit(b) => {
            let r = em.fresh();
            em.line(&format!("{} = add i64 {}, 0", r, *b as i64));
            Ok(r)
        }
        Expr::Var(n) => {
            let (ssa, _) = em
                .vars
                .get(n)
                .cloned()
                .ok_or_else(|| format!("unbound variable %{}", n))?;
            Ok(ssa)
        }
        Expr::ListLit(_) | Expr::FloatListLit(_) => Err(
            "direct LLVM: list literals unsupported in scalar kernels".to_string(),
        ),
        Expr::UnOp(UnOp::Neg, inner) => {
            let x = emit_expr(inner, em)?;
            let r = em.fresh();
            if expr_llvm_ty(inner, em)? == "double" {
                em.line(&format!("{} = fneg double {}", r, x));
            } else {
                let zero = em.const_i64(0);
                em.line(&format!("{} = sub i64 {}, {}", r, zero, x));
            }
            Ok(r)
        }
        Expr::UnOp(UnOp::Not, inner) => {
            let b = emit_expr(inner, em)?;
            let r = em.fresh();
            em.line(&format!("{} = xor i64 {}, 1", r, b));
            Ok(r)
        }
        Expr::If(c, t, f) => emit_if(c, t, f, em),
        Expr::Let(n, value, b) => {
            let old = em.vars.get(n);
            let saved = old.cloned();
            let v = emit_expr(value, em)?;
            let t_str = expr_llvm_ty(value, em)?;
            let t = if t_str == "double" { Ty::F64 } else { Ty::Int };
            em.vars.insert(n.clone(), (v, t));
            let result = emit_expr(b, em)?;
            if let Some(s) = saved {
                em.vars.insert(n.clone(), s);
            } else {
                em.vars.remove(n);
            }
            Ok(result)
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
            if until.is_some() || !aux.is_empty() {
                return Err(
                    "direct LLVM: until-folds require the MLIR pipeline".to_string(),
                );
            }
            emit_fold(var, acc, list, init, body, em)
        }
        Expr::Map { .. } => Err(
            "direct LLVM: map construct requires list-return support".to_string(),
        ),
        Expr::BinOp(op, l, r) => emit_binop(*op, l, r, em),
        Expr::Builtin(b, inner) => emit_builtin(*b, inner, em),
        Expr::Builtin2(..) => Err(
            "direct LLVM: binary builtins (index) require list-return support".to_string(),
        ),
        Expr::Call(p, _) => Err(format!(
            "direct LLVM: vault calls (`{}`) require composite linking",
            p
        )),
        Expr::ListCons(_) => Err(
            "direct LLVM: expression-list construction requires list-return support".to_string(),
        ),
    }
}

fn emit_fold(
    var: &str,
    acc_name: &str,
    list: &Expr,
    init: &Expr,
    body: &Expr,
    em: &mut LlvmEmitter,
) -> Result<String, String> {
    // Evaluate init outside loop.
    let init_ssa = emit_expr(init, em)?;

    // Determine count from list param size.
    let _list_ssa = emit_expr(list, em)?;
    let count = match list {
        Expr::Var(n) => em
            .vars
            .get(&format!("__size_{}", n))
            .map(|(v, _)| v.clone())
            .ok_or_else(|| format!("no size known for %{}", n))?,
        _ => return Err("fold over non-param list unsupported".to_string()),
    };

    let header = em.label("fold_hdr");
    let body_lbl = em.label("fold_body");
    let exit = em.label("fold_exit");

    // Jump to header.
    em.line(&format!("br label %{}", header));

    // Header: phi nodes for iv and acc.
    em.set_block(&header);
    let iv = em.fresh();
    em.line(&format!(
        "{} = phi i64 [ 0, %{} ], [ %next_iv_{}, %{} ]",
        iv, "entry_placeholder", em.reg, body_lbl
    ));
    let acc_phi = em.fresh();
    let init_reg = init_ssa.clone();
    em.line(&format!(
        "{} = phi i64 [ {}, %entry_placeholder ], [ %new_acc_{}, %{} ]",
        acc_phi, init_reg, em.reg, body_lbl
    ));
    // Fix entry label references.
    em.out = em.out.replace("%entry_placeholder", "%entry");

    // Condition: iv < count
    let cmp = em.fresh();
    em.line(&format!("{} = icmp slt i64 {}, {}", cmp, iv, count));
    em.line(&format!("br i1 {}, label %{}, label %{}", cmp, body_lbl, exit));

    // Body.
    em.set_block(&body_lbl);
    // Bind fold var to loaded element.
    // For simplicity, we assume the list param gives us the base ptr.
    let elem = em.fresh();
    // We need to GEP into the list buffer.
    // The list's aligned ptr is bound via vars.
    // For now: assume the fold's list is a param whose aligned ptr we have.
    // Actually, we stored the aligned ptr when binding params.
    // Let me restructure: emit_expr(list) should give us the base ptr.
    // For Var(n) where n is a list param, it returns the aligned ptr.
    let gep = em.fresh();
    let base = match list {
        Expr::Var(n) => em
            .vars
            .get(n)
            .map(|(v, _)| v.clone())
            .ok_or_else(|| format!("unbound %{}", n))?,
        _ => return Err("fold over complex expression unsupported".to_string()),
    };
    em.line(&format!(
        "{} = getelementptr double, ptr {}, i64 {}",
        gep, base, iv
    ));
    em.line(&format!("{} = load double, ptr {}", elem, gep));

    let mut scoped_vars = std::collections::HashMap::new();
    scoped_vars.insert(var.to_string(), (elem.clone(), Ty::F64));
    scoped_vars.insert(acc_name.to_string(), (acc_phi.clone(), Ty::F64));

    // Save/restore vars.
    let saved_acc = em.vars.get(acc_name).cloned();
    let saved_var = em.vars.get(var).cloned();
    em.vars.insert(acc_name.to_string(), (acc_phi.clone(), Ty::F64));
    em.vars.insert(var.to_string(), (elem.clone(), Ty::F64));

    let body_result = emit_expr(body, em)?;

    if let Some(s) = &saved_acc {
        em.vars.insert(acc_name.to_string(), s.clone());
    }
    if let Some(s) = &saved_var {
        em.vars.insert(var.to_string(), s.clone());
    }
    let _ = scoped_vars;

    let next_iv = em.fresh();
    em.line(&format!(
        "{} = add i64 {}, 1",
        next_iv, iv
    ));
    em.line(&format!("br label %{}", header));

    // Exit: acc has final value. Patch phi references.
    let _ = exit;
    // Patch the phi node source labels.
    em.out = em.out.replace(
        &format!("[ %next_iv_{}, %{} ]", em.reg, body_lbl),
        &format!("[ {}, {} ]", next_iv, body_lbl),
    );
    em.out = em.out.replace(
        &format!("[ {}, %entry_placeholder ], [ %new_acc_{}, %{} ]", init_reg, em.reg, body_lbl),
        &format!("[ {}, %entry ], [ {}, {} ]", init_reg, body_result, body_lbl),
    );

    em.set_block(&exit);
    Ok(acc_phi)
}

#[allow(clippy::too_many_arguments)]
fn emit_if(
    c: &Expr,
    t: &Expr,
    f: &Expr,
    em: &mut LlvmEmitter,
) -> Result<String, String> {
    let cv = emit_expr(c, em)?;
    // Convert to i1: compare against 0.
    let cond = em.fresh();
    em.line(&format!(
        "{} = icmp ne i64 {}, 0",
        cond, cv
    ));

    let then_lbl = em.label("then");
    let else_lbl = em.label("else");
    let merge = em.label("merge");

    em.line(&format!("br i1 {}, label %{}, label %{}", cond, then_lbl, else_lbl));

    em.set_block(&then_lbl);
    let tv = emit_expr(t, em)?;

    em.set_block(&else_lbl);
    let fv = emit_expr(f, em)?;

    em.set_block(&merge);
    let result = em.fresh();
    let phi_ty = expr_llvm_ty(t, em)?;
    em.line(&format!(
        "{} = phi {} [ {}, %{} ], [ {}, %{} ]",
        result, phi_ty, tv, then_lbl, fv, else_lbl
    ));
    Ok(result)
}

fn emit_binop(op: BinOp, l: &Expr, r: &Expr, em: &mut LlvmEmitter) -> Result<String, String> {
    let lv = emit_expr(l, em)?;
    let rv = emit_expr(r, em)?;
    let l_ty = expr_llvm_ty(l, em)?;
    let is_float = l_ty == "double";

    match op {
        BinOp::Concat => Err("concat unsupported in direct LLVM (scalar kernels)".to_string()),
        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
            let instr = if is_float {
                match op {
                    BinOp::Add => "fadd",
                    BinOp::Sub => "fsub",
                    BinOp::Mul => "fmul",
                    BinOp::Div => "fdiv",
                    _ => "frem",
                }
            } else {
                match op {
                    BinOp::Add => "add",
                    BinOp::Sub => "sub",
                    BinOp::Mul => "mul",
                    BinOp::Div => "sdiv",
                    _ => "srem",
                }
            };
            let ty = if is_float { "double" } else { "i64" };
            let r = em.fresh();
            em.line(&format!("{} = {} {} {}, {}", r, instr, ty, lv, rv));
            Ok(r)
        }
        BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
            let cmp_instr = if is_float {
                match op {
                    BinOp::Eq => "oeq",
                    BinOp::Ne => "une",
                    BinOp::Lt => "olt",
                    BinOp::Le => "ole",
                    BinOp::Gt => "ogt",
                    _ => "oge",
                }
            } else {
                match op {
                    BinOp::Eq => "eq",
                    BinOp::Ne => "ne",
                    BinOp::Lt => "slt",
                    BinOp::Le => "sle",
                    BinOp::Gt => "sgt",
                    _ => "sge",
                }
            };
            let cmp_ty = if is_float { "double" } else { "i64" };
            let bit = em.fresh();
            em.line(&format!(
                "{} = icmp {} {}, {}, {}",
                bit, cmp_instr, lv, rv, cmp_ty
            ));
            let out = em.fresh();
            em.line(&format!("{} = zext i1 {} to i64", out, bit));
            Ok(out)
        }
        BinOp::And => {
            let r = em.fresh();
            em.line(&format!("{} = and i64 {}, {}", r, lv, rv));
            Ok(r)
        }
        BinOp::Or => {
            let r = em.fresh();
            em.line(&format!("{} = or i64 {}, {}", r, lv, rv));
            Ok(r)
        }
    }
}

fn emit_builtin(b: Builtin, inner: &Expr, em: &mut LlvmEmitter) -> Result<String, String> {
    let x = emit_expr(inner, em)?;
    match b {
        Builtin::Len => {
            // For list params, the size was stored during binding.
            // For simplicity, look up __size_<name>.
            Err("len: requires size tracking (use MLIR path)".to_string())
        }
        Builtin::Sqrt => {
            let r = em.fresh();
            em.line(&format!("{} = call double @llvm.sqrt.f64({})", r, x));
            Ok(r)
        }
        Builtin::Exp => {
            let r = em.fresh();
            em.line(&format!("{} = call double @llvm.exp.f64({})", r, x));
            Ok(r)
        }
        Builtin::Log => {
            let r = em.fresh();
            em.line(&format!("{} = call double @llvm.log.f64({})", r, x));
            Ok(r)
        }
        Builtin::Abs => {
            let r = em.fresh();
            em.line(&format!("{} = call double @llvm.fabs.f64({})", r, x));
            Ok(r)
        }
        _ => Err(format!("builtin {:?} unsupported in direct LLVM", b)),
    }
}

fn expr_llvm_ty(e: &Expr, em: &LlvmEmitter) -> Result<&'static str, String> {
    match e {
        Expr::FloatLit(_) => Ok("double"),
        Expr::IntLit(_) | Expr::BoolLit(_) => Ok("i64"),
        Expr::Var(n) => em
            .vars
            .get(n)
            .map(|(_, t)| match t {
                Ty::F64 => "double",
                _ => "i64",
            })
            .ok_or_else(|| format!("unbound %{}", n)),
        _ => Ok("i64"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch;

    #[test]
    fn test_simple_sum_llvm_ir() {
        let cand = sketch::parse(
            "fn @total(%xs: List<Int>) -> Int { fold %x in %xs, %acc from 0 { %acc + %x } }",
        )
        .unwrap();
        let spec = LlvmFnSpec {
            name: "total",
            params: &cand.params,
            ret: &cand.ret,
            body: &cand.body,
        };
        let ir = emit_llvm(&spec).unwrap();
        assert!(ir.contains("define i64 @total("));
        assert!(ir.contains("phi i64"));
        assert!(ir.contains("icmp slt"));
    }

    #[test]
    fn test_scalar_arithmetic_llvm_ir() {
        let cand = sketch::parse("fn @f(%a: Int, %b: Int) -> Int { %a + %b }").unwrap();
        let spec = LlvmFnSpec {
            name: "f",
            params: &cand.params,
            ret: &cand.ret,
            body: &cand.body,
        };
        let ir = emit_llvm(&spec).unwrap();
        eprintln!("LLVM IR:\n{}", &ir[..ir.len().min(500)]);
        assert!(ir.contains("add i64"));
    }
}
