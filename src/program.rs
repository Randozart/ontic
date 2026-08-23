//! Program assembly: recipe → C driver over verified MLIR objects.
//!
//! The glue is generated C calling flat-MemRef-ABI functions (see
//! pipeline.rs ABI note). Every callee must already be vault-verified;
//! `ontic run` never executes unsieved code.

use crate::pipeline::{self, find_tool};
use crate::recipe::{CallArg, LogSeg, OntFile, Stmt};
use std::path::PathBuf;
use std::process::Command;

/// One resolved dependency ready for linking.
pub struct DepBinding {
    pub path: String,
    /// Function symbol inside the verified MLIR module.
    pub c_fn: String,
    pub params_is_list: Vec<bool>,
    /// Raw verified MLIR text (from the vault).
    pub mlir: String,
}

/// Extract the single `func.func @name` symbol from a vault module.
pub fn fn_symbol(mlir: &str) -> Result<String, String> {
    let start = mlir
        .find("func.func @")
        .ok_or("vault module has no func.func")?
        + "func.func @".len();
    let rest = &mlir[start..];
    let end = rest
        .find('(')
        .ok_or("vault module func.func missing parens")?;
    Ok(rest[..end].trim().to_string())
}

/// Locals tracked while generating the driver body.
enum Local {
    List { c_name: String, len: usize },
    ListF { c_name: String, len: usize },
    Scalar { c_name: String },
    ScalarF { c_name: String },
}

