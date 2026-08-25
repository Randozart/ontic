//! Deterministic constraint solver for relational probe invariants (S5).
//!
//! Rejection sampling fails when invariants couple parameters relationally
//! (`len(%m) == %n * %n`); the random phase then degrades to EdgesOnly. This
//! module proposes satisfying assignments for the INTEGER SKELETON of a row
//! — scalar Int/Bool values and list lengths — over a conservative subset:
//!
//! - Int literals, Bool literals
//! - scalar Int params, Bool params, `len()` of list params
//! - Add/Sub/Mul (Mul: constant-scaled or `x*x` square of one unknown)
//! - unary minus; comparisons Eq/Ne/Lt/Le/Gt/Ge; And/Or chains
//!
//! Anything else → `Outcome::Unsupported` and the caller keeps the existing
//! honest fallback. THE WALL: the solver only PROPOSES rows; every emitted
//! row is still verified through the interpreter oracle (`first_violation`)
//! before joining the plan. Solving is pure and deterministic — no RNG, all
//! iteration in sorted order.

use crate::gen::Gen;
use crate::lower::{conjuncts, expr_refs_res};
use crate::probes::{INT_HI, INT_LO, LIST_LEN_MAX};
use crate::sketch::{BinOp, Builtin, Expr, Ty};

/// Upper bound on Or-branch expansion before declaring Unsupported.
const MAX_ALTERNATIVES: usize = 8;
/// DFS node budget per alternative constraint set.
const NODE_BUDGET: usize = 500_000;
/// Max candidate values tried for one unknown with a wide domain.
const WIDE_GRID: i64 = 24;
/// Domain width below which every value in range is enumerated.
const DENSE_SPAN: i64 = 16;

/// What the solver concluded about a gen's input-side integer skeleton.
#[derive(Debug, Clone)]
pub enum Outcome {
    /// Invariants fall outside the supported subset.
    Unsupported,
    /// Supported but no assignment satisfies them within probe domains.
    NoSolution,
    /// Satisfying assignments, edge-value-first order.
    Solved(Vec<Solution>),
}

/// One satisfying integer skeleton: scalars by name, list lengths by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Solution {
    pub scalars: Vec<(String, i64)>,
    pub lengths: Vec<(String, usize)>,
}

/// Polynomial over unknowns: sum(coeff * x) + square_coeff * v² + c.
#[derive(Debug, Clone, Default)]
struct Poly {
    terms: Vec<(usize, i64)>,
    square: Option<(usize, i64)>,
    c: i64,
}

impl Poly {
    fn constant(v: i64) -> Self {
        Poly { terms: Vec::new(), square: None, c: v }
    }

    fn single(id: usize, coeff: i64) -> Self {
        Poly { terms: vec![(id, coeff)], square: None, c: 0 }
    }

    /// self + sign * other, merging like terms.
    fn combine(&self, other: &Poly, sign: i64) -> Option<Poly> {
        // At most one square term per polynomial: two distinct squares
        // would make a quartic — outside the supported subset.
        let square = match (&self.square, &other.square) {
            (Some((v, a)), Some((w, b))) if v == w => {
                Some((*v, a.checked_add(sign.checked_mul(*b)?)?))
            }
            (Some(_), Some(_)) => return None,
            (None, Some((v, b))) => Some((*v, sign.checked_mul(*b)?)),
            (s, None) => *s,
        };
        let mut terms = self.terms.clone();
        for (id, k) in &other.terms {
            let k = k.checked_mul(sign)?;
            if let Some(t) = terms.iter_mut().find(|(i, _)| i == id) {
                t.1 = t.1.checked_add(k)?;
            } else {
                terms.push((*id, k));
            }
        }
        let c = self.c.checked_add(sign.checked_mul(other.c)?)?;
        Some(Poly { terms, square, c })
    }

