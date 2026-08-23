//! Stage S6: static overfit shape scanner.
//!
//! Behavioral stages (S4/S5) catch wrong generalizations; this catches
//! *memorization structure* that can pass finite evidence: branch trees whose
//! guards compare inputs against literals lifted from the visible examples,
//! and output-table shapes (nested ifs returning distinct literal leaves with
//! no real computation).

use crate::sketch::{BinOp, Candidate, Expr};
use crate::wish::Example;
use std::collections::HashSet;

/// Scanner thresholds. One struct, one place — never scattered literals.
#[derive(Debug, Clone)]
pub struct OverfitConfig {
    /// Reject when > this fraction of comparisons guard on example literals.
    pub max_guard_ratio: f64,
    /// Ratio only applies when at least this many comparisons exist.
    pub min_guards_for_ratio: usize,
    /// Distinct literal leaves in an if-tree with no fold/list computation
    /// trigger table-shape rejection once at least this many appear.
    pub table_leaf_min: usize,
}

impl Default for OverfitConfig {
    fn default() -> Self {
        OverfitConfig {
            max_guard_ratio: 0.5,
            min_guards_for_ratio: 2,
            table_leaf_min: 3,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum OverfitVerdict {
    Clean,
    Suspicious(String),
}

struct Stats {
    comparisons: usize,
    example_guarded: usize,
    int_literals: HashSet<i64>,
    has_fold: bool,
    has_len: bool,
}

impl Stats {
    fn new() -> Self {
        Stats {
            comparisons: 0,
            example_guarded: 0,
            int_literals: HashSet::new(),
            has_fold: false,
            has_len: false,
        }
    }
}

/// Collect every integer literal visible to the forge: example inputs
/// (list elements included) and outputs. These are the "leaked" constants a
/// memorizing candidate would hardcode.
fn example_literals(transparent: &[Example]) -> HashSet<i64> {
    let mut out = HashSet::new();
    for ex in transparent {
        for v in ex.inputs.iter().chain(std::iter::once(&ex.output)) {
            match v {
                crate::wish::Value::Int(i) => {
                    out.insert(*i);
                }
                crate::wish::Value::List(vs) => {
                    out.extend(vs.iter().copied());
                }
                _ => {}
            }
        }
        // The output itself is the juiciest leak; already covered above.
    }
    out
}

fn walk(e: &Expr, leaked: &HashSet<i64>, st: &mut Stats) {
    match e {
        Expr::IntLit(v) => {
            st.int_literals.insert(*v);
        }
        // Float literals are not example-leak candidates in v0 (evidence is
        // int-typed); they are honest computation constants.
        Expr::FloatLit(_) => {}
        Expr::BoolLit(_) | Expr::Var(_) => {}
        Expr::ListLit(items) => {
            st.int_literals.extend(items.iter().copied());
        }
        // Sum/max/min are real computation; numeric transforms too.
        Expr::Builtin(crate::sketch::Builtin::Sum, inner)
        | Expr::Builtin(crate::sketch::Builtin::Max, inner)
        | Expr::Builtin(crate::sketch::Builtin::Min, inner) => {
            st.has_len = true;
            walk(inner, leaked, st);
        }
        Expr::Builtin(_, inner) => {
            walk(inner, leaked, st);
        }
        Expr::UnOp(_, inner) => walk(inner, leaked, st),
        Expr::If(c, t, f) => {
            walk(c, leaked, st);
            walk(t, leaked, st);
            walk(f, leaked, st);
        }
        Expr::Let(_, v, b) => {
            walk(v, leaked, st);
            walk(b, leaked, st);
        }
        Expr::Fold { list, init, body, .. } => {
            st.has_fold = true;
            walk(list, leaked, st);
            walk(init, leaked, st);
            walk(body, leaked, st);
        }
        Expr::BinOp(op, l, r) => {
            if matches!(
                op,
                BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge
            ) {
                st.comparisons += 1;
                if guard_uses_example_literal(l, r, leaked) {
                    st.example_guarded += 1;
                }
            }
            walk(l, leaked, st);
            walk(r, leaked, st);
        }
    }
}

/// True when one comparison side is an int literal present in the leaked set
/// and the other side mentions a variable (i.e., input-derived).
fn guard_uses_example_literal(
    l: &Expr,
    r: &Expr,
    leaked: &HashSet<i64>,
) -> bool {
    let lit_of = |e: &Expr| match e {
        Expr::IntLit(v) if leaked.contains(v) => Some(*v),
        _ => None,
    };
    match (lit_of(l), lit_of(r)) {
        (Some(_), None) => mentions_var(r),
        (None, Some(_)) => mentions_var(l),
        _ => false,
    }
}

fn mentions_var(e: &Expr) -> bool {
    match e {
        Expr::Var(_) => true,
        Expr::IntLit(_) | Expr::FloatLit(_) | Expr::BoolLit(_) | Expr::ListLit(_) => false,
        Expr::UnOp(_, i) => mentions_var(i),
        Expr::Builtin(_, i) => mentions_var(i),
        Expr::If(c, t, f) => mentions_var(c) || mentions_var(t) || mentions_var(f),
        Expr::Let(_, t, f) => mentions_var(t) || mentions_var(f),
        Expr::BinOp(_, l, r) => mentions_var(l) || mentions_var(r),
        Expr::Fold { list, init, body, .. } => {
            mentions_var(list) || mentions_var(init) || mentions_var(body)
        }
    }
}

/// Count distinct literal leaves reachable as results of nested if branches
/// (the shape of an output lookup table).
fn count_if_leaves(e: &Expr, leaves: &mut HashSet<i64>) {
    match e {
        Expr::IntLit(v) => {
            leaves.insert(*v);
        }
        Expr::If(_, t, f) => {
            count_if_leaves(t, leaves);
            count_if_leaves(f, leaves);
        }
        Expr::BinOp(op, l, r)
            if matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod) =>
        {
            // Arithmetic over literal leaves still counts as table-shaped when
            // both sides are leaf trees.
            count_if_leaves(l, leaves);
            count_if_leaves(r, leaves);
        }
        _ => {}
    }
}

/// Run S6 against a candidate using only transparent-visible literals.
pub fn scan(cand: &Candidate, transparent: &[Example], cfg: &OverfitConfig) -> OverfitVerdict {
    let leaked = example_literals(transparent);
    let mut st = Stats::new();
    walk(&cand.body, &leaked, &mut st);

    if st.comparisons >= cfg.min_guards_for_ratio {
        let ratio = st.example_guarded as f64 / st.comparisons as f64;
        if ratio > cfg.max_guard_ratio {
            return OverfitVerdict::Suspicious(format!(
                "guard-ratio {:.2} > {:.2} ({} of {} comparisons use example literals)",
                ratio, cfg.max_guard_ratio, st.example_guarded, st.comparisons
            ));
        }
    }

    if !st.has_fold && !st.has_len {
        let mut leaves = HashSet::new();
        count_if_leaves(&cand.body, &mut leaves);
        if leaves.len() >= cfg.table_leaf_min {
            return OverfitVerdict::Suspicious(format!(
                "table-shape: {} distinct literal leaves, no fold/len computation",
                leaves.len()
            ));
        }
    }

    OverfitVerdict::Clean
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{sketch, wish};

    fn examples() -> Vec<Example> {
        vec![
            wish::Example {
                inputs: vec![wish::Value::List(vec![1, 2, 3])],
                output: wish::Value::Int(6),
             tol: 0.0,
            },
            wish::Example {
                inputs: vec![wish::Value::List(vec![4, 5])],
                output: wish::Value::Int(9),
             tol: 0.0,
            },
        ]
    }

    #[test]
    fn test_honest_fold_is_clean() {
        let c = sketch::parse(
            "fn @t(%items: List<Int>) -> Int { fold %x in %items, %acc from 0 { %acc + %x } }",
        )
        .unwrap();
        assert_eq!(scan(&c, &examples(), &OverfitConfig::default()), OverfitVerdict::Clean);
    }

    #[test]
    fn test_lookup_table_rejected() {
        let c = sketch::parse(
            "fn @t(%items: List<Int>) -> Int { if len(%items) == 3 { 6 } else { if len(%items) == 2 { 9 } else { 0 } } }",
        )
        .unwrap();
        let v = scan(&c, &examples(), &OverfitConfig::default());
        assert!(matches!(v, OverfitVerdict::Suspicious(_)), "got {:?}", v);
    }

    #[test]
    fn test_guard_on_example_literal_flagged() {
        let c = sketch::parse(
            "fn @t(%n: Int) -> Int { if %n == 1 { 2 } else { if %n == 4 { 9 } else { 0 + %n } } }",
        )
        .unwrap();
        let exs = vec![
            Example { inputs: vec![wish::Value::Int(1)], output: wish::Value::Int(2), tol: 0.0 },
            Example { inputs: vec![wish::Value::Int(4)], output: wish::Value::Int(9), tol: 0.0 },
        ];
        let v = scan(&c, &exs, &OverfitConfig::default());
        assert!(matches!(v, OverfitVerdict::Suspicious(_)), "got {:?}", v);
    }

    #[test]
    fn test_legitimate_constant_compare_passes() {
        // Comparing against non-example constant is honest branching.
        let c = sketch::parse("fn @t(%n: Int) -> Int { if %n > 100 { 100 } else { %n * 2 } }").unwrap();
        let exs = vec![
            Example { inputs: vec![wish::Value::Int(3)], output: wish::Value::Int(6), tol: 0.0 },
            Example { inputs: vec![wish::Value::Int(7)], output: wish::Value::Int(14), tol: 0.0 },
        ];
        assert_eq!(scan(&c, &exs, &OverfitConfig::default()), OverfitVerdict::Clean);
    }
}
