//! Recipes: linear programs over verified functions.
//!
//! THE WALL preserved: recipe glue is deterministic text; every computation
//! lives in sieved, vault-verified gens. One `.ont` may hold many `fn`
//! gens plus at most one `program` block.
//!
//! ```text
//! program Demo
//!   gen Ledger.total
//! start
//!   %xs = [1,2,3]
//!   %r  = Ledger.total(%xs)
//!   print(%r)
//! end
//! ```

use crate::sketch::Ty;
use crate::gen::{self, Gen};

/// One linear statement.
#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    /// `%x = [1,2,3]` / `%x = 7`
    BindLit(String, gen::Value),
    /// `%r = Path.name(%a, 7)` — callee must be a declared dependency;
    /// args are variables or literals.
    BindCall(String, String, Vec<CallArg>),
    /// `print(%v)`
    Print(String),
    /// `write %v -> "path.csv"` — CSV: scalar row / list column.
    Write(String, String),
    /// `dump %v -> "out.json"` — JSON object {"name": value}.
    Dump(String, String),
    /// `log "text %var ..."` — console with interpolation.
    Log(Vec<LogSeg>),
}

/// One segment of a log template.
#[derive(Debug, Clone, PartialEq)]
pub enum LogSeg {
    Text(String),
    Var(String),
}

/// Call-site argument.
#[derive(Debug, Clone, PartialEq)]
pub enum CallArg {
    Var(String),
    Lit(gen::Value),
}

impl CallArg {
    fn ty(&self, locals: &[(String, Ty)]) -> Result<Ty, String> {
        match self {
            CallArg::Var(n) => lookup(locals, n),
            CallArg::Lit(v) => Ok(v.ty()),
        }
    }
}

/// Parsed program block.
#[derive(Debug, Clone)]
pub struct Program {
    pub name: String,
    /// Declared dependencies, in order. Each must resolve to a verified gen.
    pub deps: Vec<String>,
    pub body: Vec<Stmt>,
}

/// A whole `.ont` file: possibly several gens plus an optional program.
#[derive(Debug, Clone)]
pub struct OntFile {
    pub gens: Vec<Gen>,
    pub program: Option<Program>,
}

/// Split raw `.ont` text into gen chunks and an optional program block.
/// A line starting with `fn ` begins a new gen chunk; indented continuation
/// lines (`|`, `=>`, `??`, `wrapping`) join the open chunk.
fn split_chunks(src: &str) -> Result<(Vec<String>, Vec<String>), String> {
    let mut gens: Vec<String> = Vec::new();
    let mut prog_lines: Vec<String> = Vec::new();
    let mut in_program = false;
    let mut pending: Option<String> = None;
    for (lineno, raw) in src.lines().enumerate() {
        let line = raw.trim_end();
        let trimmed = line.trim();
        let ctx = |m: &str| format!("line {}: {}", lineno + 1, m);
        let _ = &ctx;
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        if trimmed.starts_with("program ") || trimmed == "program" {
            if in_program {
                return Err(ctx("nested program block"));
            }
            in_program = true;
            prog_lines.push(trimmed.to_string());
            continue;
        }
        if in_program {
            if trimmed == "start" || trimmed == "end" {
                prog_lines.push(trimmed.to_string());
                if trimmed == "end" {
                    in_program = false;
                }
                continue;
            }
            prog_lines.push(trimmed.to_string());
            continue;
        }
        if trimmed.starts_with("use ") && !in_program {
            // Dependency declarations ride the pending prefix onto the gen.
            let e = pending.get_or_insert_with(String::new);
            e.push_str(line);
            e.push('\n');
            continue;
        }
        if trimmed.starts_with("fn ") {
            // A pre-`fn` prefix (e.g. `wrapping`) belongs to this gen.
            gens.push(pending.take().unwrap_or_default());
            let last = gens.last_mut().expect("chunk exists");
            if !last.is_empty() {
                last.push('\n');
            }
        } else if gens.is_empty() {
            // Buffer pre-signature lines; validated by gen::parse later.
            let e = pending.get_or_insert_with(String::new);
            e.push_str(line);
            e.push('\n');
            continue;
        }
        let last = gens.last_mut().expect("chunk exists");
        last.push_str(line);
        last.push('\n');
    }
    if in_program {
        return Err("program block missing `end`".to_string());
    }
    Ok((gens, prog_lines))
}

