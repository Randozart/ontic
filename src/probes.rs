//! Probe generation (stage S5 oracle input): deterministic random inputs from
//! type domains, plus canonical edge cases, used to test candidate universality.
//! Probe strength is bounded by invariant strength — documented honestly in
//! `ontic check` output.

use crate::interp::{self, Ctx, Env};
use crate::rng::Rng;
use crate::sketch::Ty;
use crate::gen::{Value, Gen};
use std::collections::HashMap;

/// Rejection-sampling attempts per random row before declaring the gen's
/// invariants unsatisfiable over the probe domain.
const SAMPLE_ATTEMPTS: usize = 256;

/// Probe-plan failure: not even the canonical edge combinations satisfy the
/// declared invariants, so the plan would be empty. Surfaced as a wish error,
/// never a candidate kill.
#[derive(Debug, Clone, PartialEq)]
pub struct Unsatisfiable;

/// How much of the requested random coverage the plan actually achieved.
/// Relational invariants (e.g. `len(%a) == %n * %n`) can make independent
/// random sampling a lottery; degraded plans keep edge rows and say so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanQuality {
    /// All requested random rows sampled.
    Full,
    /// Random phase exhausted its budget without satisfying the contract;
    /// plan carries edge rows only.
    EdgesOnly,
}

/// A probe plan plus honest accounting of what the sampler could achieve.
#[derive(Debug, Clone)]
pub struct ProbePlan {
    pub rows: Vec<Vec<Value>>,
    pub quality: PlanQuality,
    /// Total random-phase attempts drawn (accepted + rejected).
    pub attempts: usize,
    /// Rejections attributed to the FIRST violated invariant per attempt,
    /// as `(invariant display, count)`, sorted by count descending.
    pub rejects: Vec<(String, usize)>,
}

impl ProbePlan {
    /// The invariant that rejected the most random attempts, if any.
    pub fn top_rejector(&self) -> Option<&(String, usize)> {
        self.rejects.first()
    }
}

/// First input-side invariant violated by a row, by index. Invariants
/// referencing `res` cannot be evaluated here and are skipped (they remain
/// enforced post-hoc by the sieve with the result bound).
fn first_violation(gen: &Gen, row: &[Value], ctx: &Ctx) -> Option<usize> {
    let env: Env = gen
        .params
        .iter()
        .zip(row.iter())
        .map(|((n, _), v)| (n.clone(), v.clone()))
        .collect();
    for (i, inv) in gen.invariants.iter().enumerate() {
        match interp::eval_ctx(inv, &env, ctx) {
            Ok(Value::Bool(false)) => return Some(i),
            _ => {}
        }
    }
    None
}

/// Probe-domain bounds for v0 integer values.
pub const INT_LO: i64 = -1000;
pub const INT_HI: i64 = 1000;
/// Probe list length upper bound (exclusive).
pub const LIST_LEN_MAX: usize = 8;
/// Element bound inside probe lists.
pub const ELEM_LO: i64 = -100;
pub const ELEM_HI: i64 = 100;

/// Canonical edge cases prepended before random rows (deterministic).
fn edges(ty: &Ty) -> Vec<Value> {
    match ty {
        Ty::Str => vec![
            Value::Str(String::new()),
            Value::Str("a".to_string()),
            Value::Str("hello".to_string()),
        ],
        Ty::Int => vec![Value::Int(0), Value::Int(1), Value::Int(-1)],
        Ty::F64 | Ty::F32 => vec![
            Value::Float(0.0),
            Value::Float(1.5),
            Value::Float(-2.5),
            Value::Float(1e9),
        ],
        Ty::Bool => vec![Value::Bool(true), Value::Bool(false)],
        Ty::ListInt => vec![
            Value::List(vec![]),
            Value::List(vec![0]),
            Value::List(vec![1]),
            Value::List(vec![-1]),
        ],
        Ty::ListF64 | Ty::ListF32 => vec![
            Value::FloatList(vec![]),
            Value::FloatList(vec![1.5]),
            Value::FloatList(vec![-2.5]),
        ],
        // Tuple params are rejected by the checker before probing.
        Ty::Tuple(_) => vec![],
    }
}