    fn eval(&self, assign: &[i64]) -> Option<i64> {
        let mut acc = self.c;
        for (id, k) in &self.terms {
            acc = acc.checked_add(assign[*id].checked_mul(*k)?)?;
        }
        if let Some((v, k)) = self.square {
            let x = assign[v];
            acc = acc.checked_add(x.checked_mul(x)?.checked_mul(k)?)?;
        }
        Some(acc)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cmp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Debug, Clone)]
struct Constraint {
    lhs: Poly,
    op: Cmp,
    rhs: Poly,
}

impl Constraint {
    fn holds(&self, assign: &[i64]) -> Option<bool> {
        let l = self.lhs.eval(assign)?;
        let r = self.rhs.eval(assign)?;
        Some(match self.op {
            Cmp::Eq => l == r,
            Cmp::Ne => l != r,
            Cmp::Lt => l < r,
            Cmp::Le => l <= r,
            Cmp::Gt => l > r,
            Cmp::Ge => l >= r,
        })
    }
}

/// Unknown table entry: what an id means.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Flag,
    Length,
    Scalar,
}

struct Table {
    entries: Vec<(String, Kind)>,
}

impl Table {
    fn id(&self, name: &str, kind: &Kind) -> Option<usize> {
        self.entries.iter().position(|(n, k)| n == name && k == kind)
    }
}

/// Build the unknown table from gen params, sorted for determinism.
fn table_of(gen: &Gen) -> Table {
    let mut entries: Vec<(String, Kind)> = Vec::new();
    for (n, t) in &gen.params {
        match t {
            Ty::Int => entries.push((n.clone(), Kind::Scalar)),
            Ty::Bool => entries.push((n.clone(), Kind::Flag)),
            Ty::ListInt | Ty::ListF64 | Ty::ListF32 => {
                entries.push((n.clone(), Kind::Length))
            }
            Ty::Tuple(_) => {}
            Ty::F64 | Ty::F32 => {}
        }
    }
    entries.sort();
    Table { entries }
}

/// Initial domain (lo, hi) per unknown kind.
fn domain(kind: &Kind) -> (i64, i64) {
    match kind {
        Kind::Flag => (0, 1),
        Kind::Length => (0, LIST_LEN_MAX as i64 - 1),
        Kind::Scalar => (INT_LO, INT_HI),
    }
}

/// Deterministic candidate values for an unknown given its current domain.
fn candidates(lo: i64, hi: i64) -> Vec<i64> {
    if hi - lo <= DENSE_SPAN {
        return (lo..=hi).collect();
    }
    // Edge-first zigzag: lo, hi, then mids spiralling outward.
    let mut out = vec![lo, hi];
    let mid = lo.div_euclid(2) + hi.div_euclid(2);
    for d in 0..=(WIDE_GRID / 2) {
        for v in [mid + d, mid - d, mid + d + 1, mid - d - 1] {
            if lo < v && v < hi && !out.contains(&v) {
                out.push(v);
            }
        }
        if out.len() >= WIDE_GRID as usize {
            break;
        }
    }
    out.truncate(WIDE_GRID as usize);
    out
}

