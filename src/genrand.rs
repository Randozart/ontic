//! Type-directed random candidate generation — the uniform-sampler baseline.
//!
//! Purpose (ablation control): quantify whether the transformer earns its
//! slot versus grammar-and-type-guided enumeration. Generates WELL-TYPED,
//! grammar-valid sketches uniformly from the space our constraints allow —
//! the fair fight. Raw token noise would flatter the LLM.
//!
//! Deterministic under seed. Biased to reference every parameter at least
//! once so candidates are semantically plausible inputs to the sieve.

use crate::sketch::{BinOp, Expr, Ty};
use crate::gen::Gen;
use crate::rng::Rng;

/// One generated candidate: source text plus its parameter env echo.
pub struct Generated {
    pub text: String,
}

struct Ctx {
    rng: Rng,
    depth_budget: usize,
    vars: Vec<(String, Ty)>,
    var_seq: usize,
}

impl Ctx {
    fn fresh(&mut self, prefix: &str) -> String {
        self.var_seq += 1;
        format!("{}{}", prefix, self.var_seq)
    }

    fn vars_of_type(&self, ty: &Ty) -> Vec<String> {
        self.vars
            .iter()
            .filter(|(_, t)| t == ty)
            .map(|(n, _)| n.clone())
            .collect()
    }

    /// Emit a variable reference if one exists; else a typed literal.
    fn var_or_lit(&mut self, ty: &Ty) -> Expr {
        let vs = self.vars_of_type(ty);
        if !vs.is_empty() && self.rng.below(10) < 6 {
            let n = &vs[self.rng.below(vs.len())];
            return Expr::Var(n.clone());
        }
        lit_of(ty)
    }
}

fn lit_of(ty: &Ty) -> Expr {
    match ty {
        Ty::Int => Expr::IntLit(1),
        Ty::F64 => Expr::FloatLit(2.0),
        Ty::Bool => Expr::BoolLit(true),
        Ty::ListInt => Expr::ListLit(vec![1, 2]),
        Ty::ListF64 => Expr::ListLit(vec![1, 2]), // int lists widen in broadcasts
    }
}

/// Generate one well-typed expression of the requested type.
fn gen_expr(ty: &Ty, ctx: &mut Ctx) -> Expr {
    if ctx.depth_budget == 0 {
        return ctx.var_or_lit(ty);
    }
    // Weighted shape choice per target type.
    match ty {
        Ty::Bool => {
            ctx.depth_budget -= 1;
            let a = gen_expr(&Ty::Int, ctx);
            let b = gen_expr(&Ty::Int, ctx);
            let op = match ctx.rng.below(4) {
                0 => BinOp::Lt,
                1 => BinOp::Le,
                2 => BinOp::Gt,
                _ => BinOp::Ge,
            };
            ctx.depth_budget += 1;
            Expr::BinOp(op, Box::new(a), Box::new(b))
        }
        Ty::Int | Ty::F64 => {
            ctx.depth_budget -= 1;
            let roll = ctx.rng.below(10);
            if roll < 4 {
                // arithmetic over same-kind operands
                let a = gen_expr(ty, ctx);
                let b = gen_expr(ty, ctx);
                let op = match ctx.rng.below(4) {
                    0 => BinOp::Add,
                    1 => BinOp::Sub,
                    2 => BinOp::Mul,
                    _ => BinOp::Div,
                };
                let e = Expr::BinOp(op, Box::new(a), Box::new(b));
                ctx.depth_budget += 1;
                e
            } else if roll < 6 {
                // reduction builtin over an available list
                let lt = if matches!(ty, Ty::F64) { Ty::ListF64 } else { Ty::ListInt };
                let l = gen_expr(&lt, ctx);
                let b = if matches!(ty, Ty::F64) && matches!(lt, Ty::ListInt) {
                    // widen: multiply by 1.0 keeps F64 typing after promotion
                    Some(Expr::BinOp(BinOp::Mul, Box::new(l.clone()), Box::new(Expr::FloatLit(1.0))))
                } else {
                    None
                };
                let src = b.unwrap_or(l);
                let bi = match ctx.rng.below(3) {
                    0 => crate::sketch::Builtin::Sum,
                    1 => crate::sketch::Builtin::Max,
                    _ => crate::sketch::Builtin::Min,
                };
                ctx.depth_budget += 1;
                Expr::Builtin(bi, Box::new(src))
            } else {
                let e = ctx.var_or_lit(ty);
                ctx.depth_budget += 1;
                e
            }
        }
        Ty::ListInt | Ty::ListF64 => {
            // Broadcast arithmetic: list op scalar / scalar op list / zip
            ctx.depth_budget -= 1;
            let elem_f = matches!(ty, Ty::ListF64);
            let l_ty = if elem_f { Ty::ListF64 } else { Ty::ListInt };
            let s_ty = if elem_f { Ty::F64 } else { Ty::Int };
            let lhs = gen_expr(&l_ty, ctx);
            let rhs = if ctx.rng.below(2) == 0 {
                gen_expr(&l_ty, ctx)
            } else {
                gen_expr(&s_ty, ctx)
            };
            let op = match ctx.rng.below(2) {
                0 => BinOp::Add,
                _ => BinOp::Mul,
            };
            ctx.depth_budget += 1;
            Expr::BinOp(op, Box::new(lhs), Box::new(rhs))
        }
    }
}