/// Generate the complete C driver for a program.
pub fn driver_source(prog: &crate::recipe::Program, deps: &[DepBinding]) -> Result<String, String> {
    let mut protos = String::new();
    let mut body = String::new();
    let mut locals: Vec<(String, Local)> = Vec::new();

    for dep in deps {
        let mut proto = String::new();
        for (n, is_list) in dep.params_is_list.iter().enumerate() {
            if n > 0 {
                proto.push_str(", ");
            }
            if *is_list {
                proto.push_str("void*, void*, long, long, long");
            } else {
                proto.push_str("long");
            }
        }
        protos.push_str(&format!(
            "extern long {}({});\n",
            c_symbol(&dep.c_fn),
            proto
        ));
    }

    let mut seq = 0usize;
    for stmt in &prog.body {
        match stmt {
            Stmt::BindLit(name, value) => {
                match value {
                    crate::gen::Value::List(vs) => {
                        let cn = format!("v{}", seq);
                        seq += 1;
                        let items: Vec<String> = vs.iter().map(|v| v.to_string()).collect();
                        body.push_str(&format!(
                            "  long {}[] = {{{}}};\n",
                            cn,
                            items.join(", ")
                        ));
                        locals.push((name.clone(), Local::List { c_name: cn, len: vs.len() }));
                    }
                    crate::gen::Value::Int(v) => {
                        let cn = format!("v{}", seq);
                        seq += 1;
                        body.push_str(&format!("  long {} = {}L;\n", cn, v));
                        locals.push((name.clone(), Local::Scalar { c_name: cn }));
                    }
                    crate::gen::Value::Float(v) => {
                        let cn = format!("v{}", seq);
                        seq += 1;
                        body.push_str(&format!("  double {} = {:e};\n", cn, v));
                        locals.push((name.clone(), Local::ScalarF { c_name: cn }));
                    }
                    crate::gen::Value::Bool(b) => {
                        let cn = format!("v{}", seq);
                        seq += 1;
                        body.push_str(&format!("  long {} = {}L;\n", cn, *b as i64));
                        locals.push((name.clone(), Local::Scalar { c_name: cn }));
                    }
                    crate::gen::Value::FloatList(vs) => {
                        let cn = format!("v{}", seq);
                        seq += 1;
                        let items: Vec<String> = vs.iter().map(|x| format!("{:e}", x)).collect();
                        body.push_str(&format!(
                            "  double {}[] = {{{}}};\n",
                            cn,
                            items.join(", ")
                        ));
                        locals.push((name.clone(), Local::ListF { c_name: cn, len: vs.len() }));
                    }
                }
            }
            Stmt::BindCall(target, callee, args) => {
                let dep = deps
                    .iter()
                    .find(|d| &d.path == callee)
                    .ok_or_else(|| format!("callee `{}` not among resolved deps", callee))?;
                if args.len() != dep.params_is_list.len() {
                    return Err(format!("`{}` arity mismatch at codegen", callee));
                }
                let mut call_args = String::new();
                for (arg, is_list) in args.iter().zip(dep.params_is_list.iter()) {
                    match (arg, is_list) {
                        (CallArg::Var(v), true) => match lookup(&locals, v)? {
                            Local::List { c_name, len } => {
                                call_args
                                    .push_str(&format!("{0}, {0}, 0, {1}, 1, ", c_name, len));
                            }
                            Local::ListF { c_name, len } => {
                                call_args
                                    .push_str(&format!("{0}, {0}, 0, {1}, 1, ", c_name, len));
                            }
                            Local::Scalar { .. } | Local::ScalarF { .. } => {
                                return Err(format!("%{} is scalar, `{}` wants list", v, callee))
                            }
                        },
                        (CallArg::Var(v), false) => match lookup(&locals, v)? {
                            Local::Scalar { c_name } => {
                                call_args.push_str(&format!("{}, ", c_name))
                            }
                            Local::ScalarF { c_name } => {
                                call_args.push_str(&format!("{}, ", c_name))
                            }
                            Local::List { .. } | Local::ListF { .. } => {
                                return Err(format!("%{} is list, `{}` wants scalar", v, callee))
                            }
                        },
                        (CallArg::Lit(value), false) => match value {
                            crate::gen::Value::Int(v) => {
                                call_args.push_str(&format!("{}L, ", v))
                            }
                            crate::gen::Value::Float(v) => {
                                call_args.push_str(&format!("{:e}, ", v))
                            }
                            crate::gen::Value::Bool(b) => {
                                call_args.push_str(&format!("{}L, ", *b as i64))
                            }
                            crate::gen::Value::List(_)
                            | crate::gen::Value::FloatList(_) => {
                                return Err(format!("list literal arg to `{}` unsupported", callee))
                            }
                        },
                        (CallArg::Lit(_), true) => {
                            return Err(format!("list literal arg to `{}` unsupported", callee))
                        }
                    }
                }
                let tn = format!("v{}", seq);
                seq += 1;
                body.push_str(&format!(
                    "  long {} = {}({});\n",
                    tn,
                    c_symbol(&dep.c_fn),
                    call_args.trim_end_matches(", ")
                ));
                locals.push((target.clone(), Local::Scalar { c_name: tn }));
            }
            Stmt::Write(name, path) => {
                // CSV: one value per line. Scalars only in v0.
                let c_name = scalar_local(&locals, name)?;
                body.push_str(&format!(
                    "  {{ FILE* f = fopen(\"{p}\", \"w\"); if (!f) return 2; fprintf(f, \"%ld\\n\", {v}); fclose(f); }}\n",
                    p = c_escape(path),
                    v = c_name
                ));
            }
            Stmt::Dump(name, path) => {
                // {"<name>": <value>}\n — quotes escaped for the C literal.
                let c_name = scalar_local(&locals, name)?;
                let p = c_escape(path);
                let key = c_escape(name);
                // Key baked into the literal: exactly one %ld consumes v.
                let fmt = format!("{{\\\"{k}\\\": %ld}}\\n", k = key);
                body.push_str(&format!(
                    "  {{ FILE* f = fopen(\"{p}\", \"w\"); if (!f) return 2; fprintf(f, \"{fmt}\", {v}); fclose(f); }}\n",
                    p = p,
                    fmt = fmt,
                    v = c_name
                ));
            }
            Stmt::Log(segs) => {
                let mut fmt = String::new();
                let mut args: Vec<String> = Vec::new();
                for seg in segs {
                    match seg {
                        LogSeg::Text(t) => fmt.push_str(&c_escape(t).replace("\\n", "\n")),
                        LogSeg::Var(n) => match lookup(&locals, n)? {
                            Local::Scalar { c_name } => {
                                fmt.push_str("%ld");
                                args.push(c_name.clone());
                            }
                            Local::ScalarF { c_name } => {
                                fmt.push_str("%.17g");
                                args.push(c_name.clone());
                            }
                            _ => {
                                return Err(format!(
                                    "log cannot print lists (%{}) in v0",
                                    n
                                ))
                            }
                        },
                    }
                }
                let arg_s = if args.is_empty() {
                    String::new()
                } else {
                    format!(", {}", args.join(", "))
                };
                body.push_str(&format!(
                    "  printf(\"{}\\n\"{});\n",
                    c_escape(&fmt),
                    arg_s
                ));
            }
            Stmt::Print(name) => match lookup(&locals, name)? {
                Local::Scalar { c_name } => {
                    body.push_str(&format!("  printf(\"%ld\\n\", {});\n", c_name));
                }
                Local::ScalarF { c_name } => {
                    body.push_str(&format!(
                        "  printf(\"%.17g\\n\", {});\n",
                        c_name
                    ));
                }
                Local::List { .. } | Local::ListF { .. } => {
                    return Err(format!("cannot print list %{} (v0 prints scalars)", name))
                }
            },
        }
    }

    Ok(format!(
        "#include <stdio.h>\n#include <stdlib.h>\n\n{}\
long ontic_trap(void) {{ abort(); }}\n\n\
int main(void) {{\n{}  return 0;\n}}\n",
        protos, body
    ))
}

