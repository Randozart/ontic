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

/// Probe-plan failure: the declared invariants exclude every value the
/// type domain can produce. Surfaced as a wish error, never a candidate kill.
#[derive(Debug, Clone, PartialEq)]
pub struct Unsatisfiable;

/// Check a probe row against input-side invariant satisfaction. Invariants
/// referencing `res` cannot be evaluated here and are ignored (they remain
/// enforced post-hoc by the sieve with the result bound). Returns false only
/// when some invariant evaluates to Bool(false) on the inputs alone.
fn inputs_satisfy(gen: &Gen, row: &[Value], ctx: &Ctx) -> bool {
    let env: Env = gen
        .params
        .iter()
        .zip(row.iter())
        .map(|((n, _), v)| (n.clone(), v.clone()))
        .collect();
    for inv in &gen.invariants {
        match interp::eval_ctx(inv, &env, ctx) {
            Ok(Value::Bool(false)) => return false,
            _ => {}
        }
    }
    true
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
        Ty::Int => vec![Value::Int(0), Value::Int(1), Value::Int(-1)],
        Ty::F64 => vec![
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
        Ty::ListF64 => vec![
            Value::FloatList(vec![]),
            Value::FloatList(vec![1.5]),
            Value::FloatList(vec![-2.5]),
        ],
    }
}

fn sample(ty: &Ty, rng: &mut Rng) -> Value {
    match ty {
        Ty::Int => Value::Int(rng.range_i64(INT_LO, INT_HI)),
        // Deterministic float sampling: integer grid scaled, no denormals.
        Ty::F64 => Value::Float(rng.range_i64(INT_LO * 8, INT_HI * 8) as f64 / 8.0),
        Ty::ListF64 => {
            let len = rng.below(LIST_LEN_MAX);
            let items = (0..len)
                .map(|_| rng.range_i64(ELEM_LO * 8, ELEM_HI * 8) as f64 / 8.0)
                .collect();
            Value::FloatList(items)
        }
        Ty::Bool => Value::Bool(rng.next_u64() % 2 == 0),
        Ty::ListInt => {
            let len = rng.below(LIST_LEN_MAX);
            let items = (0..len)
                .map(|_| rng.range_i64(ELEM_LO, ELEM_HI))
                .collect();
            Value::List(items)
        }
    }
}

/// Generate a deterministic probe plan: up to `edge_budget` edge combinations
/// followed by `count` random rows. Every row satisfies the gen's input-side
/// invariants (Golden Rule 4: probe domain = type domains ∩ invariants).
/// Edge rows outside the declared domain are skipped; random rows are
/// rejection-sampled. Errors when the contract excludes the whole domain.
pub fn generate(
    gen: &Gen,
    count: usize,
    seed: u64,
    edge_budget: usize,
    ctx: &Ctx,
) -> Result<Vec<Vec<Value>>, Unsatisfiable> {
    let mut rows: Vec<Vec<Value>> = Vec::new();
    if gen.params.is_empty() {
        return Ok(rows);
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
        if inputs_satisfy(gen, &row, ctx) {
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
    for _ in 0..count {
        let mut accepted = None;
        for _ in 0..SAMPLE_ATTEMPTS {
            let row: Vec<Value> = gen
                .params
                .iter()
                .map(|(_, t)| sample(t, &mut rng))
                .collect();
            if inputs_satisfy(gen, &row, ctx) {
                accepted = Some(row);
                break;
            }
        }
        match accepted {
            Some(row) => rows.push(row),
            None => return Err(Unsatisfiable),
        }
    }
    Ok(rows)
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
        let a = generate(&w, 64, 0x5EED, 8, &ctx).unwrap();
        let b = generate(&w, 64, 0x5EED, 8, &ctx).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn test_probe_shape_and_bounds() {
        let w = ledger_wish();
        let ctx = interp::Ctx::checked();
        let rows = generate(&w, 256, 7, 8, &ctx).unwrap();
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
        let rows = generate(&w, 0, 1, 16, &ctx).unwrap();
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
        let rows = generate(&w, 0, 1, 16, &ctx).unwrap();
        assert!(!rows.is_empty());
        for row in &rows {
            match &row[0] {
                Value::List(vs) => assert!(!vs.is_empty()),
                other => panic!("bad probe value {}", other),
            }
        }
    }

    #[test]
    fn test_unsatisfiable_invariant_errors_not_kills() {
        // A contract no Int can satisfy must surface as Unsatisfiable.
        let w = gen::parse(
            "fn f(%x: Int) -> Int\n  | %x > 100000\n  | %x < -100000\n  => 0 -> 0\n",
        )
        .unwrap();
        let ctx = interp::Ctx::checked();
        let out = generate(&w, 4, 7, 2, &ctx);
        assert_eq!(out, Err(Unsatisfiable));
    }
}
