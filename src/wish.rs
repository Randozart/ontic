//! Wish parsing: `.ont` specification files.
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
    Bool(bool),
    List(Vec<i64>),
}

impl Value {
    /// The wish-level type of this value.
    pub fn ty(&self) -> Ty {
        match self {
            Value::Int(_) => Ty::Int,
            Value::Bool(_) => Ty::Bool,
            Value::List(_) => Ty::ListInt,
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Int(v) => write!(f, "{}", v),
            Value::Bool(v) => write!(f, "{}", v),
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

/// One input→output evidence pair.
#[derive(Debug, Clone, PartialEq)]
pub struct Example {
    pub inputs: Vec<Value>,
    pub output: Value,
}

/// Parsed wish with the transparent/opaque split already applied.
#[derive(Debug, Clone)]
pub struct Wish {
    pub path: String,
    pub name: String,
    pub params: Vec<(String, Ty)>,
    pub ret: Ty,
    pub invariants: Vec<Expr>,
    pub transparent: Vec<Example>,
    pub opaque: Vec<Example>,
    pub auto_split: bool,
    /// Declared wrapping tier: arithmetic wraps mod 2^64 instead of killing
    /// candidates on overflow. Speed requires declaration (AGENTS rule 11).
    pub wrapping: bool,
}

/// Parse a single value token: int, bool, or `[i,i,...]` list literal.
fn parse_value(s: &str) -> Result<Value, String> {
    let t = s.trim();
    if t == "true" {
        return Ok(Value::Bool(true));
    }
    if t == "false" {
        return Ok(Value::Bool(false));
    }
    if let Some(inner) = t.strip_prefix('[').and_then(|x| x.strip_suffix(']')) {
        let inner = inner.trim();
        if inner.is_empty() {
            return Ok(Value::List(Vec::new()));
        }
        let mut items = Vec::new();
        for part in inner.split(',') {
            items.push(parse_int(part.trim())?);
        }
        return Ok(Value::List(items));
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
    let output = parse_value(out_s).map_err(|e| format!("{}: {}", what, e))?;
    Ok(Example { inputs, output })
}

/// Parse a full `.ont` wish. Applies auto-split when no explicit `??` exist.
pub fn parse(src: &str) -> Result<Wish, String> {
    let mut path = String::new();
    let mut params: Vec<(String, Ty)> = Vec::new();
    let mut ret = Ty::Int;
    let mut invariants = Vec::new();
    let mut wrapping = false;
    let mut transparent = Vec::new();
    let mut opaque = Vec::new();

    for (lineno, raw) in src.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        let ctx = |msg: String| format!("line {}: {}", lineno + 1, msg);
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
        if line == "wrapping" {
            wrapping = true;
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

    let mut wish = Wish {
        name: path.rsplit('.').next().unwrap_or(&path).to_string(),
        path,
        params,
        ret,
        invariants,
        transparent,
        opaque,
        auto_split: false,
        wrapping,
    };
    apply_auto_split(&mut wish);
    validate(&wish)?;
    Ok(wish)
}

fn parse_pident(s: &str) -> Result<String, String> {
    s.strip_prefix('%')
        .map(|x| x.to_string())
        .filter(|x| !x.is_empty())
        .ok_or_else(|| format!("param must be %-prefixed: `{}`", s))
}

fn parse_type(s: &str) -> Result<Ty, String> {
    match s {
        "Int" => Ok(Ty::Int),
        "Bool" => Ok(Ty::Bool),
        "List<Int>" => Ok(Ty::ListInt),
        other => Err(format!("unsupported type `{}` (v0: Int, Bool, List<Int>)", other)),
    }
}

/// Deterministic auto-hide: keep first ceil(n/2), hide last floor(n/2).
fn apply_auto_split(wish: &mut Wish) {
    if !wish.opaque.is_empty() || wish.transparent.len() < 4 {
        return;
    }
    let n = wish.transparent.len();
    let hide = n / 2;
    let keep = n - hide;
    let hidden: Vec<Example> = wish.transparent.drain(keep..).collect();
    wish.opaque = hidden;
    wish.auto_split = true;
}

/// Structural validation: arity/type agreement of every example.
fn validate(wish: &Wish) -> Result<(), String> {
    if wish.transparent.is_empty() {
        return Err(format!("wish `{}` has no transparent examples", wish.path));
    }
    check_set(wish, &wish.transparent, "transparent")?;
    check_set(wish, &wish.opaque, "opaque")
}

fn check_set(wish: &Wish, set: &[Example], label: &str) -> Result<(), String> {
    for ex in set {
        if ex.inputs.len() != wish.params.len() {
            return Err(format!(
                "wish `{}`: {} example arity {} != signature arity {}",
                wish.path,
                label,
                ex.inputs.len(),
                wish.params.len()
            ));
        }
        for ((v, (_, t)), idx) in ex.inputs.iter().zip(wish.params.iter()).zip(0..) {
            if v.ty() != *t {
                return Err(format!(
                    "wish `{}`: {} example param #{} is {}, expected {}",
                    wish.path,
                    label,
                    idx + 1,
                    v.ty().name(),
                    t.name()
                ));
            }
        }
        if ex.output.ty() != wish.ret {
            return Err(format!(
                "wish `{}`: {} example output is {}, expected {}",
                wish.path,
                label,
                ex.output.ty().name(),
                wish.ret.name()
            ));
        }
    }
    Ok(())
}

impl Wish {
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
        if self.wrapping {
            out.push_str("wrapping\n");
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