/// Parse the program block lines into a Program.
fn parse_program(lines: &[String]) -> Result<Program, String> {
    let head = lines.first().ok_or("empty program block")?;
    let name = head
        .strip_prefix("program ")
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .ok_or("program block missing name")?;

    let mut deps = Vec::new();
    let mut body = Vec::new();
    let mut started = false;
    for line in &lines[1..] {
        match line.as_str() {
            "start" => {
                if started {
                    return Err("duplicate `start`".to_string());
                }
                started = true;
                continue;
            }
            "end" => break,
            _ => {}
        }
        if !started {
            let dep = line.strip_prefix("use ").ok_or_else(|| {
                format!("only `use` declarations allowed before start, got `{}`", line)
            })?;
            deps.push(dep.trim().to_string());
            continue;
        }
        body.push(parse_stmt(line)?);
    }
    if !started {
        return Err("program missing `start`".to_string());
    }
    Ok(Program { name, deps, body })
}

/// Parse one statement inside the program body.
fn parse_stmt(line: &str) -> Result<Stmt, String> {
    if let Some(rest) = line.strip_prefix("write") {
        let (var_s, path_s) = rest
            .split_once("->")
            .ok_or_else(|| format!("write needs `-> \"path\"`: `{}`", line))?;
        let var = parse_var_ref(var_s.trim())?;
        let path = unquote(path_s.trim(), "write target")?;
        return Ok(Stmt::Write(var, path));
    }
    if let Some(rest) = line.strip_prefix("dump") {
        let (var_s, path_s) = rest
            .split_once("->")
            .ok_or_else(|| format!("dump needs `-> \"path\"`: `{}`", line))?;
        let var = parse_var_ref(var_s.trim())?;
        let path = unquote(path_s.trim(), "dump target")?;
        return Ok(Stmt::Dump(var, path));
    }
    if let Some(rest) = line.strip_prefix("log ") {
        return Ok(Stmt::Log(parse_log_template(rest.trim())?));
    }
    if let Some(rest) = line.strip_prefix("print") {
        let var = rest.trim();
        let name = var.strip_prefix('(')
            .and_then(|x| x.strip_suffix(')'))
            .map(|x| x.trim())
            .unwrap_or(var);
        return Ok(Stmt::Print(parse_var_ref(name)?));
    }
    if let Some((lhs, rhs)) = line.split_once('=') {
        let target = parse_var_ref(lhs.trim())?;
        let rhs = rhs.trim();
        if rhs.starts_with('[') || rhs.parse::<i64>().is_ok() || rhs == "true" || rhs == "false"
        {
            let val = wish_value_from_str(rhs)?;
            return Ok(Stmt::BindLit(target, val));
        }
        // Call form: Path.name(arg, ...)
        let (callee, args_s) = rhs
            .split_once('(')
            .ok_or_else(|| format!("unrecognized right-hand side `{}`", rhs))?;
        let args_s = args_s
            .strip_suffix(')')
            .ok_or_else(|| format!("call missing closing paren: `{}`", rhs))?;
        let mut args = Vec::new();
        for part in args_s.split(',') {
            let p = part.trim();
            if p.is_empty() {
                continue;
            }
            if let Some(v) = p.strip_prefix('%') {
                args.push(CallArg::Var(v.to_string()));
            } else {
                args.push(CallArg::Lit(wish_value_from_str(p)?));
            }
        }
        return Ok(Stmt::BindCall(target, callee.trim().to_string(), args));
    }
    Err(format!("unrecognized statement `{}`", line))
}