/// Escape a Rust string into a C string literal body (no surrounding quotes).
fn c_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '%'
                if false =>
            {
                // percent is legal in plain text; only printf FORMAT strings
                // need escaping, and we only escape the template separately.
            }
            _ => out.push(c),
        }
    }
    out
}

/// Require a scalar local and return its C name.
fn scalar_local<'a>(locals: &'a [(String, Local)], name: &str) -> Result<&'a str, String> {
    match lookup(locals, name)? {
        Local::Scalar { c_name } => Ok(c_name),
        Local::ScalarF { c_name } => Ok(c_name),
        Local::List { .. } | Local::ListF { .. } => Err(format!(
            "effect target `%{} ` must be scalar in v0 (list effects come with P3)",
            name
        )),
    }
}

fn c_symbol(name: &str) -> String {
    // Sketch symbols are [a-z_] identifiers already; keep honest mapping.
    name.to_string()
}

fn lookup<'a>(locals: &'a [(String, Local)], name: &str) -> Result<&'a Local, String> {
    locals
        .iter()
        .rev()
        .find(|(n, _)| n == name)
        .map(|(_, l)| l)
        .ok_or_else(|| format!("undefined variable `%{}`", name))
}

/// Execute a parsed file using the configured vault.
pub fn run(file: &OntFile) -> Result<Vec<String>, String> {
    let vault_dir = std::env::var("ONTIC_VAULT").unwrap_or_else(|_| ".ontic/vault".to_string());
    run_in(file, &vault_dir)
}

