//! Gen parsing: `.ont` specification files.
//!
//! ```text
//! fn Ledger.total(%items: List<Int>) -> Int
//!   | %res >= 0
//!   => [1,2,3] -> 6
//!   ?? [7,8,9] -> 24
//! ```
//!
//! Invariants and examples share the sketch expression language; `%res`
//! refers to the result, `%name` to parameters. Opaque policy: explicit `??`
//! wins; otherwise auto-hide floor(50%) of transparent examples when >= 4.

use crate::sketch::{self, Expr, Ty};
use std::fmt;

/// A concrete value used in examples and probes.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Int(i64),
    Float(f64),
    Bool(bool),
    List(Vec<i64>),
    FloatList(Vec<f64>),
    /// Opaque string.
    Str(String),
    /// Multi-component example output for tuple-returning gens.
    Tuple(Vec<Value>),
}

impl Value {
    /// The gen-level type of this value.
    pub fn ty(&self) -> Ty {
        match self {
            Value::Int(_) => Ty::Int,
            Value::Float(_) => Ty::F64,
            Value::Bool(_) => Ty::Bool,
            Value::List(_) => Ty::ListInt,
            Value::FloatList(_) => Ty::ListF64,
            Value::Str(_) => Ty::Str,
            Value::Tuple(vs) => Ty::Tuple(vs.iter().map(|v| v.ty()).collect()),
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Tuple(vs) => {
                write!(f, "(")?;
                for (i, v) in vs.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{v}")?;
                }
                write!(f, ")")
            }
            Value::Int(v) => write!(f, "{}", v),
            Value::Float(v) => write!(f, "{}", v),
            Value::Bool(v) => write!(f, "{}", v),
            Value::Str(s) => write!(f, "{}", s),
            Value::FloatList(vs) => {
                write!(f, "[")?;
                for (i, v) in vs.iter().enumerate() {
                    if i > 0 {
                        write!(f, ",")?;
                    }
                    write!(f, "{}", v)?;
                }
                write!(f, "]")
            }
            Value::List(vs) => {
                write!(f, "[")?;
                for (i, v) in vs.iter().enumerate() {
                    if i > 0 {
                        write!(f, ",")?;
                    }
                    write!(f, "{}", v)?;
                }
                write!(f, "]")
            }
        }
    }
}

/// One input→output evidence pair. `tol` applies to F64 outputs:
/// pass when |got - want| <= tol + 1e-9*|want|. Zero means exact.
#[derive(Debug, Clone, PartialEq)]
pub struct Example {
    pub inputs: Vec<Value>,
    pub output: Value,
    pub tol: f64,
}

/// Parsed gen with the transparent/opaque split already applied.
#[derive(Debug, Clone)]
pub struct Gen {
    pub path: String,
    pub name: String,
    pub params: Vec<(String, Ty)>,
    pub ret: Ty,
    pub invariants: Vec<Expr>,
    pub transparent: Vec<Example>,
    pub opaque: Vec<Example>,
    pub auto_split: bool,
    /// Vault symbols this gen may call: `use Stats.mean` lines.
    pub deps: Vec<String>,
    /// Author guidance for the forge. Advice, never evidence (rule 12):
    /// flows into prompts only, never into canonical text or verdicts.
    pub hints: Vec<String>,
    /// Raw source chunk this gen was parsed from (spec text incl. opaque
    /// examples). Ships in vault manifests as `gen_text` so packages can
    /// re-verify on import. Never part of canonical text.
    pub source: String,
}

/// Public wrapper so recipe.rs can reuse the example value grammar for
/// program literals without duplicating it.
pub fn parse_example_line_pub(line: &str) -> Result<Example, String> {
    parse_example_line(line, "value")
}