/// Extract a polynomial or declare the expression unsupported.
fn extract_poly(e: &Expr, tab: &Table) -> Option<Poly> {
    match e {
        Expr::IntLit(v) => Some(Poly::constant(*v)),
        Expr::BoolLit(b) => Some(Poly::constant(*b as i64)),
        Expr::Var(n) => {
            let id = tab.id(n, &Kind::Scalar).or_else(|| tab.id(n, &Kind::Flag))?;
            Some(Poly::single(id, 1))
        }
        Expr::Builtin(Builtin::Len, inner) => match inner.as_ref() {
            Expr::Var(n) => {
                let id = tab.id(n, &Kind::Length)?;
                Some(Poly::single(id, 1))
            }
            _ => None,
        },
        Expr::BinOp(op, l, r) => match op {
            BinOp::Add => {
                let a = extract_poly(l, tab)?;
                let b = extract_poly(r, tab)?;
                a.combine(&b, 1)
            }
            BinOp::Sub => {
                let a = extract_poly(l, tab)?;
                let b = extract_poly(r, tab)?;
                a.combine(&b, -1)
            }
            BinOp::Mul => {
                // Constant-scaled, or x*x square of one unknown.
                if let Expr::IntLit(k) = l.as_ref() {
                    return extract_poly(r, tab)?.scale(*k);
                }
                if let Expr::IntLit(k) = r.as_ref() {
                    return extract_poly(l, tab)?.scale(*k);
                }
                match (l.as_ref(), r.as_ref()) {
                    (Expr::Var(a), Expr::Var(b)) if a == b => {
                        let id = tab.id(a, &Kind::Scalar)?;
                        Some(Poly {
                            terms: Vec::new(),
                            square: Some((id, 1)),
                            c: 0,
                        })
                    }
                    _ => None,
                }
            }
            _ => None,
        },
        Expr::UnOp(crate::sketch::UnOp::Neg, inner) => {
            let p = extract_poly(inner, tab)?;
            p.scale(-1)
        }
        // Boolean negation of a flag: !t == 1 - t.
        Expr::UnOp(crate::sketch::UnOp::Not, inner) => {
            let p = extract_poly(inner, tab)?;
            Poly::constant(1).combine(&p, -1)
        }
        _ => None,
    }
}

impl Poly {
    fn scale(self, k: i64) -> Option<Poly> {
        if k == 0 {
            return Some(Poly::constant(0));
        }
        let mut p = self;
        for (_, c) in p.terms.iter_mut() {
            *c = c.checked_mul(k)?;
        }
        if let Some((_, s)) = &mut p.square {
            *s = s.checked_mul(k)?;
        }
        p.c = p.c.checked_mul(k)?;
        Some(p)
    }
}

/// Extract constraint alternatives (Or branches expand, And multiplies).
fn extract_alts(e: &Expr, tab: &Table, out: &mut Vec<Vec<Constraint>>) -> Option<()> {
    if out.len() > MAX_ALTERNATIVES {
        return None;
    }
    match e {
        Expr::BinOp(BinOp::And, l, r) => {
            extract_alts(l, tab, out)?;
            extract_alts(r, tab, out)
        }
        Expr::BinOp(BinOp::Or, l, r) => {
            let mut la = out.clone();
            extract_alts(l, tab, &mut la)?;
            extract_alts(r, tab, out)?;
            out.extend(la);
            if out.len() > MAX_ALTERNATIVES {
                return None;
            }
            Some(())
        }
        Expr::BinOp(op @ (BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge), l, r) => {
            let lhs = extract_poly(l, tab)?;
            let rhs = extract_poly(r, tab)?;
            let cmp = match op {
                BinOp::Eq => Cmp::Eq,
                BinOp::Ne => Cmp::Ne,
                BinOp::Lt => Cmp::Lt,
                BinOp::Le => Cmp::Le,
                BinOp::Gt => Cmp::Gt,
                _ => Cmp::Ge,
            };
            if out.is_empty() {
                out.push(Vec::new());
            }
            for alt in out.iter_mut() {
                alt.push(Constraint {
                    lhs: lhs.clone(),
                    op: cmp,
                    rhs: rhs.clone(),
                });
            }
            Some(())
        }
        Expr::Var(n) => {
            // Bare flag used truthily: %t means %t != 0.
            let id = tab.id(n, &Kind::Flag)?;
            if out.is_empty() {
                out.push(Vec::new());
            }
            for alt in out.iter_mut() {
                alt.push(Constraint {
                    lhs: Poly::single(id, 1),
                    op: Cmp::Ne,
                    rhs: Poly::constant(0),
                });
            }
            Some(())
        }
        Expr::UnOp(crate::sketch::UnOp::Not, inner) => match inner.as_ref() {
            // !%t means %t == 0.
            Expr::Var(n) => {
                let id = tab.id(n, &Kind::Flag)?;
                if out.is_empty() {
                    out.push(Vec::new());
                }
                for alt in out.iter_mut() {
                    alt.push(Constraint {
                        lhs: Poly::single(id, 1),
                        op: Cmp::Eq,
                        rhs: Poly::constant(0),
                    });
                }
                return Some(());
            }
            _ => None,
        },
        _ => None,
    }
}