fn unquote(s: &str, what: &str) -> Result<String, String> {
    s.strip_prefix('"')
        .and_then(|x| x.strip_suffix('"'))
        .map(|x| x.to_string())
        .filter(|x| !x.is_empty())
        .ok_or_else(|| format!("{} must be a double-quoted path", what))
}

/// Split a log template into text and %var segments.
fn parse_log_template(s: &str) -> Result<Vec<LogSeg>, String> {
    let quoted = s
        .strip_prefix('"')
        .and_then(|x| x.strip_suffix('"'))
        .ok_or_else(|| format!("log template must be double-quoted: `{}`", s))?;
    let mut segs = Vec::new();
    let mut text = String::new();
    let mut chars = quoted.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            let mut name = String::new();
            while let Some(&n) = chars.peek() {
                if n.is_ascii_alphanumeric() || n == '_' {
                    name.push(n);
                    chars.next();
                } else {
                    break;
                }
            }
            if !text.is_empty() {
                segs.push(LogSeg::Text(std::mem::take(&mut text)));
            }
            if name.is_empty() {
                return Err("log template has bare %".to_string());
            }
            segs.push(LogSeg::Var(name));
        } else {
            text.push(c);
        }
    }
    if !text.is_empty() {
        segs.push(LogSeg::Text(text));
    }
    Ok(segs)
}

/// `%name` reference without the sigil.
fn parse_var_ref(s: &str) -> Result<String, String> {
    s.strip_prefix('%')
        .map(|x| x.to_string())
        .filter(|x| !x.is_empty())
        .ok_or_else(|| format!("expected %variable, got `{}`", s))
}

/// Reuse the gen example-value scanner for literals.
fn wish_value_from_str(s: &str) -> Result<gen::Value, String> {
    // Values share syntax with examples; wrap as a tiny fake example and
    // borrow its parser to avoid duplicating grammar.
    let ex = format!("=> {} -> 0", s);
    let parsed = crate::gen::parse_example_line_pub(&ex)?;
    Ok(parsed.inputs.into_iter().next().expect("one value"))
}

// ---------------------------------------------------------------------------
// Typechecking
// ---------------------------------------------------------------------------

/// Type of every live local after checking the whole program.
pub fn typecheck(prog: &Program, gens: &[Gen]) -> Result<(), String> {
    // Dependency resolution: same-file gens by full path.
    let mut sigs: Vec<&Gen> = Vec::new();
    for dep in &prog.deps {
        match gens.iter().find(|w| &w.path == dep) {
            Some(w) => sigs.push(w),
            None => {
                return Err(format!(
                    "dependency `{}` not found among same-file gens",
                    dep
                ))
            }
        }
    }

    let mut locals: Vec<(String, Ty)> = Vec::new();
    for stmt in &prog.body {
        match stmt {
            Stmt::BindLit(name, value) => {
                declare(&mut locals, name.clone(), value.ty())?;
            }
            Stmt::BindCall(target, callee, args) => {
                let sig = sigs
                    .iter()
                    .find(|w| &w.path == callee)
                    .ok_or_else(|| format!("call to undeclared dependency `{}`", callee))?;
                if args.len() != sig.params.len() {
                    return Err(format!(
                        "`{}` expects {} args, got {}",
                        callee,
                        sig.params.len(),
                        args.len()
                    ));
                }
                for (idx, (arg, (_, want))) in
                    args.iter().zip(sig.params.iter()).enumerate()
                {
                    let got = arg.ty(&locals)?;
                    if got != *want {
                        return Err(format!(
                            "`{}` arg #{} wants {}, got {}",
                            callee,
                            idx + 1,
                            want.name(),
                            got.name()
                        ));
                    }
                }
                declare(&mut locals, target.clone(), sig.ret.clone())?;
            }
            Stmt::Print(name) => {
                lookup(&locals, name)?;
            }
            Stmt::Write(name, _) | Stmt::Dump(name, _) => {
                lookup(&locals, name)?;
            }
            Stmt::Log(segs) => {
                for seg in segs {
                    if let LogSeg::Var(n) = seg {
                        lookup(&locals, n)?;
                    }
                }
            }
        }
    }
    Ok(())
}

