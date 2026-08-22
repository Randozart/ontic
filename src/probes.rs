//! Probe generation (stage S5 oracle input): deterministic random inputs from
//! type domains, plus canonical edge cases, used to test candidate universality.
//! Probe strength is bounded by invariant strength — documented honestly in
//! `ontic check` output.

use crate::rng::Rng;
use crate::sketch::Ty;
use crate::wish::{Value, Wish};

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
        Ty::Bool => vec![Value::Bool(true), Value::Bool(false)],
        Ty::ListInt => vec![
            Value::List(vec![]),
            Value::List(vec![0]),
            Value::List(vec![1]),
            Value::List(vec![-1]),
        ],
    }
}

fn sample(ty: &Ty, rng: &mut Rng) -> Value {
    match ty {
        Ty::Int => Value::Int(rng.range_i64(INT_LO, INT_HI)),
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
/// followed by `count` random rows.
///
/// Edge combination cap keeps multi-param wishes from exploding; selection is
/// round-robin over per-param edge lists so coverage stays spread evenly.
pub fn generate(wish: &Wish, count: usize, seed: u64, edge_budget: usize) -> Vec<Vec<Value>> {
    let mut rows: Vec<Vec<Value>> = Vec::new();
    if wish.params.is_empty() {
        return rows;
    }
    let per_param: Vec<Vec<Value>> = wish.params.iter().map(|(_, t)| edges(t)).collect();
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
        rows.push(row);
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
        rows.push(
            wish.params
                .iter()
                .map(|(_, t)| sample(t, &mut rng))
                .collect(),
        );
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wish;

    fn ledger_wish() -> Wish {
        wish::parse("fn f(%items: List<Int>) -> Int\n  => [1] -> 1\n  => [2] -> 2\n").unwrap()
    }

    #[test]
    fn test_probes_are_deterministic() {
        let w = ledger_wish();
        let a = generate(&w, 64, 0x5EED, 8);
        let b = generate(&w, 64, 0x5EED, 8);
        assert_eq!(a, b);
    }

    #[test]
    fn test_probe_shape_and_bounds() {
        let w = ledger_wish();
        let rows = generate(&w, 256, 7, 8);
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
        let rows = generate(&w, 0, 1, 16);
        assert!(rows.iter().any(|r| r[0] == Value::List(vec![])));
        assert!(rows.iter().any(|r| r[0] == Value::List(vec![0])));
    }
}