/// Flip a comparison under negation of both sides.
fn flip(op: Cmp) -> Cmp {
    match op {
        Cmp::Lt => Cmp::Gt,
        Cmp::Le => Cmp::Ge,
        Cmp::Gt => Cmp::Lt,
        Cmp::Ge => Cmp::Le,
        other => other,
    }
}

/// ceil(k / m) for m > 0.
fn cdiv(k: i64, m: i64) -> i64 {
    k.div_euclid(m) + i64::from(k.rem_euclid(m) != 0)
}

/// Narrow x's domain given mag*x OP k with mag > 0.
/// Returns None when the constraint is unsatisfiable over any integer.
fn bound(op: Cmp, mag: i64, k: i64) -> Option<(i64, i64)> {
    Some(match op {
        Cmp::Eq => {
            if k.rem_euclid(mag) != 0 {
                return None;
            }
            (k / mag, k / mag)
        }
        Cmp::Ne => (i64::MIN, i64::MAX),
        Cmp::Lt => (i64::MIN, cdiv(k, mag) - 1),
        Cmp::Le => (i64::MIN, k.div_euclid(mag)),
        Cmp::Gt => (k.div_euclid(mag) + 1, i64::MAX),
        Cmp::Ge => (cdiv(k, mag), i64::MAX),
    })
}

/// Solve one alternative set over the table's unknowns.
/// Unary constraints narrow domains first (no-square, single-unknown
/// normalized polys); DFS assigns remaining freedom edge-first and verifies
/// every constraint exactly at each leaf.
fn solve_set(cs: &[Constraint], tab: &Table, want: usize) -> Vec<Vec<i64>> {
    let n = tab.entries.len();
    let mut doms: Vec<(i64, i64)> = (0..n)
        .map(|i| domain(&tab.entries[i].1))
        .collect();

    // Normalized single-unknown view for propagation: A*x + cl OP B*x + cr
    // becomes sign*x OP k with sign = A - B, k = cr - cl.
    // Square-carrying polys are skipped here (verified at leaves).
    let mut changed = true;
    let mut rounds = 0;
    while changed && rounds < 8 {
        changed = false;
        rounds += 1;
        for c in cs {
            if c.lhs.square.is_some() || c.rhs.square.is_some() {
                continue;
            }
            let mut ids: Vec<usize> = Vec::new();
            for (id, _) in c.lhs.terms.iter().chain(c.rhs.terms.iter()) {
                if !ids.contains(id) {
                    ids.push(*id);
                }
            }
            if ids.len() != 1 {
                continue;
            }
            let id = ids[0];
            let a = c.lhs.terms.iter().find(|(i, _)| *i == id).map(|(_, k)| *k).unwrap_or(0);
            let b = c.rhs.terms.iter().find(|(i, _)| *i == id).map(|(_, k)| *k).unwrap_or(0);
            let sign = match a.checked_sub(b) {
                Some(s) if s != 0 => s,
                _ => continue,
            };
            let k = match c.rhs.c.checked_sub(c.lhs.c) {
                Some(v) => v,
                None => continue,
            };
            // Normalize to positive sign: sign*x OP k  =>  |sign|*x OP' ±k.
            let mag = sign.abs();
            let kk = if sign > 0 { k } else { k.checked_neg().unwrap_or(k) };
            let op = if sign > 0 { c.op } else { flip(c.op) };
            let (lo, hi) = doms[id];
            let (nlo, nhi) = match bound(op, mag, kk) {
                Some(b) => b,
                None => return Vec::new(), // definitively unsatisfiable
            };
            let nlo = nlo.max(lo);
            let nhi = nhi.min(hi);
            if nlo > nhi {
                return Vec::new();
            }
            if (nlo, nhi) != (lo, hi) {
                doms[id] = (nlo, nhi);
                changed = true;
            }
        }
    }

    // Deterministic variable order: by domain width then id.
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by_key(|&i| (doms[i].1 - doms[i].0, i));

    let mut solutions: Vec<Vec<i64>> = Vec::new();
    let mut assign = vec![0i64; n];
    let mut nodes = 0usize;

    fn dfs(
        depth: usize,
        order: &[usize],
        doms: &[(i64, i64)],
        assign: &mut [i64],
        cs: &[Constraint],
        solutions: &mut Vec<Vec<i64>>,
        want: usize,
        nodes: &mut usize,
    ) -> bool {
        // Returns true when the budget is exhausted.
        if solutions.len() >= want {
            return true;
        }
        *nodes += 1;
        if *nodes > NODE_BUDGET {
            return true;
        }
        if depth == order.len() {
            if cs.iter().all(|c| c.holds(assign) == Some(true)) {
                solutions.push(assign.to_vec());
            }
            return false;
        }
        let id = order[depth];
        let (lo, hi) = doms[id];
        for v in candidates(lo, hi) {
            assign[id] = v;
            // Partial check: prune on constraints fully inside assigned ids
            // is skipped for simplicity; leaf verification is exact.
            if dfs(depth + 1, order, doms, assign, cs, solutions, want, nodes) {
                return true;
            }
            if solutions.len() >= want {
                return true;
            }
        }
        false
    }

    dfs(0, &order, &doms, &mut assign, cs, &mut solutions, want, &mut nodes);
    solutions
}