/// Parse a single value token: int, bool, or `[i,i,...]` list literal.
fn parse_value(s: &str) -> Result<Value, String> {
    let t = s.trim();
    // Tuple literal: `(v1, v2, ...)`. Components parse recursively.
    if t.starts_with('(') && t.ends_with(')') {
        let inner = &t[1..t.len() - 1];
        // Bracket-aware split: list literals inside tuples carry commas.
        let parts: Result<Vec<Value>, String> = split_top_level(inner)
            .iter()
            .filter(|p| !p.trim().is_empty())
            .map(|p| parse_value(p).map_err(|e| format!("tuple component: {e}")))
            .collect();
        return Ok(Value::Tuple(parts?));
    }
    if t == "true" {
        return Ok(Value::Bool(true));
    }
    if t == "false" {
        return Ok(Value::Bool(false));
    }
    // Quoted string literal: opaque value, only str_len/str_eq consume it.
    if t.len() >= 2 && t.starts_with('"') && t.ends_with('"') {
        let inner = &t[1..t.len() - 1];
        if inner.contains('"') {
            return Err("string literal contains unescaped quote".to_string());
        }
        return Ok(Value::Str(inner.to_string()));
    }
    // List literals first: `[2.0, 4.0]` contains '.', which must not trip
    // scalar-float detection.
    if let Some(inner) = t.strip_prefix('[').and_then(|x| x.strip_suffix(']')) {
        let inner = inner.trim();
        if inner.is_empty() {
            // Empty list is Int-typed by default; typed by usage validation.
            return Ok(Value::List(Vec::new()));
        }
        let is_float = inner
            .split(',')
            .any(|p| p.contains('.') || p.contains('e') || p.contains('E'));
        let mut items_i = Vec::new();
        let mut items_f = Vec::new();
        for part in inner.split(',') {
            let p = part.trim();
            if is_float {
                items_f.push(
                    p.parse::<f64>()
                        .map_err(|_| format!("bad float `{}`", p))?,
                );
            } else {
                items_i.push(parse_int(p)?);
            }
        }
        return Ok(if is_float {
            Value::FloatList(items_f)
        } else {
            Value::List(items_i)
        });
    }
    if t.contains('.') || t.contains('e') || t.contains('E') {
        let v: f64 = t.parse().map_err(|_| format!("bad float `{}`", t))?;
        return Ok(Value::Float(v));
    }
    Ok(Value::Int(parse_int(t)?))
}

fn parse_int(t: &str) -> Result<i64, String> {
    t.parse::<i64>()
        .map_err(|_| format!("bad integer `{}`", t))
}