fn declare(locals: &mut Vec<(String, Ty)>, name: String, ty: Ty) -> Result<(), String> {
    if locals.iter().any(|(n, _)| n == &name) {
        return Err(format!("duplicate local `%{}`", name));
    }
    locals.push((name, ty));
    Ok(())
}

fn lookup(locals: &[(String, Ty)], name: &str) -> Result<Ty, String> {
    locals
        .iter()
        .rev()
        .find(|(n, _)| n == name)
        .map(|(_, t)| t.clone())
        .ok_or_else(|| format!("undefined variable `%{}`", name))
}

/// Parse a full `.ont` file: many gens, optional program.
pub fn parse_ont(src: &str) -> Result<OntFile, String> {
    let (chunks, prog_lines) = split_chunks(src)?;
    if chunks.is_empty() {
        return Err("no gens in file".to_string());
    }
    let mut gens = Vec::new();
    for chunk in &chunks {
        gens.push(gen::parse(chunk)?);
    }
    let program = if prog_lines.is_empty() {
        None
    } else {
        Some(parse_program(&prog_lines)?)
    };
    let file = OntFile { gens, program };
    if let Some(prog) = &file.program {
        typecheck(prog, &file.gens)?;
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "\
fn Ledger.total(%items: List<Int>) -> Int
  | %res >= -1000000000
  wrapping
  => [1,2,3] -> 6
  => [] -> 0

fn Twice(%n: Int) -> Int
  => 21 -> 42

program Demo
  use Ledger.total
  use Twice
start
  %xs = [1,2,3]
  %r  = Ledger.total(%xs)
  print(%r)
  %n  = Twice(21)
  print(%n)
end
";

    #[test]
    fn test_parse_multi_wish_file_with_program() {
        let f = parse_ont(FILE).expect("parses");
        assert_eq!(f.gens.len(), 2);
        let prog = f.program.expect("has program");
        assert_eq!(prog.name, "Demo");
        assert_eq!(prog.deps, vec!["Ledger.total", "Twice"]);
        assert_eq!(prog.body.len(), 5);
        assert_eq!(
            prog.body[0],
            Stmt::BindLit("xs".into(), gen::Value::List(vec![1, 2, 3]))
        );
        assert_eq!(
            prog.body[1],
            Stmt::BindCall(
            "r".into(),
            "Ledger.total".into(),
            vec![CallArg::Var("xs".into())],
        )
        );
        assert_eq!(prog.body[2], Stmt::Print("r".into()));
    }

    #[test]
    fn test_gens_remain_individually_valid() {
        let f = parse_ont(FILE).expect("parses");
        assert_eq!(f.gens[0].name, "total");
        assert!(f.gens[0].wrapping);
        assert_eq!(f.gens[1].name, "Twice");
        assert!(!f.gens[1].wrapping);
        assert_eq!(f.gens[0].transparent.len(), 2);
    }

    #[test]
    fn test_typecheck_catches_arity_and_types() {
        let bad_arity = FILE.replace("Twice(21)", "Twice()");
        assert!(parse_ont(&bad_arity).is_err());
        let bad_type = FILE.replace("Ledger.total(%xs)", "Ledger.total(%n)");
        assert!(parse_ont(&bad_type).is_err());
        let undef = FILE.replace("%xs = [1,2,3]\n  ", "");
        assert!(parse_ont(&undef).is_err());
    }

    #[test]
    fn test_missing_dependency_rejected() {
        let missing = FILE.replace("  use Twice\n", "");
        let e = parse_ont(&missing).expect_err("must fail");
        assert!(e.contains("undeclared dependency"), "{}", e);
    }

    #[test]
    fn test_unclosed_program_rejected() {
        let unclosed = FILE.replace("end\n", "");
        assert!(parse_ont(&unclosed).is_err());
    }
}