/// Solve the gen's input-side integer skeleton.
pub fn solve(gen: &Gen, want: usize) -> Outcome {
    let tab = table_of(gen);
    if tab.entries.is_empty() {
        return Outcome::Unsupported;
    }
    // Input-side conjuncts only: skip res-referencing invariants (the
    // oracle enforces those post-hoc).
    let mut alts: Vec<Vec<Constraint>> = Vec::new();
    for inv in &gen.invariants {
        if expr_refs_res(inv) {
            continue;
        }
        for part in conjuncts(inv) {
            if extract_alts(part, &tab, &mut alts).is_none() {
                return Outcome::Unsupported;
            }
            if alts.len() > MAX_ALTERNATIVES {
                return Outcome::Unsupported;
            }
        }
    }
    if alts.is_empty() {
        return Outcome::Unsupported;
    }
    let mut all: Vec<Solution> = Vec::new();
    for alt in &alts {
        let rows = solve_set(alt, &tab, want);
        for r in rows {
            if all.len() >= want {
                break;
            }
            let mut sol = Solution {
                scalars: Vec::new(),
                lengths: Vec::new(),
            };
            for (i, (name, kind)) in tab.entries.iter().enumerate() {
                match kind {
                    Kind::Scalar => sol.scalars.push((name.clone(), r[i])),
                    Kind::Flag => sol.scalars.push((name.clone(), r[i])),
                    Kind::Length => sol.lengths.push((name.clone(), r[i] as usize)),
                }
            }
            if !all.contains(&sol) {
                all.push(sol);
            }
        }
    }
    if all.is_empty() {
        Outcome::NoSolution
    } else {
        all.truncate(want);
        Outcome::Solved(all)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gen;

    fn solve_gen(text: &str, want: usize) -> Outcome {
        let g = gen::parse(text).expect("gen parses");
        solve(&g, want)
    }

    #[test]
    fn test_square_length_relation_solved() {
        // QR-style: len(%m) == %n * %n with %n >= 0.
        let out = solve_gen(
            "fn Qr.shape(%n: Int, %m: List<F64>) -> F64\n  | %n >= 0\n  | len(%m) == %n * %n\n  => 2, [1.0] -> 1.0 ± 1e-9\n",
            16,
        );
        match out {
            Outcome::Solved(sols) => {
                assert!(!sols.is_empty());
                // n=0 => len 0 must be among solutions (edge-first).
                assert!(
                    sols.iter().any(|s| s.scalars.iter().any(|(n, v)| n == "n" && *v == 0)),
                    "n=0 edge missing: {sols:?}"
                );
                for s in &sols {
                    let n = s.scalars.iter().find(|(k, _)| k == "n").unwrap().1;
                    let lm = s.lengths.iter().find(|(k, _)| k == "m").unwrap().1 as i64;
                    assert_eq!(lm, n * n, "square violated: {s:?}");
                    assert!(n >= 0);
                }
            }
            other => panic!("expected Solved, got {other:?}"),
        }
    }

    #[test]
    fn test_coupled_multi_constraint_solved() {
        // kmeans-lite style: three coupled constraints across two lists.
        let out = solve_gen(
            "fn K.step(%k: Int, %pts: List<F64>, %cent: List<F64>) -> F64\n  | %k >= 1\n  | len(%pts) >= len(%cent)\n  | len(%cent) == %k * 2\n  => 1, [1.0], [2.0, 3.0] -> 1.0 ± 1e-9\n",
            16,
        );
        match out {
            Outcome::Solved(sols) => {
                assert!(!sols.is_empty());
                for s in &sols {
                    let k = s.scalars.iter().find(|(x, _)| x == "k").unwrap().1;
                    let lp = s.lengths.iter().find(|(x, _)| x == "pts").unwrap().1;
                    let lc = s.lengths.iter().find(|(x, _)| x == "cent").unwrap().1;
                    assert!(k >= 1);
                    assert!(lp >= lc);
                    assert_eq!(lc as i64, k * 2);
                }
            }
            other => panic!("expected Solved, got {other:?}"),
        }
    }

    #[test]
    fn test_scalar_equality_pair_solved() {
        let out = solve_gen(
            "fn P.eq(%x: Int, %y: Int) -> Int\n  | %x == %y + 7\n  => 0, 7 -> 1 ± 0\n",
            8,
        );
        match out {
            Outcome::Solved(sols) => {
                assert!(!sols.is_empty());
                for s in &sols {
                    let x = s.scalars.iter().find(|(k, _)| k == "x").unwrap().1;
                    let y = s.scalars.iter().find(|(k, _)| k == "y").unwrap().1;
                    assert_eq!(x, y + 7);
                }
            }
            other => panic!("expected Solved, got {other:?}"),
        }
    }

    #[test]
    fn test_unsupported_builtin_is_not_claimed() {
        // sum() is outside the extraction subset.
        let out = solve_gen(
            "fn U.s(%xs: List<Int>) -> Int\n  | sum(%xs) >= len(%xs)\n  => [] -> 0 ± 0\n",
            4,
        );
        assert!(matches!(out, Outcome::Unsupported), "{out:?}");
    }

    #[test]
    fn test_unsat_linear_system_reports_no_solution() {
        // len(%a) > 5 && len(%a) < 3 cannot hold (lengths <= 7).
        let out = solve_gen(
            "fn C.bad(%a: List<Int>) -> Int\n  | len(%a) > 5\n  | len(%a) < 3\n  => [9] -> 9 ± 0\n",
            4,
        );
        assert!(matches!(out, Outcome::NoSolution), "{out:?}");
    }

    #[test]
    fn test_determinism_same_seed_free() {
        let text = "fn Qr.shape(%n: Int, %m: List<F64>) -> F64\n  | %n >= 0\n  | len(%m) == %n * %n\n  => 2, [1.0] -> 1.0 ± 1e-9\n";
        let a = solve_gen(text, 12);
        let b = solve_gen(text, 12);
        assert_eq!(format!("{a:?}"), format!("{b:?}"), "solver must be deterministic");
    }

    #[test]
    fn test_bool_flag_constrains_length() {
        let out = solve_gen(
            "fn B.f(%t: Bool, %xs: List<Int>) -> Int\n  | (%t && len(%xs) > 0) || (!%t && len(%xs) == 0)\n  => true, [4] -> 1 ± 0\n",
            8,
        );
        match out {
            Outcome::Solved(sols) => {
                assert!(!sols.is_empty(), "flag/length coupling unsolvable");
            }
            other => panic!("expected Solved, got {other:?}"),
        }
    }
}