/// Split a comma-separated argument list respecting bracket nesting.
fn split_top_level(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut cur = String::new();
    for c in s.chars() {
        match c {
            '[' => {
                depth += 1;
                cur.push(c);
            }
            ']' => {
                depth = depth.saturating_sub(1);
                cur.push(c);
            }
            ',' if depth == 0 => {
                parts.push(cur.clone());
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() || !parts.is_empty() {
        parts.push(cur);
    }
    parts
}

fn parse_example_line(line: &str, what: &str) -> Result<Example, String> {
    let body = line
        .strip_prefix("=>")
        .or_else(|| line.strip_prefix("??"))
        .ok_or_else(|| format!("{} line must start with => or ??", what))?;
    let (in_s, out_s) = body
        .split_once("->")
        .ok_or_else(|| format!("{} line missing `->`", what))?;
    let mut inputs = Vec::new();
    for part in split_top_level(in_s) {
        inputs.push(parse_value(&part).map_err(|e| format!("{}: {}", what, e))?);
    }
    // Optional tolerance suffix on float outputs: `-> 6.28 ± 1e-9`
    let (out_v, tol) = match out_s.split_once('±') {
        Some((v_s, tol_s)) => {
            let v = parse_value(v_s.trim()).map_err(|e| format!("{}: {}", what, e))?;
            let t: f64 = tol_s
                .trim()
                .parse()
                .map_err(|_| format!("{}: bad tolerance `{}`", what, tol_s.trim()))?;
            (v, t.abs())
        }
        None => (parse_value(out_s).map_err(|e| format!("{}: {}", what, e))?, 0.0),
    };
    if matches!(out_v, Value::Int(_) | Value::Bool(_)) && tol != 0.0 {
        return Err(format!(
            "{}: ± tolerance only applies to F64 outputs",
            what
        ));
    }
    Ok(Example {
        inputs,
        output: out_v,
        tol,
    })
}

/// Parse a full `.ont` gen. Applies auto-split when no explicit `??` exist.
pub fn parse(src: &str) -> Result<Gen, String> {
    let mut path = String::new();
    let mut params: Vec<(String, Ty)> = Vec::new();
    let mut ret = Ty::Int;
    let mut invariants = Vec::new();
    let mut deps: Vec<String> = Vec::new();
    let mut hints: Vec<String> = Vec::new();
    let mut transparent = Vec::new();
    let mut opaque = Vec::new();

    for (lineno, raw) in src.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        let ctx = |msg: String| format!("line {}: {}", lineno + 1, msg);
        if let Some(rest) = line.strip_prefix("hint ") {
            let quoted = rest.trim();
            let text = quoted
                .strip_prefix('"')
                .and_then(|x| x.strip_suffix('"'))
                .map(|x| x.to_string())
                .ok_or_else(|| {
                    ctx(format!("hint must be double-quoted: `{}`", line))
                })?;
            hints.push(text);
            continue;
        }
        if let Some(dep) = line.strip_prefix("use ") {
            if !path.is_empty() {
                return Err(ctx("use lines must precede the fn signature".into()));
            }
            deps.push(dep.trim().to_string());
            continue;
        }
        if let Some(sig) = line.strip_prefix("fn ") {
            let sig = sig.trim();
            let (head, tail) = sig
                .split_once('(')
                .ok_or_else(|| ctx("signature missing `(`".into()))?;
            let (args_s, ret_s) = tail
                .split_once(')')
                .ok_or_else(|| ctx("signature missing `)`".into()))?;
            path = head.trim().to_string();
            for part in split_top_level(args_s) {
                let p = part.trim();
                if p.is_empty() {
                    continue;
                }
                let (pname, pty) = p
                    .split_once(':')
                    .ok_or_else(|| ctx(format!("param missing type: `{}`", p)))?;
                params.push((parse_pident(pname.trim())?, parse_type(pty.trim())?));
            }
            ret = parse_type(
                ret_s
                    .trim()
                    .strip_prefix("->")
                    .unwrap_or(ret_s.trim())
                    .trim(),
            )?;
            continue;
        }
        if let Some(inv) = line.strip_prefix('|') {
            let e = sketch::parse_expr_str(inv)
                .map_err(|e| ctx(format!("invariant: {} at {}", e.message, e.offset)))?;
            invariants.push(e);
            continue;
        }
        if line.starts_with("=>") {
            transparent.push(parse_example_line(line, "example").map_err(ctx)?);
            continue;
        }
        if line.starts_with("??") {
            opaque.push(parse_example_line(line, "opaque example").map_err(ctx)?);
            continue;
        }
        return Err(ctx(format!("unrecognized line: `{}`", line)));
    }

    if path.is_empty() {
        return Err("missing `fn` signature".to_string());
    }

    let mut gen = Gen {
        name: path.rsplit('.').next().unwrap_or(&path).to_string(),
        path,
        params,
        ret,
        invariants,
        transparent,
        opaque,
        auto_split: false,
        deps,
        hints,
        source: src.trim_end().to_string(),
    };
    apply_auto_split(&mut gen);
    validate(&gen)?;
    Ok(gen)
}

fn parse_pident(s: &str) -> Result<String, String> {
    s.strip_prefix('%')
        .map(|x| x.to_string())
        .filter(|x| !x.is_empty())
        .ok_or_else(|| format!("param must be %-prefixed: `{}`", s))
}

fn parse_type(s: &str) -> Result<Ty, String> {
    let t = s.trim();
    if t.starts_with('(') && t.ends_with(')') {
        let inner = &t[1..t.len() - 1];
        let parts: Result<Vec<Ty>, String> = inner
            .split(',')
            .map(|p| parse_type(p).map_err(|e| format!("tuple component: {e}")))
            .collect();
        return Ok(Ty::Tuple(parts?));
    }
    let s = t;
    match s {
        "Int" => Ok(Ty::Int),
        "F64" => Ok(Ty::F64),
        "F32" => Ok(Ty::F32),
        "Bool" => Ok(Ty::Bool),
        "Str" => Ok(Ty::Str),
        "List<Int>" => Ok(Ty::ListInt),
        "List<F64>" => Ok(Ty::ListF64),
        "List<F32>" => Ok(Ty::ListF32),
        other => Err(format!(
            "unsupported type `{}` (v1: Int, F64, F32, Bool, Str, List<Int>, List<F64>, List<F32>)",
            other
        )),
    }
}

/// Deterministic auto-hide: keep first ceil(n/2), hide last floor(n/2).
fn apply_auto_split(gen: &mut Gen) {
    if !gen.opaque.is_empty() || gen.transparent.len() < 4 {
        return;
    }
    let n = gen.transparent.len();
    let hide = n / 2;
    let keep = n - hide;
    let hidden: Vec<Example> = gen.transparent.drain(keep..).collect();
    gen.opaque = hidden;
    gen.auto_split = true;
}

/// Structural validation: arity/type agreement of every example.
fn validate(gen: &Gen) -> Result<(), String> {
    if gen.transparent.is_empty() {
        return Err(format!("gen `{}` has no transparent examples", gen.path));
    }
    check_set(gen, &gen.transparent, "transparent")?;
    check_set(gen, &gen.opaque, "opaque")
}


/// Widen an int literal to float in place when the declared slot is F64.
/// Returns true when a coercion happened.
/// Widen int literals to floats when the declared slot is F64 (params,
/// list elements, outputs). Canonical serialization writes whole floats
/// as bare integers; the canonical form must re-parse. Semantics are
/// identical and vault keys are unaffected (computed from canonical()).
fn coerce_f64(v: &mut Value, t: &Ty) -> bool {
    // Compute the replacement first; mutating through `v` inside a match
    // that borrows it fights the borrow checker.
    let repl = match (t, &*v) {
        (Ty::F64, Value::Int(i)) => Some(Value::Float(*i as f64)),
        (Ty::ListF64, Value::List(items)) => {
            Some(Value::FloatList(items.iter().map(|&i| i as f64).collect()))
        }
        _ => None,
    };
    match repl {
        Some(nv) => {
            *v = nv;
            true
        }
        None => false,
    }
}

fn check_set(gen: &Gen, set: &[Example], label: &str) -> Result<(), String> {
    for ex in set {
        if ex.inputs.len() != gen.params.len() {
            return Err(format!(
                "gen `{}`: {} example arity {} != signature arity {}",
                gen.path,
                label,
                ex.inputs.len(),
                gen.params.len()
            ));
        }
        // Int literals coerce into declared F64 slots (params, list elems,
        // output). Canonical serialization writes whole floats as `3`; the
        // canonical form must re-parse. Semantics identical (interp sees
        // Float(3.0)); vault keys unchanged (computed from canonical()).
        let mut inputs = ex.inputs.clone();
        let mut output = ex.output.clone();
        let mut coerced = false;
        for (v, (_, t)) in inputs.iter_mut().zip(gen.params.iter()) {
            coerced |= coerce_f64(v, t);
        }
        coerced |= coerce_f64(&mut output, &gen.ret);
        let ex = Example {
            inputs,
            output,
            tol: ex.tol,
        };
        let _ = coerced;
        for ((v, (_, t)), idx) in ex.inputs.iter().zip(gen.params.iter()).zip(0..) {
            // Empty list literals are polymorphic; they adopt the declared
            // element type of the parameter they feed.
            let polymorphic_empty =
                matches!(v, Value::List(vs) if vs.is_empty()) && matches!(t, Ty::ListF64);
            // F32/F64 are interchangeable at the gen level (interp uses f64 internally).
            let float_compat = matches!(
                (v.ty(), t),
                (Ty::F64, Ty::F32) | (Ty::F32, Ty::F64)
                    | (Ty::ListF64, Ty::ListF32) | (Ty::ListF32, Ty::ListF64)
            );
            if v.ty() != *t && !polymorphic_empty && !float_compat {
                return Err(format!(
                    "gen `{}`: {} example param #{} is {}, expected {}",
                    gen.path,
                    label,
                    idx + 1,
                    v.ty().name(),
                    t.name()
                ));
            }
        }
        if ex.output.ty() != gen.ret {
            // F32/F64 are interchangeable at the gen level.
            let float_ret = matches!(
                (ex.output.ty(), &gen.ret),
                (Ty::F64, Ty::F32) | (Ty::F32, Ty::F64)
                    | (Ty::ListF64, Ty::ListF32) | (Ty::ListF32, Ty::ListF64)
            );
            if !float_ret {
                return Err(format!(
                    "gen `{}`: {} example output is {}, expected {}",
                    gen.path,
                    label,
                    ex.output.ty().name(),
                    gen.ret.name()
                ));
            }
        }
    }
    Ok(())
}

impl Gen {
    /// Canonical deterministic serialization — the vault hash key payload.
    pub fn canonical(&self) -> String {
        let params: Vec<String> = self
            .params
            .iter()
            .map(|(n, t)| format!("%{}: {}", n, t.name()))
            .collect();
        let invs: Vec<String> = self
            .invariants
            .iter()
            .map(|e| format!("| {}", crate::lower::expr_display(e)))
            .collect();
        let trans: Vec<String> = self.transparent.iter().map(example_str).collect();
        let _ = &self.opaque; // opaque set deliberately excluded from canonical text
        let mut out = String::new();
        for d in &self.deps {
            out.push_str(&format!("use {}\n", d));
        }
        out.push_str(&format!(
            "fn {}({}) -> {}\n",
            self.path,
            params.join(", "),
            self.ret.name()
        ));
        for i in invs {
            out.push_str(&i);
            out.push('\n');
        }
        // NOTE: only the transparent set enters the canonical form. The opaque
        // set is sieve-internal evidence and MUST NOT influence the key that
        // forge-facing caches derive from.
        for t in trans {
            out.push_str(&t);
            out.push('\n');
        }
        out
    }
}

fn example_str(ex: &Example) -> String {
    let ins: Vec<String> = ex.inputs.iter().map(|v| v.to_string()).collect();
    format!("=> {} -> {}", ins.join(", "), ex.output)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEDGER: &str = "\
fn Ledger.total(%items: List<Int>) -> Int
  | %res >= 0
  => [1,2,3] -> 6
  => [] -> 0
  => [5] -> 5
  => [10,20,30] -> 60
";

    #[test]
    fn test_parse_ledger_wish() {
        let w = parse(LEDGER).expect("parses");
        assert_eq!(w.name, "total");
        assert_eq!(w.params.len(), 1);
        assert_eq!(w.invariants.len(), 1);
    }

    #[test]
    fn test_auto_split_hides_half_when_no_opaque() {
        let w = parse(LEDGER).expect("parses");
        assert!(w.auto_split);
        assert_eq!(w.transparent.len(), 2);
        assert_eq!(w.opaque.len(), 2);
        // First ceil(n/2) stay transparent, last floor(n/2) become opaque.
        assert_eq!(w.transparent[0].output, Value::Int(6));
        assert_eq!(w.opaque[1].output, Value::Int(60));
    }

    #[test]
    fn test_explicit_opaque_wins_no_auto_split() {
        let src = "\
fn f(%a: Int) -> Int
  => 1 -> 2
  ?? 2 -> 3
";
        let w = parse(src).expect("parses");
        assert!(!w.auto_split);
        assert_eq!(w.transparent.len(), 1);
        assert_eq!(w.opaque.len(), 1);
    }

    #[test]
    fn test_small_sets_never_auto_split() {
        let src = "fn f(%a: Int) -> Int\n  => 1 -> 2\n  => 2 -> 4\n";
        let w = parse(src).expect("parses");
        assert!(!w.auto_split);
        assert_eq!(w.opaque.len(), 0);
    }

    #[test]
    fn test_multi_param_and_bool() {
        let src = "fn g(%a: Int, %b: Bool) -> Int\n  => 3, true -> 4\n";
        let w = parse(src).expect("parses");
        assert_eq!(w.transparent[0].inputs[1], Value::Bool(true));
    }

    #[test]
    fn test_arity_mismatch_rejected() {
        let src = "fn f(%a: Int) -> Int\n  => 1, 2 -> 3\n";
        assert!(parse(src).is_err());
    }

    #[test]
    fn test_canonical_is_stable_and_transparent_only() {
        let w = parse(LEDGER).expect("parses");
        let c1 = w.canonical();
        let c2 = parse(&c1).expect("canonical reparses").canonical();
        assert_eq!(c1, c2);
        assert!(!c1.contains("10,20,30"), "opaque set leaked into canonical");
    }
}

#[cfg(test)]
mod float_tests {
    use super::*;

    #[test]
    fn test_float_example_with_tolerance() {
        let w = parse("fn g(%x: F64) -> F64\n  => 2.0 -> 6.28 ± 1e-9\n").unwrap();
        assert_eq!(w.transparent[0].tol, 1e-9);
        assert_eq!(w.transparent[0].output, Value::Float(6.28));
    }

    #[test]
    fn test_tolerance_on_int_output_rejected() {
        assert!(parse("fn g(%x: Int) -> Int\n  => 2 -> 6 ± 1e-9\n").is_err());
    }

    #[test]
    fn test_f64_type_roundtrip() {
        let w = parse("fn g(%x: F64) -> F64\n  => 1e-9 -> 2.5E3\n").unwrap();
        assert!(matches!(w.transparent[0].output, Value::Float(2500.0)));
        assert!(w.canonical().contains("F64"));
    }
}

#[cfg(test)]
mod hint_tests {
    use super::*;

    const HINTED: &str = "\
fn Stats.meansqdev(%xs: List<F64>) -> F64
  | %res >= 0.0
  hint \"two passes: mean first via let, then fold deviations\"
  hint \"guard empty list before dividing by len\"
  => [2.0,4.0] -> 1.0 ± 1e-9
";

    #[test]
    fn test_hints_parse_and_preserve_order() {
        let w = parse(HINTED).unwrap();
        assert_eq!(w.hints.len(), 2);
        assert!(w.hints[0].contains("two passes"));
        assert!(w.hints[1].contains("guard empty"));
    }

    #[test]
    fn test_hints_excluded_from_canonical() {
        let with_h = parse(HINTED).unwrap();
        let without = parse("fn Stats.meansqdev(%xs: List<F64>) -> F64\n  | %res >= 0.0\n  => [2.0,4.0] -> 1.0 ± 1e-9\n").unwrap();
        // Same contract ⇒ same vault key regardless of hints.
        assert_eq!(with_h.canonical(), without.canonical());
    }

    #[test]
    fn test_unquoted_hint_rejected() {
        assert!(parse("fn f(%a: Int) -> Int\n  hint two passes\n  => 1 -> 1\n").is_err());
    }
}