fn sample(ty: &Ty, rng: &mut Rng) -> Value {
    match ty {
        Ty::Int => Value::Int(rng.range_i64(INT_LO, INT_HI)),
        // Deterministic float sampling: integer grid scaled, no denormals.
        Ty::F64 | Ty::F32 => Value::Float(rng.range_i64(INT_LO * 8, INT_HI * 8) as f64 / 8.0),
        Ty::ListF64 | Ty::ListF32 => {
            let len = rng.below(LIST_LEN_MAX);
            let items = (0..len)
                .map(|_| rng.range_i64(ELEM_LO * 8, ELEM_HI * 8) as f64 / 8.0)
                .collect();
            Value::FloatList(items)
        }
        Ty::Tuple(_) => Value::Int(0),
        Ty::Bool => Value::Bool(rng.next_u64() % 2 == 0),
        Ty::ListInt => {
            let len = rng.below(LIST_LEN_MAX);
            let items = (0..len)
                .map(|_| rng.range_i64(ELEM_LO, ELEM_HI))
                .collect();
            Value::List(items)
        }
        Ty::Str => {
            let len = (rng.next_u64() % 8) as usize;
            let s: String = (0..len).map(|i| char::from(b'a' + i as u8)).collect();
            Value::Str(s)
        }
    }
}

/// Materialize a full probe row from a solved integer skeleton: scalars and
/// lengths come from the solution; float scalars and list bodies are sampled
/// as usual. The row is accepted only when the interpreter oracle finds no
/// violated invariant — the solver proposes, the oracle disposes.
fn materialize(
    gen: &Gen,
    sol: &crate::probes_solver::Solution,
    rng: &mut Rng,
    ctx: &Ctx,
) -> Option<Vec<Value>> {
    let scalar = |n: &str| sol.scalars.iter().find(|(s, _)| s == n).map(|(_, v)| *v);
    let length = |n: &str| sol.lengths.iter().find(|(s, _)| s == n).map(|(_, v)| *v);
    let mut row: Vec<Value> = Vec::new();
    for (n, t) in &gen.params {
        match t {
            Ty::Int => row.push(Value::Int(scalar(n)?)),
            Ty::Bool => row.push(Value::Bool(scalar(n)? != 0)),
            Ty::F64 | Ty::F32 => row.push(sample(t, rng)),
            Ty::Tuple(_) => {},
            Ty::ListInt => {
                let len = length(n)?;
                let items = (0..len).map(|_| rng.range_i64(ELEM_LO, ELEM_HI)).collect();
                row.push(Value::List(items));
            }
            Ty::ListF64 | Ty::ListF32 => {
                let len = length(n)?;
                let items = (0..len)
                    .map(|_| rng.range_i64(ELEM_LO * 8, ELEM_HI * 8) as f64 / 8.0)
                    .collect();
                row.push(Value::FloatList(items));
            }
        Ty::Str => {
            let len = (rng.next_u64() % 8) as usize;
            let s: String = (0..len).map(|i| char::from(b'a' + i as u8)).collect();
            row.push(Value::Str(s));
        }
        }
    }
    if first_violation(gen, &row, ctx).is_some() {
        None
    } else {
        Some(row)
    }
}

