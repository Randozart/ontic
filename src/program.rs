//! Program assembly: recipe → C driver over verified MLIR objects.
//!
//! The glue is generated C calling flat-MemRef-ABI functions (see
//! pipeline.rs ABI note). Every callee must already be vault-verified;
//! `ontic run` never executes unsieved code.

use crate::pipeline::{self, find_tool};
use crate::recipe::{CallArg, OntFile, Stmt};
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
                    crate::wish::Value::List(vs) => {
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
                    crate::wish::Value::Int(v) => {
                        let cn = format!("v{}", seq);
                        seq += 1;
                        body.push_str(&format!("  long {} = {}L;\n", cn, v));
                        locals.push((name.clone(), Local::Scalar { c_name: cn }));
                    }
                    crate::wish::Value::Float(v) => {
                        let cn = format!("v{}", seq);
                        seq += 1;
                        body.push_str(&format!("  double {} = {:e};\n", cn, v));
                        locals.push((name.clone(), Local::ScalarF { c_name: cn }));
                    }
                    crate::wish::Value::Bool(b) => {
                        let cn = format!("v{}", seq);
                        seq += 1;
                        body.push_str(&format!("  long {} = {}L;\n", cn, *b as i64));
                        locals.push((name.clone(), Local::Scalar { c_name: cn }));
                    }
                    crate::wish::Value::FloatList(vs) => {
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
                            crate::wish::Value::Int(v) => {
                                call_args.push_str(&format!("{}L, ", v))
                            }
                            crate::wish::Value::Float(v) => {
                                call_args.push_str(&format!("{:e}, ", v))
                            }
                            crate::wish::Value::Bool(b) => {
                                call_args.push_str(&format!("{}L, ", *b as i64))
                            }
                            crate::wish::Value::List(_)
                            | crate::wish::Value::FloatList(_) => {
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
            .wishes
            .iter()
            .find(|w| &w.path == dep_path)
            .ok_or_else(|| format!("dependency `{}` not among same-file wishes", dep_path))?;
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

    /// End-to-end recipe gate (toolchain-gated): two hand-solved wishes,
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

        // Solve both wishes by hand into an isolated vault.
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
                .wishes
                .iter()
                .find(|w| w.path == *path)
                .expect("wish present");
            let cand = sketch::parse(text).expect("candidate parses");
            crate::check::check(&cand).expect("typechecks");
            let mlir =
                crate::lower::emit_fn(&cand.name, &cand.params, &cand.ret, &cand.body, w.wrapping)
                    .expect("lowers");
            v.put(w, text, &mlir).expect("vaults");
        }

        let lines = run_in(&file, vault_dir.to_str().expect("dir")).expect("runs");
        assert_eq!(lines, vec!["6", "42"]);
    }
}