/// Execute a parsed file against an explicit vault directory.
pub fn run_in(file: &OntFile, vault_dir: &str) -> Result<Vec<String>, String> {
    let prog = file
        .program
        .as_ref()
        .ok_or("no program block in file")?;
    let dir = scratch();
    let mut bindings: Vec<DepBinding> = Vec::new();
    let mut objects: Vec<PathBuf> = Vec::new();

    for (i, dep_path) in prog.deps.iter().enumerate() {
        let w = file
            .gens
            .iter()
            .find(|w| &w.path == dep_path)
            .ok_or_else(|| format!("dependency `{}` not among same-file gens", dep_path))?;
        let key = crate::vault::Vault::key_for(w);
        let v = crate::vault::Vault::open(vault_dir)?;
        let entry = v.get(&key).ok_or_else(|| {
            format!(
                "dependency `{}` is not solved+vaulted yet — run: ontic solve <file> --hand <candidate>",
                dep_path
            )
        })?;
        let c_fn = fn_symbol(&entry.mlir)?;
        let mlir_p = dir.join(format!("dep{}.mlir", i));
        let ll_p = dir.join(format!("dep{}_llvm.mlir", i));
        let o_p = dir.join(format!("dep{}.o", i));
        std::fs::write(&mlir_p, &entry.mlir).map_err(|e| e.to_string())?;
        pipeline::mlir_to_llvmir(&mlir_p, &ll_p)?;
        pipeline::object_from_ll(&ll_p, &o_p)?;
        objects.push(o_p);
        bindings.push(DepBinding {
            path: dep_path.clone(),
            c_fn,
            params_is_list: w.params.iter().map(|(_, t)| matches!(t, crate::sketch::Ty::ListInt)).collect(),
            mlir: entry.mlir.clone(),
        });
    }

    let c_src = driver_source(prog, &bindings)?;
    let c_p = dir.join("driver.c");
    let bin_p = dir.join("driver");
    std::fs::write(&c_p, c_src).map_err(|e| e.to_string())?;
    let cc = find_tool("clang").unwrap_or_else(|| PathBuf::from("clang"));
    let mut link_args: Vec<&str> = vec!["-O2", c_p.to_str().expect("c path")];
    for o in &objects {
        link_args.push(o.to_str().expect("obj path"));
    }
    link_args.push("-o");
    link_args.push(bin_p.to_str().expect("bin path"));
    run_cmd(&cc, &link_args)?;

    let exec = Command::new(&bin_p)
        .output()
        .map_err(|e| format!("driver exec failed: {}", e))?;
    if !exec.status.success() {
        return Err(format!(
            "driver failed:\n{}",
            String::from_utf8_lossy(&exec.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&exec.stdout)
        .lines()
        .map(|l| l.to_string())
        .collect())
}

fn scratch() -> PathBuf {
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ontic-run-{}-{}", std::process::id(), n));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}



/// Thin command wrapper returning clean errors.
fn run_cmd(cc: &PathBuf, args: &[&str]) -> Result<(), String> {
    let out = Command::new(cc)
        .args(args)
        .output()
        .map_err(|e| format!("spawn {} failed: {}", cc.display(), e))?;
    if !out.status.success() {
        return Err(format!(
            "link failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{pipeline, sketch};

    /// End-to-end recipe gate (toolchain-gated): two hand-solved gens,
    /// one linear program, executed natively.
    #[test]
    fn test_recipe_end_to_end_native() {
        if find_tool("mlir-opt").is_none() || find_tool("llc").is_none() {
            eprintln!("toolchain missing; recipe e2e skipped");
            return;
        }
        let src = "\
fn Ledger.total(%items: List<Int>) -> Int
  wrapping
  => [1,2,3] -> 6

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
        let file = crate::recipe::parse_ont(src).expect("file parses");

        // Solve both gens by hand into an isolated vault.
        let vault_dir = scratch();
        let v = crate::vault::Vault::open(&vault_dir).expect("vault opens");
        let cands: &[(&str, &str)] = &[
            (
                "Ledger.total",
                "fn @total(%items: List<Int>) -> Int { fold %x in %items, %acc from 0 { %acc + %x } }",
            ),
            ("Twice", "fn @Twice(%n: Int) -> Int { %n * 2 }"),
        ];
        for (path, text) in cands {
            let w = file
                .gens
                .iter()
                .find(|w| w.path == *path)
                .expect("gen present");
            let cand = sketch::parse(text).expect("candidate parses");
            crate::check::check(&cand).expect("typechecks");
            let mlir = crate::lower::emit_fn(
                &cand.name,
                &cand.params,
                &cand.ret,
                &cand.body,
                w.wrapping,
                &crate::lower::CallMap::new(),
            )
            .expect("lowers");
            v.put(w, text, &mlir).expect("vaults");
        }

        let lines = run_in(&file, vault_dir.to_str().expect("dir")).expect("runs");
        assert_eq!(lines, vec!["6", "42"]);
    }
}

#[cfg(test)]
mod effect_tests {
    use super::*;
    use crate::vault::Vault;
    use crate::{pipeline, recipe};

    /// EG2 gate: write/dump/log effects produce deterministic driver output.
    #[test]
    fn test_effects_end_to_end() {
        if pipeline::find_tool("mlir-opt").is_none() || pipeline::find_tool("llc").is_none() {
            eprintln!("toolchain missing; effects e2e skipped");
            return;
        }
        let src = "\
fn Twice(%n: Int) -> Int
  => 21 -> 42

program FX
  use Twice
start
  %v = Twice(21)
  log \"computed %v\"
  write %v -> \"out.csv\"
  dump %v -> \"out.json\"
end
";
        let file = recipe::parse_ont(src).expect("parses");

        // Solve Twice into an isolated vault.
        let vault_dir = scratch();
        let v = Vault::open(&vault_dir).expect("vault opens");
        let w = &file.gens[0];
        let cand = crate::sketch::parse("fn @Twice(%n: Int) -> Int { %n * 2 }").unwrap();
        crate::check::check(&cand).unwrap();
        let mlir = crate::lower::emit_fn(
            &cand.name,
            &cand.params,
            &cand.ret,
            &cand.body,
            w.wrapping,
            &crate::lower::CallMap::new(),
        )
        .unwrap();
        v.put(w, "fn @Twice(%n: Int) -> Int { %n * 2 }", &mlir).unwrap();

        // Run from a working dir containing the effect outputs.
        let workdir = scratch();
        let bin_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(&workdir).unwrap();
        let lines = run_in(&file, vault_dir.to_str().unwrap()).expect("runs");
        std::env::set_current_dir(bin_dir).unwrap();

        assert_eq!(lines, vec!["computed 42"]);
        assert_eq!(
            std::fs::read_to_string(workdir.join("out.csv")).unwrap(),
            "42\n"
        );
        assert_eq!(
            std::fs::read_to_string(workdir.join("out.json")).unwrap(),
            "{\"v\": 42}\n"
        );
    }
}


#[cfg(test)]
mod ffi_tests {
    use super::*;
    use crate::{pipeline, recipe, vault::Vault};

    /// KG3 gate: generated header + shared library are consumable by a plain
    /// C caller with zero Ontic involvement at runtime.
    #[test]
    fn test_kernel_ffi_c_caller() {
        if pipeline::find_tool("mlir-opt").is_none() || pipeline::find_tool("llc").is_none() {
            eprintln!("toolchain missing; ffi gate skipped");
            return;
        }
        let dir = std::env::temp_dir().join(format!(
            "ontic-ffi-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let cand = crate::sketch::parse("fn @Twice(%n: Int) -> Int { %n * 2 }").unwrap();
        let mlir = crate::lower::emit_fn(
            &cand.name,
            &cand.params,
            &cand.ret,
            &cand.body,
            true,
            &crate::lower::CallMap::new(),
        )
        .unwrap();

        let so_path = dir.join("libTwice.so");
        pipeline::build_shared_so(&mlir, &so_path).expect("builds so");

        let header = crate::lower::emit_header(
            &cand.name,
            &cand.params,
            &cand.ret,
        )
        .unwrap();
        let h_path = dir.join("Twice.h");
        std::fs::write(&h_path, &header).unwrap();

        let caller = format!(
            "#include <stdio.h>\n#include \"{}\"\nint main(void) {{ printf(\"%ld\\n\", Twice(21)); return 0; }}\n",
            h_path.display()
        );
        let c_path = dir.join("caller.c");
        let bin = dir.join("caller");
        std::fs::write(&c_path, caller).unwrap();
        let cc = pipeline::find_tool("clang").unwrap_or_else(|| std::path::PathBuf::from("clang"));
        let out = std::process::Command::new(&cc)
            .arg(c_path.to_str().unwrap())
            .arg(format!("-L{}", dir.display()))
            .arg("-lTwice")
            .arg(format!("-Wl,-rpath,{}", dir.display()))
            .arg("-o")
            .arg(bin.to_str().unwrap())
            .output()
            .expect("caller compile spawns");
        assert!(out.status.success(), "link failed: {}", String::from_utf8_lossy(&out.stderr));
        let run = std::process::Command::new(&bin).output().expect("runs");
        assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "42");
    }
}