/// Generate a deterministic probe plan: up to `edge_budget` edge combinations
/// followed by `count` random rows. Every row satisfies the gen's input-side
/// invariants (Golden Rule 4: probe domain = type domains ∩ invariants).
/// Edge rows outside the declared domain are skipped; random rows are
/// rejection-sampled. When relational invariants out-select the sampler the
/// plan degrades to edge rows and reports EdgesOnly; an empty plan is an
/// Unsatisfiable contract. Rejection attribution names the guilty invariant.
pub fn generate(
    gen: &Gen,
    count: usize,
    seed: u64,
    edge_budget: usize,
    ctx: &Ctx,
) -> Result<ProbePlan, Unsatisfiable> {
    let mut rows: Vec<Vec<Value>> = Vec::new();
    if gen.params.is_empty() {
        return Ok(ProbePlan {
            rows,
            quality: PlanQuality::Full,
            attempts: 0,
            rejects: Vec::new(),
        });
    }
    let per_param: Vec<Vec<Value>> = gen.params.iter().map(|(_, t)| edges(t)).collect();
    let mut cursor = vec![0usize; per_param.len()];
    for _ in 0..edge_budget {
        let row: Vec<Value> = per_param
            .iter()
            .zip(cursor.iter())
            .map(|(opts, i)| {
                let k = (*i).min(opts.len() - 1);
                opts[k].clone()
            })
            .collect();
        if first_violation(gen, &row, ctx).is_none() {
            rows.push(row);
        }
        // Advance the first param that still has unseen edges; wrap others.
        let mut advanced = false;
        for i in 0..cursor.len() {
            if cursor[i] + 1 < per_param[i].len() {
                cursor[i] += 1;
                advanced = true;
                break;
            }
            cursor[i] = 0;
        }
        if !advanced {
            break;
        }
    }
    let mut rng = Rng::new(seed);
    let mut quality = PlanQuality::Full;
    let mut attempts = 0usize;
    let mut reject_counts: HashMap<usize, usize> = HashMap::new();
    let edge_rows = rows.len();
    'random: for _ in 0..count {
        for _ in 0..SAMPLE_ATTEMPTS {
            attempts += 1;
            let row: Vec<Value> = gen
                .params
                .iter()
                .map(|(_, t)| sample(t, &mut rng))
                .collect();
            match first_violation(gen, &row, ctx) {
                None => {
                    rows.push(row);
                    continue 'random;
                }
                Some(idx) => {
                    *reject_counts.entry(idx).or_insert(0) += 1;
                }
            }
        }
        // Budget exhausted without a satisfying row. Consult the constraint
        // solver for a satisfying integer skeleton before degrading.
        // Skeletons may be reused with fresh element fills: rows stay
        // distinct where the type domain allows variation.
        let have = rows.len() - edge_rows;
        let want = count - have;
        match crate::probes_solver::solve(gen, want.max(4)) {
            crate::probes_solver::Outcome::Solved(sols) if !sols.is_empty() => {
                let mut added = 0usize;
                let mut tries = 0usize;
                while added < want && tries < want.saturating_mul(SAMPLE_ATTEMPTS) {
                    let s = &sols[tries % sols.len()];
                    tries += 1;
                    attempts += 1;
                    if let Some(row) = materialize(gen, s, &mut rng, ctx) {
                        rows.push(row);
                        added += 1;
                    }
                }
                if added >= want {
                    // Full coverage recovered through solving.
                    break;
                }
                quality = PlanQuality::EdgesOnly;
            }
            _ => quality = PlanQuality::EdgesOnly,
        }
        break;
    }
    if rows.is_empty() {
        return Err(Unsatisfiable);
    }
    // Attribution report: display text per invariant index, count desc.
    let mut rejects: Vec<(String, usize)> = reject_counts
        .into_iter()
        .map(|(idx, n)| (crate::lower::expr_display(&gen.invariants[idx]), n))
        .collect();
    rejects.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    Ok(ProbePlan {
        rows,
        quality,
        attempts,
        rejects,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gen;

    fn ledger_wish() -> Gen {
        gen::parse("fn f(%items: List<Int>) -> Int\n  => [1] -> 1\n  => [2] -> 2\n").unwrap()
    }

    #[test]
    fn test_probes_are_deterministic() {
        let w = ledger_wish();
        let ctx = interp::Ctx::checked();
        let a = generate(&w, 64, 0x5EED, 8, &ctx).unwrap().rows;
        let b = generate(&w, 64, 0x5EED, 8, &ctx).unwrap().rows;
        assert_eq!(a, b);
    }

    #[test]
    fn test_probe_shape_and_bounds() {
        let w = ledger_wish();
        let ctx = interp::Ctx::checked();
        let plan = generate(&w, 256, 7, 8, &ctx).unwrap();
        assert_eq!(plan.quality, PlanQuality::Full);
        let rows = &plan.rows;
        assert_eq!(rows.len(), 256 + 4); // edges for List<Int> + randoms
        for row in &rows[4..] {
            assert_eq!(row.len(), 1);
            match &row[0] {
                Value::List(vs) => {
                    assert!(vs.len() < LIST_LEN_MAX);
                    for v in vs {
                        assert!((ELEM_LO..=ELEM_HI).contains(v));
                    }
                }
                other => panic!("bad probe value {}", other),
            }
        }
    }

    #[test]
    fn test_edges_include_empty_and_zero() {
        let w = ledger_wish();
        let ctx = interp::Ctx::checked();
        let rows = generate(&w, 0, 1, 16, &ctx).unwrap().rows;
        assert!(rows.iter().any(|r| r[0] == Value::List(vec![])));
        assert!(rows.iter().any(|r| r[0] == Value::List(vec![0])));
    }

    #[test]
    fn test_invariants_filter_edge_rows() {
        // len(%xs) > 0 must exclude the empty-list edge from the plan.
        let w = gen::parse(
            "fn f(%xs: List<Int>) -> Int\n  | len(%xs) > 0\n  => [1] -> 1\n",
        )
        .unwrap();
        let ctx = interp::Ctx::checked();
        let rows = generate(&w, 0, 1, 16, &ctx).unwrap().rows;
        assert!(!rows.is_empty());
        for row in &rows {
            match &row[0] {
                Value::List(vs) => assert!(!vs.is_empty()),
                other => panic!("bad probe value {}", other),
            }
        }
    }

    #[test]
    fn test_empty_plan_is_unsatisfiable() {
        // A contract no Int can satisfy must surface as Unsatisfiable.
        let w = gen::parse(
            "fn f(%x: Int) -> Int\n  | %x > 100000\n  | %x < -100000\n  => 0 -> 0\n",
        )
        .unwrap();
        let ctx = interp::Ctx::checked();
        let out = generate(&w, 4, 7, 2, &ctx);
        assert!(matches!(out, Err(Unsatisfiable)));
    }

    
#[test]
    fn test_relational_contract_recovered_by_solver() {
        // matmul-style shape relation: independent random sampling is a
        // lottery; the constraint solver recovers full coverage instead of
        // degrading to edge rows.
        let w = gen::parse(
            "fn mm(%a: List<F64>, %b: List<F64>, %n: Int) -> F64\n  | %n > 0\n  | len(%a) == %n * %n\n  | len(%b) == %n * %n\n  => [1.0], [2.0], 1 -> 2.0\n",
        )
        .unwrap();
        let ctx = interp::Ctx::checked();
        let plan = generate(&w, 32, 7, 64, &ctx).unwrap();
        assert_eq!(plan.quality, PlanQuality::Full, "{:?}", plan.rows);
        // Every row honours the relational contract.
        for r in &plan.rows {
            let n = match &r[2] {
                Value::Int(v) => *v,
                other => panic!("bad row {other:?}"),
            };
            let la = match &r[0] {
                Value::FloatList(vs) => vs.len(),
                other => panic!("bad row {other:?}"),
            };
            let lb = match &r[1] {
                Value::FloatList(vs) => vs.len(),
                other => panic!("bad row {other:?}"),
            };
            assert_eq!(la as i64, n * n);
            assert_eq!(lb as i64, n * n);
        }
    }
}