/// Render an Expr back to sketch surface syntax (compact printer).
pub fn render(e: &Expr) -> String {
    use Expr::*;
    match e {
        IntLit(v) => v.to_string(),
        FloatLit(v) => format!("{:.1}", v),
        BoolLit(b) => b.to_string(),
        Var(n) => format!("%{}", n),
        FloatListLit(items) => {
            let inner: Vec<String> = items.iter().map(|v| v.to_string()).collect();
            format!("[{}]", inner.join(", "))
        }
        ListLit(items) => {
            let inner: Vec<String> = items.iter().map(|v| v.to_string()).collect();
            format!("[{}]", inner.join(", "))
        }
        Builtin(b, i) => {
            let name = match b {
                crate::sketch::Builtin::Len => "len",
                crate::sketch::Builtin::Range => "range",
                crate::sketch::Builtin::Sum => "sum",
                crate::sketch::Builtin::Max => "max",
                crate::sketch::Builtin::Min => "min",
                crate::sketch::Builtin::MinEl => "min_el",
                crate::sketch::Builtin::MaxEl => "max_el",
                crate::sketch::Builtin::Sqrt => "sqrt",
                crate::sketch::Builtin::Exp => "exp",
                crate::sketch::Builtin::Log => "log",
                crate::sketch::Builtin::Abs => "abs",
                crate::sketch::Builtin::Index => unreachable_index_name(),
            };
            format!("{}({})", name, render(i))
        }
        Builtin2(b, l, r) => {
            assert!(matches!(b, crate::sketch::Builtin::Index));
            format!("index({}, {})", render(l), render(r))
        }
        UnOp(crate::sketch::UnOp::Neg, i) => format!("(-{})", render(i)),
        UnOp(crate::sketch::UnOp::Not, i) => format!("!{}", render(i)),
        If(c, t, f) => format!(
            "if {} {{ {} }} else {{ {} }}",
            render(c), render(t), render(f)
        ),
        Let(n, v, b) => format!("let %{} = {}; {}", n, render(v), render(b)),
        Fold { var, acc, list, init, body, until, aux } => format!(
            "fold %{} in {}, %{} from {}{} {{ {} }}{}",
            var,
            render(list),
            acc,
            render(init),
            aux.iter()
                .map(|(n, e)| format!(", %{} from {}", n, render(e)))
                .collect::<String>(),
            render(body),
            match until {
                Some(u) => format!(" until {}", render(u)),
                None => String::new(),
            }
        ),
        Tuple(items) => {
            let inner: Vec<String> = items.iter().map(render).collect();
            format!("({})", inner.join(", "))
        }
        ListCons(elems) => {
            let inner: Vec<String> = elems.iter().map(render).collect();
            format!("[{}]", inner.join(", "))
        }
        Map { var, list, body } => format!(
            "map(%{} in {}) {{ {} }}",
            var, render(list), render(body)
        ),
        Call(p, args) => {
            let as_: Vec<String> = args.iter().map(render).collect();
            format!("{}({})", p, as_.join(", "))
        }
        BinOp(op, l, r) => format!(
            "({} {} {})",
            render(l),
            crate::lower::binop_display(*op),
            render(r)
        ),
    }
}

/// Generate `count` well-typed candidate sources for a gen's signature.
pub fn generate(g: &Gen, count: usize, seed: u64) -> Vec<Generated> {
    let mut out = Vec::new();
    for i in 0..count {
        let mut ctx = Ctx {
            rng: crate::rng::Rng::new(seed.wrapping_add(i as u64).wrapping_mul(0x9E37)),
            depth_budget: 3 + (i % 3),
            vars: g.params.clone(),
            var_seq: 0,
        };
        // Body: chain 1-3 lets ending in the return expression.
        let mut lets: Vec<String> = Vec::new();
        let extra = ctx.rng.below(3);
        for j in 0..extra {
            let t = if g.ret == Ty::F64 && j == extra - 1 {
                g.ret.clone()
            } else {
                g.ret.clone()
            };
            let e = gen_expr(&t, &mut ctx);
            let name = ctx.fresh("t");
            lets.push(format!("let %{} = {}; {}", name, render(&e), ""));
            let _ = name;
        }
        let final_e = gen_expr(&g.ret, &mut ctx);
        let body = if lets.is_empty() {
            render(&final_e)
        } else {
            let joined: String = lets.iter().map(|l| l.replace("  ", "")).collect::<Vec<_>>().join(" ");
            format!("{} {}", joined.trim_end(), render(&final_e))
        };
        let params: Vec<String> = g
            .params
            .iter()
            .map(|(n, t)| format!("%{}: {}", n, t.name()))
            .collect();
        out.push(Generated {
            text: format!(
                "fn @gen{}({}) -> {} {{ {} }}",
                i,
                params.join(", "),
                g.ret.name(),
                body
            ),
        });
    }
    out
}

fn unreachable_index_name() -> ! {
    panic!("internal: Index is binary")
}
