//! Native pipeline: verified MLIR → LLVM IR → object → measured binary.
//!
//! Uses the system LLVM toolchain (mlir-opt, mlir-translate, llc, clang) via
//! subprocess. Tool discovery: ONTIC_MLIR_BIN dir override, then common
//! llvm-prefix dirs, then PATH. Every stage reports clean errors.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Per-invocation scratch dir uniquifier — parallel tests/processes must
/// never share staged files.
static SCRATCH_SEQ: AtomicUsize = AtomicUsize::new(0);

fn scratch_dir(tag: &str) -> PathBuf {
    let n = SCRATCH_SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("ontic-{}-{}-{}", tag, std::process::id(), n))
}

/// Public scratch-dir accessor for import-side builds in main.rs.
pub fn scratch_dir_pub(tag: &str) -> PathBuf {
    scratch_dir(tag)
}

/// C scalar type for a component kind.
fn kind_ctype(k: &CK) -> &'static str {
    match k {
        CK::I64 => "long",
        CK::F64 => "double",
        CK::F32 => "float",
        _ => "void*",
    }
}

fn kind_letter(k: &CK) -> char {
    match k {
        CK::I64 => 'l',
        CK::F64 => 'd',
        CK::F32 => 'f',
        _ => 'p',
    }
}

/// Deterministic C struct tag for a tuple return: ontic_tup<arity>_<letters>.
pub fn tuple_tag(kinds: &[CK]) -> String {
    let letters: String = kinds.iter().map(kind_letter).collect();
    format!("ontic_tup{}_{letters}", kinds.len())
}

/// C typedef for a tuple return: `typedef struct { T0 _0; ... } tag;`.
pub fn tuple_typedef(kinds: &[CK]) -> String {
    let fields: Vec<String> = kinds
        .iter()
        .enumerate()
        .map(|(i, k)| format!("{} _{};", kind_ctype(k), i))
        .collect();
    format!("typedef struct {{ {} }} {};", fields.join(" "), tuple_tag(kinds))
}

/// What a differential driver prints about the return value.
#[derive(Debug, Clone, PartialEq)]
pub enum RetSpec {
    I64,
    F64,
    /// memref<?xf64> descriptor returned by value; print first 4 elements.
    ListF64,
    /// Multi-value struct return; components printed space-separated.
    Tuple(Vec<CK>),
}

/// Harness-level ABI kind per function parameter / return.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CK {
    /// memref<?xi64> expanded flat (5 args)
    List,
    /// memref<?xf64> expanded flat (5 args)
    ListF64,
    /// memref<?xf32> expanded flat (5 args)
    ListF32,
    /// i64 scalar
    I64,
    /// f64 scalar
    F64,
    /// f32 scalar
    F32,
}

impl CK {
    fn proto(&self) -> &'static str {
        match self {
            CK::List | CK::ListF64 | CK::ListF32 => "void*, void*, long, long, long",
            CK::I64 => "long",
            CK::F64 => "double",
            CK::F32 => "float",
        }
    }
}

/// Candidate directories probed for toolchain binaries.
const TOOL_DIRS: &[&str] = &[
    "/usr/lib/llvm-18/bin",
    "/usr/local/bin",
    "/usr/bin",
];

/// Resolve a tool binary path; env override wins.
pub fn find_tool(name: &str) -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("ONTIC_MLIR_BIN") {
        let p = PathBuf::from(dir).join(name);
        if p.exists() {
            return Some(p);
        }
    }
    for dir in TOOL_DIRS {
        let p = PathBuf::from(dir).join(name);
        if p.exists() {
            return Some(p);
        }
    }
    which_on_path(name)
}

fn which_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var("PATH").ok()?;
    for dir in path.split(':') {
        let p = PathBuf::from(dir).join(name);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

fn run(tool: &PathBuf, args: &[&str], what: &str) -> Result<(), String> {
    let out = Command::new(tool)
        .args(args)
        .output()
        .map_err(|e| format!("{}: spawn {} failed: {}", what, tool.display(), e))?;
    if !out.status.success() {
        return Err(format!(
            "{} failed:\n{}",
            what,
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(())
}

/// Lower verified MLIR (arith/scf/memref/func module) all the way to LLVM IR
/// text via the canonical conversion chain.
pub fn mlir_to_llvmir(mlir_path: &std::path::Path, out_ll: &std::path::Path) -> Result<(), String> {
    let opt = find_tool("mlir-opt").ok_or("mlir-opt not found")?;
    let translate = find_tool("mlir-translate").ok_or("mlir-translate not found")?;
    // Ubuntu 18.1.3 pass names: scf->cf then finalize-memref/arith/index/
    // func/cf to llvm, reconcile the casts, then translate.
    run(
        &opt,
        &[
            mlir_path.to_str().ok_or("bad mlir path")?,
            "--convert-scf-to-cf",
            "--convert-math-to-llvm",
            "--finalize-memref-to-llvm",
            "--convert-arith-to-llvm",
            "--convert-index-to-llvm",
            "--convert-func-to-llvm",
            "--convert-cf-to-llvm",
            "--reconcile-unrealized-casts",
            "-o",
            out_ll.to_str().ok_or("bad ll path")?,
        ],
        "mlir-opt lowering to llvm dialect",
    )?;
    // Translate must never read and write the same path — mlir-translate
    // truncates lazily and segfaults on larger modules. Stage via temp file.
    let tmp_ll = out_ll.with_extension("ll.tmp");
    run(
        &translate,
        &[
            "--mlir-to-llvmir",
            out_ll.to_str().ok_or("bad ll path")?,
            "-o",
            tmp_ll.to_str().ok_or("bad tmp ll path")?,
        ],
        "mlir-translate to textual LLVM IR",
    )?;
    std::fs::rename(&tmp_ll, out_ll).map_err(|e| format!("rename {}: {}", tmp_ll.display(), e))?;
    // Middle-end: llc alone runs codegen only; clang-built references get
    // the full opt pipeline (unroll, reassociate, vectorize). Match that.
    let opt = find_tool("opt").unwrap_or_else(|| PathBuf::from("opt"));
    run(
        &opt,
        &[
            "-O3",
            "-S",
            out_ll.to_str().ok_or("bad ll path")?,
            "-o",
            out_ll.to_str().ok_or("bad ll path")?,
        ],
        "opt -O3 middle end",
    )?;
    Ok(())
}

/// Compile textual LLVM IR to an object file at -O3.
pub fn object_from_ll(ll_path: &std::path::Path, out_o: &std::path::Path) -> Result<(), String> {
    let llc = find_tool("llc").ok_or("llc not found")?;
    run(
        &llc,
        &[
            "-O3",
            "-filetype=obj",
            ll_path.to_str().ok_or("bad ll path")?,
            "-o",
            out_o.to_str().ok_or("bad obj path")?,
        ],
        "llc object emission",
    )
}

/// Validate raw MLIR text with mlir-opt (structural gate).
pub fn validate_mlir(mlir_path: &std::path::Path) -> Result<(), String> {
    let opt = find_tool("mlir-opt").ok_or("mlir-opt not found")?;
    run(
        &opt,
        &[mlir_path.to_str().ok_or("bad path")?, "-o", "/dev/null"],
        "mlir-opt validation",
    )
}

/// Build the C benchmark harness source for one candidate function.
///
/// ABI note: `finalize-memref-to-llvm` expands a single dynamic-dim memref
/// argument FLAT into five scalars: (allocated*, aligned*, offset, size,
/// stride). Verified against emitted objects this session. Scalar sketch
/// params pass as plain longs. The loop accumulates results into `acc`
/// (printed, so it survives dead-code elimination); the binary times itself
/// with CLOCK_MONOTONIC and prints `<total_ns> <acc>`.
pub fn bench_c_source(fn_name: &str, kinds: &[CK], iters: usize, ret_kinds: &[CK]) -> String {
    let tup_tag = if ret_kinds.is_empty() {
        String::new()
    } else {
        tuple_tag(ret_kinds)
    };
    let tup_typedef_text = if ret_kinds.is_empty() {
        String::new()
    } else {
        tuple_typedef(ret_kinds)
    };
    let ret_decl = if ret_kinds.is_empty() {
        "long".to_string()
    } else {
        tup_tag.clone()
    };
    let mut proto = String::new();
    let mut decls = String::new();
    let mut init = String::new();
    let mut call_args = String::new();
    for (i, k) in kinds.iter().enumerate() {
        if !proto.is_empty() {
            proto.push_str(", ");
        }
        proto.push_str(k.proto());
        match k {
            CK::List | CK::ListF64 => {
                let t = if matches!(k, CK::List) { "long" } else { "double" };
                decls.push_str(&format!("  {}* b{} = malloc(N * sizeof({}));\n", t, i, t));
                init.push_str(&format!(
                    "    for (long i = 0; i < N; i++) b{0}[i] = ({1})(i % 97);\n",
                    i, t
                ));
                call_args.push_str(&format!("b{}, b{}, 0, N, 1, ", i, i));
            }
            CK::I64 => {
                decls.push_str(&format!("  long s{} = 3;\n", i));
                call_args.push_str(&format!("s{}, ", i));
            }
            CK::F64 => {
                decls.push_str(&format!("  double s{} = 3.0;\n", i));
                call_args.push_str(&format!("s{}, ", i));
            }
            CK::F32 => {
                decls.push_str(&format!("  float s{} = 3.0f;\n", i));
                call_args.push_str(&format!("s{}, ", i));
            }
            CK::ListF32 => {
                decls.push_str(&format!("  float b{0}[] = {{0.f}};\n", i));
                init.push_str("    b0[0] = 0.f;\n");
                call_args.push_str(&format!("b{0}, b{0}, 0, N, 1, ", i));
            }
        }
    }
    let tail = call_args.trim_end_matches(", ");
    let acc_stmt = if ret_kinds.is_empty() {
        format!("acc += {fn_name}({tail});")
    } else {
        let acc_t = kind_ctype(&ret_kinds[0]);
        format!(
            "{{ {tup_tag} r = {fn_name}({tail}); acc += ({acc_t})r._0; }}"
        )
    };
    format!(
        r#"#include <stdio.h>
#include <stdlib.h>
#include <time.h>

{tup_typedef_text}
extern {ret_decl} {fname}({proto});

long ontic_trap(void) {{
  extern void abort(void);
  abort();
}}

int main(void) {{
  const long N = 1024;
  const long ITERS = {iters};
{decls}
{init}
  struct timespec t0, t1;
  long acc = 0;
  clock_gettime(CLOCK_MONOTONIC, &t0);
  for (long k = 0; k < ITERS; k++) {{
    {acc_stmt}
  }}
  clock_gettime(CLOCK_MONOTONIC, &t1);
  long ns = (t1.tv_sec - t0.tv_sec) * 1000000000L + (t1.tv_nsec - t0.tv_nsec);
  printf("%ld %ld\n", ns, acc);
  return acc == 42 ? 1 : 0;
}}
"#,
        fname = fn_name,
        proto = proto,
        ret_decl = ret_decl,
        acc_stmt = acc_stmt,
        tup_typedef_text = tup_typedef_text,
        iters = iters,
        decls = decls,
        init = init,
    )
}

/// Build a C driver that calls the function once on FIXED inputs and prints
/// the result (%.17g). Used by differential tests: interpreter and native
/// must agree bit-for-bit.
pub fn eval_c_source(
    fn_name: &str,
    kinds: &[CK],
    list_vals: &[i64],
    list_f64_vals: &[f64],
    scalars_i64: &[i64],
    scalars_f64: &[f64],
    ret: RetSpec,
) -> Result<String, String> {
    let mut proto = String::new();
    let mut decls = String::new();
    let mut call_args = String::new();
    let (mut li, mut si, mut sf) = (0usize, 0usize, 0usize);
    for k in kinds {
        if !proto.is_empty() {
            proto.push_str(", ");
        }
        proto.push_str(k.proto());
        match k {
            CK::List => {
                let vals: Vec<String> = list_vals.iter().map(|v| v.to_string()).collect();
                decls.push_str(&format!(
                    "  long b{0}[] = {{{1}}};\n",
                    li,
                    vals.join(", ")
                ));
                call_args.push_str(&format!(
                    "b{0}, b{0}, 0, {1}, 1, ",
                    li,
                    list_vals.len()
                ));
                li += 1;
            }
            CK::ListF64 => {
                let vals: Vec<String> = list_f64_vals.iter().map(|v| format!("{:e}", v)).collect();
                decls.push_str(&format!(
                    "  double d{0}[] = {{{1}}};\n",
                    li,
                    vals.join(", ")
                ));
                call_args.push_str(&format!(
                    "d{0}, d{0}, 0, {1}, 1, ",
                    li,
                    list_f64_vals.len()
                ));
                li += 1;
            }
            CK::I64 => {
                decls.push_str(&format!("  long s{} = {}L;\n", si, scalars_i64[si]));
                call_args.push_str(&format!("s{}, ", si));
                si += 1;
            }
            CK::F64 => {
                decls.push_str(&format!("  double f{} = {:e};\n", sf, scalars_f64[sf]));
                call_args.push_str(&format!("f{}, ", sf));
                sf += 1;
            }
            CK::F32 | CK::ListF32 => {
                return Err("F32 drivers not wired into differential eval yet".to_string())
            }
        }
    }
    let call_args_tail = call_args.trim_end_matches(", ");
    let tuple_kinds: Vec<CK> = match &ret {
        RetSpec::Tuple(ks) => ks.clone(),
        _ => Vec::new(),
    };
    let (ret_t, fmt) = match ret {
        RetSpec::I64 => ("long".to_string(), "%ld".to_string()),
        RetSpec::F64 => ("double".to_string(), "%.17g".to_string()),
        RetSpec::ListF64 => ("MR".to_string(), String::new()),
        RetSpec::Tuple(_) => (tuple_tag(&tuple_kinds), String::new()),
    };
    let mr_def = "typedef struct { void* base; void* data; long off; long size; long stride; } MR;";
    let tup_def = if matches!(ret, RetSpec::Tuple(_)) {
        tuple_typedef(&tuple_kinds)
    } else {
        String::new()
    };
    let body = match ret {
        RetSpec::ListF64 => format!(
            "  MR r = {fname}({args});\n  long n = r.size < 4 ? r.size : 4;\n  printf(\"%ld\", n);\n  double* p = (double*)r.data;\n  for (long i = 0; i < n; i++) printf(\" %.17g\", p[i]);\n  printf(\"\\n\");",
            fname = fn_name,
            args = call_args_tail
        ),
        RetSpec::Tuple(_) => {
            let prints: Vec<String> = (0..tuple_kinds.len())
                .map(|i| {
                    let f = match tuple_kinds[i] {
                        CK::I64 => "%ld",
                        CK::F32 => "%.9g",
                        _ => "%.17g",
                    };
                    format!("  printf(\" {f}\", r._{i});")
                })
                .collect();
            format!(
                "  {tag} r = {fname}({args});\n  printf(\"{arity}\");\n{prints}\n  printf(\"\\n\");",
                tag = ret_t,
                fname = fn_name,
                args = call_args_tail,
                arity = tuple_kinds.len(),
                prints = prints.join("\n"),
            )
        }
        _ => format!(
            "  {ret_t} v = {fname}({args});\n  printf(\"{fmt}\\n\", v);",
            ret_t = ret_t,
            fmt = fmt,
            fname = fn_name,
            args = call_args_tail
        ),
    };
    Ok(format!(
        r#"#include <stdio.h>
#include <stdlib.h>

{mr_def}
{tup_def}

extern {ret_t} {fname}({proto});

long ontic_trap(void) {{
  extern void abort(void);
  abort();
}}

int main(void) {{
{decls}
{body}
  return 0;
}}
"#,
        mr_def = mr_def,
        tup_def = tup_def,
        ret_t = ret_t,
        fname = fn_name,
        proto = proto,
        decls = decls,
        body = body,
    ))
}

/// Run the function once natively; returns the parsed numeric result
/// (integer results arrive as exact f64).
pub fn eval_native(
    mlir_text: &str,
    fn_name: &str,
    kinds: &[CK],
    list_vals: &[i64],
    list_f64_vals: &[f64],
    scalars_i64: &[i64],
    scalars_f64: &[f64],
    ret: RetSpec,
    extra_mls: &[String],
) -> Result<Vec<f64>, String> {
    let dir = scratch_dir("eval");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mlir_p = dir.join("cand.mlir");
    let ll_mlir = dir.join("cand_llvm.mlir");
    let o_p = dir.join("cand.o");
    let c_p = dir.join("eval.c");
    let bin_p = dir.join("eval");

    std::fs::write(&mlir_p, mlir_text).map_err(|e| e.to_string())?;
    mlir_to_llvmir(&mlir_p, &ll_mlir)?;
    object_from_ll(&ll_mlir, &o_p)?;
    let mut objects = vec![o_p.clone()];
    for (i, extra) in extra_mls.iter().enumerate() {
        let ep_mlir = dir.join(format!("extra{}.mlir", i));
        let ep_ll = dir.join(format!("extra{}_llvm.mlir", i));
        let ep_o = dir.join(format!("extra{}.o", i));
        std::fs::write(&ep_mlir, extra).map_err(|e| e.to_string())?;
        mlir_to_llvmir(&ep_mlir, &ep_ll)?;
        object_from_ll(&ep_ll, &ep_o)?;
        objects.push(ep_o);
    }
    let c_text = eval_c_source(
        fn_name,
        kinds,
        list_vals,
        list_f64_vals,
        scalars_i64,
        scalars_f64,
        ret,
    )?;
    std::fs::write(&c_p, c_text).map_err(|e| e.to_string())?;
    let cc = find_tool("clang").unwrap_or_else(|| PathBuf::from("clang"));
    let mut link_args: Vec<&str> =
        vec!["-O2", c_p.to_str().ok_or("bad c path")?];
    for o in &objects {
        link_args.push(o.to_str().ok_or("bad obj path")?);
    }
    link_args.push("-o");
    link_args.push(bin_p.to_str().ok_or("bad bin path")?);
    run(&cc, &link_args, "differential link")?;
    let out = Command::new(&bin_p)
        .output()
        .map_err(|e| format!("differential exec failed: {}", e))?;
    if !out.status.success() {
        return Err(format!(
            "differential exec failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let vals: Result<Vec<f64>, _> = text
        .split_whitespace()
        .map(|t| t.parse::<f64>())
        .collect();
    vals.map_err(|_| format!("bad differential output `{}`", text))
}

/// Full native measurement: write mlir, lower, emit object, build harness,
/// run ROUNDS times, return median ns-per-call as measured inside the binary.
pub fn bench_native(
    mlir_text: &str,
    fn_name: &str,
    kinds: &[CK],
    iters: usize,
    extra_mls: &[String],
    ret_kinds: &[CK],
) -> Result<u64, String> {
    let dir = scratch_dir("bench");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mlir_p = dir.join("cand.mlir");
    let ll_mlir = dir.join("cand_llvm.mlir");
    let o_p = dir.join("cand.o");
    let c_p = dir.join("bench.c");
    let bin_p = dir.join("bench");

    std::fs::write(&mlir_p, mlir_text).map_err(|e| e.to_string())?;
    mlir_to_llvmir(&mlir_p, &ll_mlir)?;
    object_from_ll(&ll_mlir, &o_p)?;
    std::fs::write(&c_p, bench_c_source(fn_name, kinds, iters, ret_kinds))
        .map_err(|e| e.to_string())?;
    let cc = find_tool("clang").unwrap_or_else(|| PathBuf::from("clang"));
    // Extra dependency modules compiled alongside the candidate.
    let mut dep_objs: Vec<PathBuf> = Vec::new();
    for (i, extra) in extra_mls.iter().enumerate() {
        let ep_mlir = dir.join(format!("dep{}.mlir", i));
        let ep_ll = dir.join(format!("dep{}_llvm.mlir", i));
        let ep_o = dir.join(format!("dep{}.o", i));
        std::fs::write(&ep_mlir, extra).map_err(|e| e.to_string())?;
        mlir_to_llvmir(&ep_mlir, &ep_ll)?;
        object_from_ll(&ep_ll, &ep_o)?;
        dep_objs.push(ep_o);
    }
    let mut link_args: Vec<&str> =
        vec!["-O2", c_p.to_str().ok_or("bad c path")?, o_p.to_str().ok_or("bad obj path")?];
    for o in &dep_objs {
        link_args.push(o.to_str().ok_or("bad dep obj")?);
    }
    link_args.push("-o");
    link_args.push(bin_p.to_str().ok_or("bad bin path")?);
    run(&cc, &link_args, "harness link")?;

    const ROUNDS: usize = 9;
    let mut samples: Vec<u64> = Vec::new();
    for _ in 0..ROUNDS {
        let out = Command::new(&bin_p)
            .output()
            .map_err(|e| format!("bench exec failed: {}", e))?;
        if !out.status.success() {
            return Err(format!(
                "bench exec failed: {}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let ns = text
            .split_whitespace()
            .next()
            .and_then(|v| v.parse::<u64>().ok())
            .ok_or_else(|| format!("bad bench output `{}`", text.trim()))?;
        samples.push(ns / iters.max(1) as u64);
    }
    samples.sort_unstable();
    Ok(samples[samples.len() / 2])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{check, interp, lower, sketch};
    use crate::gen::Value;

    const SUM_SRC: &str =
        "fn @total(%items: List<Int>) -> Int { fold %x in %items, %acc from 0 { %acc + %x } }";

    /// Differential gate (W1b): interpreter and native object must agree
    /// bit-for-bit. Skips cleanly when the toolchain is absent.
    #[test]
    fn test_interpreter_native_bit_parity() {
        if find_tool("mlir-opt").is_none() || find_tool("llc").is_none() {
            eprintln!("toolchain missing; differential parity skipped");
            return;
        }
        let cand = sketch::parse(SUM_SRC).unwrap();
        check::check(&cand).unwrap();
        let mlir = lower::emit_fn(&cand.name, &cand.params, &cand.ret, &cand.body, &lower::CallMap::new()).unwrap();

        let inputs = vec![Value::List(vec![3, 1, 4, 1, 5, 9, 2, 6])];
        let expect = interp::eval_candidate(&cand, &inputs, &interp::Ctx::checked())
            .expect("interp evaluates");
        let got = eval_native(
            &mlir,
            &cand.name,
            &[CK::List],
            &[3, 1, 4, 1, 5, 9, 2, 6],
            &[],
            &[],
            &[],
            RetSpec::I64,
            &[],
        )
        .expect("native evaluates");
        assert_eq!(got, vec![31.0], "sanity");
        assert_eq!(got[0], match expect {
            Value::Int(v) => v as f64,
            other => panic!("unexpected {:?}", other),
        });
    }

    #[test]
    fn test_bench_harness_source_shape() {
        let c = bench_c_source("f", &[CK::List, CK::I64], 10, &[]);
        assert!(c.contains("extern long f(void*, void*, long, long, long, long);"));
        assert!(c.contains("b0, b0, 0, N, 1, s1"));
        let c_scalar_only = bench_c_source("g", &[CK::I64], 5, &[]);
        assert!(c_scalar_only.contains("extern long g(long);"));
    }
}

#[cfg(test)]
mod trap_tests {
    use super::*;
    use crate::{check, interp, lower, sketch};
    use crate::gen::Value;

    /// Checked-tier honesty gate: native traps exactly where the interpreter
    /// kills. Overflowing inputs must fail natively; clean inputs agree.
    #[test]
    fn test_checked_tier_native_trap_matches_interpreter() {
        if find_tool("mlir-opt").is_none() || find_tool("llc").is_none() {
            eprintln!("toolchain missing; trap differential skipped");
            return;
        }
        let cand = sketch::parse("fn @f(%items: List<Int>) -> Int { fold %x in %items, %acc from 0 { %acc + %x } }").unwrap();
        check::check(&cand).unwrap();
        let mlir = lower::emit_fn(&cand.name, &cand.params, &cand.ret, &cand.body, &lower::CallMap::new()).unwrap();

        // Clean inputs: both tiers agree.
        let got =
            eval_native(&mlir, "f", &[CK::List], &[3, 4, 5], &[], &[], &[], RetSpec::I64, &[])
                .expect("clean runs");
        assert_eq!(got, vec![12.0]);

        // Overflowing inputs: interpreter kills...
        let killed = interp::eval_candidate(
            &cand,
            &[Value::List(vec![i64::MAX, 1])],
            &interp::Ctx::checked(),
        );
        assert!(killed.is_err());
        // ...and native must not return a value either.
        assert!(
            eval_native(&mlir, "f", &[CK::List], &[i64::MAX, 1], &[], &[], &[], RetSpec::I64, &[]).is_err(),
            "native returned a value where the oracle kills"
        );
    }
}

#[cfg(test)]
mod float_tests {
    use super::*;
    use crate::{check, interp, lower, sketch};
    use crate::gen::Value;

    /// P1 gate: F64 candidates lower to arith.mulf/cmpf-style IR (never the
    /// integer trap path) and match the interpreter bit-for-bit natively.
    #[test]
    fn test_f64_native_parity_and_no_trap_expansion() {
        if find_tool("mlir-opt").is_none() || find_tool("llc").is_none() {
            eprintln!("toolchain missing; f64 parity skipped");
            return;
        }
        let cand = sketch::parse("fn @m(%a: F64, %b: F64) -> F64 { %a * %b + %a }").unwrap();
        crate::check::check(&cand).unwrap();
        // Checked tier must NOT wrap float math in i128 checks.
        let mlir = lower::emit_fn(&cand.name, &cand.params, &cand.ret, &cand.body, &lower::CallMap::new()).unwrap();
        assert!(!mlir.contains("i128"), "float math entered trap expansion");
        assert!(mlir.contains("arith.mulf"));

        let expect = interp::eval_candidate(
            &cand,
            &[Value::Float(1.5), Value::Float(2.5)],
            &interp::Ctx::checked(),
        )
        .unwrap();
        let got = eval_native(
            &mlir,
            "m",
            &[CK::F64, CK::F64],
            &[],
            &[],
            &[],
            &[1.5, 2.5],
            RetSpec::F64,
            &[],
        )
        .unwrap();
        match expect {
            Value::Float(f) => assert_eq!(got, vec![f]),
            other => panic!("unexpected {:?}", other),
        }
    }
}

#[cfg(test)]
mod listf64_tests {
    use super::*;
    use crate::{check, interp, lower, sketch};
    use crate::gen::Value;

    /// Layer B gate: fold over List<F64> matches the oracle bit-for-bit.
    #[test]
    fn test_float_list_fold_native_parity() {
        if find_tool("mlir-opt").is_none() || find_tool("llc").is_none() {
            eprintln!("toolchain missing; f64-list parity skipped");
            return;
        }
        let cand = sketch::parse(
            "fn @dot(%xs: List<F64>) -> F64 { fold %x in %xs, %acc from 0.0 { %acc + %x * 2.0 } }",
        )
        .unwrap();
        crate::check::check(&cand).unwrap();
        let mlir =
            lower::emit_fn(&cand.name, &cand.params, &cand.ret, &cand.body, &lower::CallMap::new()).unwrap();
        assert!(mlir.contains("memref<?xf64>"), "param type not f64");
        assert!(mlir.contains("memref.load") && mlir.contains("arith.addf"));

        let inputs = vec![Value::FloatList(vec![1.5, 2.0, -0.5])];
        let expect = interp::eval_candidate(&cand, &inputs, &interp::Ctx::checked()).unwrap();
        let got = eval_native(
            &mlir,
            "dot",
            &[CK::ListF64],
            &[],
            &[1.5, 2.0, -0.5],
            &[],
            &[],
            RetSpec::F64,
            &[],
        )
        .unwrap();
        match expect {
            Value::Float(f) => assert_eq!(got, vec![f]),
            other => panic!("unexpected {:?}", other),
        }
    }
}

#[cfg(test)]
mod broadcast_tests {
    use super::*;
    use crate::{check, interp, lower, sketch};
    use crate::gen::Value;

    /// P2 gate: a broadcasting function returns a memref descriptor natively;
    /// elements must match the oracle elementwise.
    #[test]
    fn test_broadcast_native_parity() {
        if find_tool("mlir-opt").is_none() || find_tool("llc").is_none() {
            eprintln!("toolchain missing; broadcast parity skipped");
            return;
        }
        let cand = sketch::parse(
            "fn @scale(%xs: List<F64>) -> List<F64> { %xs * 3.0 + 1.0 }",
        )
        .unwrap();
        crate::check::check(&cand).unwrap();
        let mlir = lower::emit_fn(&cand.name, &cand.params, &cand.ret, &cand.body, &lower::CallMap::new())
            .expect("lowers");
        assert!(mlir.contains("memref.alloc"), "no result alloc");
        assert!(mlir.contains("arith.mulf"), "no elementwise mulf");

        let expect = interp::eval_candidate(
            &cand,
            &[Value::FloatList(vec![1.0, 2.0])],
            &interp::Ctx::checked(),
        )
        .unwrap();
        let got = eval_native(
            &mlir,
            "scale",
            &[CK::ListF64],
            &[],
            &[1.0, 2.0],
            &[],
            &[],
            RetSpec::ListF64,
            &[],
        )
        .expect("native runs");
        match expect {
            Value::FloatList(fs) => {
                // Driver prints count then up to 4 elements.
                assert_eq!(got[0] as usize, fs.len());
                for (g, f) in got[1..].iter().zip(fs.iter()) {
                    assert_eq!(g, f);
                }
            }
            other => panic!("unexpected {:?}", other),
        }
    }
}


/// Compile a (composite) MLIR module into a self-contained shared library.
/// The module must already contain every dependency function.
pub fn build_shared_so(composite_mlir: &str, out_so: &Path) -> Result<(), String> {
    let dir = scratch_dir("so");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mlir_p = dir.join("composite.mlir");
    let _ = &dir;
    let _ = &mlir_p;
    let ll_mlir = dir.join("composite_llvm.mlir");
    let o_p = dir.join("composite.o");
    std::fs::write(&mlir_p, composite_mlir)
        .map_err(|e| format!("write {}: {}", mlir_p.display(), e))?;
    mlir_to_llvmir(&mlir_p, &ll_mlir)
        .map_err(|e| format!("lower-to-llvm: {}", e))?;
    object_from_ll(&ll_mlir, &o_p)
        .map_err(|e| format!("object: {}", e))?;
    // Trap stub: provides ontic_trap/ontic_trapf definitions.
    let trap_c = dir.join("trap.c");
    std::fs::write(&trap_c, "#include <stdlib.h>\nlong ontic_trap(void) { abort(); }\ndouble ontic_trapf(void) { abort(); }\n").map_err(|e| e.to_string())?;
    let cc = find_tool("clang").unwrap_or_else(|| PathBuf::from("clang"));
    run(
        &cc,
        &[
            "-shared",
            "-O2",
            o_p.to_str().ok_or("bad obj")?,
            trap_c.to_str().ok_or("bad trap")?,
            "-o",
            out_so.to_str().ok_or("bad so")?,
        ],
        "shared link",
    )
}

/// Link a compiled object plus optional C sources into a shared library.
/// C sources are precompiled `-fPIC` first (TLS state in shims requires
/// it). Shared by solve-time builds and package imports; the trap stub
/// provides `ontic_trap` / `ontic_trapf` definitions.
pub fn link_shared_so(obj: &Path, c_sources: &[&Path], out_so: &Path) -> Result<(), String> {
    let dir = scratch_dir("so_link");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let cc = find_tool("clang")
        .or_else(|| find_tool("cc"))
        .ok_or("no C compiler found (clang/cc)")?;
    let mut objs: Vec<PathBuf> = vec![obj.to_path_buf()];
    for (i, src) in c_sources.iter().enumerate() {
        let o = dir.join(format!("src_{i}.o"));
        run(
            &cc,
            &[
                "-c",
                "-fPIC",
                "-O2",
                src.to_str().ok_or("bad c source path")?,
                "-o",
                o.to_str().ok_or("bad temp obj")?,
            ],
            "pic compile",
        )?;
        objs.push(o);
    }
    let trap_c = dir.join("trap.c");
    std::fs::write(
        &trap_c,
        "#include <stdlib.h>\nlong ontic_trap(void) { abort(); }\ndouble ontic_trapf(void) { abort(); }\n",
    )
    .map_err(|e| e.to_string())?;
    let trap_o = dir.join("trap.o");
    run(
        &cc,
        &[
            "-c",
            "-fPIC",
            "-O2",
            trap_c.to_str().ok_or("bad trap")?,
            "-o",
            trap_o.to_str().ok_or("bad trap obj")?,
        ],
        "trap compile",
    )?;
    objs.push(trap_o);
    let args: Vec<String> = std::iter::once("-shared".to_string())
        .chain(std::iter::once("-O2".to_string()))
        .chain(objs.iter().map(|p| p.to_string_lossy().into_owned()))
        .chain(std::iter::once("-o".to_string()))
        .chain(std::iter::once(out_so.to_string_lossy().into_owned()))
        .collect();
    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    run(&cc, &arg_refs, "shared link")
}

/// Build a guarded shared library: rename the MLIR kernel to `__raw`, compile
/// the C shim that owns the public ABI symbol, and link everything into
/// `out_so`.  The shim source is returned for vault reproducibility.
pub fn build_shared_so_guarded(
    composite_mlir: &str,
    shim_source: &str,
    out_so: &Path,
) -> Result<String, String> {
    let dir = scratch_dir("so_guarded");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    // Rename kernel: func.func @<name>(...)  →  func.func @<name>__raw(...)
    let guarded_mlir = composite_mlir
        .lines()
        .map(|line| {
            if line.trim_start().starts_with("func.func @") {
                // Replace the first @name with @name__raw
                if let Some(at_pos) = line.find('@') {
                    let after_at = &line[at_pos + 1..];
                    if let Some(paren_pos) = after_at.find('(') {
                        let name = &after_at[..paren_pos];
                        if !name.ends_with("__raw") {
                            return format!(
                                "{}@{}__raw{}",
                                &line[..at_pos],
                                name,
                                &after_at[paren_pos..]
                            );
                        }
                    }
                }
            }
            line.to_string()
        })
        .collect::<Vec<_>>()
        .join("\n");

    let mlir_p = dir.join("composite_guarded.mlir");
    let ll_mlir = dir.join("composite_guarded_llvm.mlir");
    let o_p = dir.join("composite_guarded.o");
    let shim_c = dir.join("shim.c");
    let shim_o = dir.join("shim.o");

    std::fs::write(&mlir_p, &guarded_mlir)
        .map_err(|e| format!("write {}: {}", mlir_p.display(), e))?;
    std::fs::write(&shim_c, shim_source)
        .map_err(|e| format!("write shim: {}", e))?;

    mlir_to_llvmir(&mlir_p, &ll_mlir)
        .map_err(|e| format!("guarded lower-to-llvm: {}", e))?;
    object_from_ll(&ll_mlir, &o_p)
        .map_err(|e| format!("guarded object: {}", e))?;

    let cc = find_tool("clang").unwrap_or_else(|| PathBuf::from("clang"));
    // Compile shim object
    run(
        &cc,
        &["-fPIC", "-c", shim_c.to_str().ok_or("bad shim")?, "-o", shim_o.to_str().ok_or("bad shim_o")?],
        "guarded shim compile",
    )?;

    // Trap stub
    let trap_c = dir.join("trap.c");
    std::fs::write(
        &trap_c,
        "#include <stdlib.h>\nlong ontic_trap(void) { abort(); }\ndouble ontic_trapf(void) { abort(); }\n",
    )
    .map_err(|e| e.to_string())?;

    // Link: kernel .o + shim .o + trap stub
    run(
        &cc,
        &[
            "-shared",
            "-O2",
            "-lm",
            "-lpthread",
            o_p.to_str().ok_or("bad obj")?,
            shim_o.to_str().ok_or("bad shim_o")?,
            trap_c.to_str().ok_or("bad trap")?,
            "-o",
            out_so.to_str().ok_or("bad so")?,
        ],
        "guarded shared link",
    )?;

    Ok(shim_source.to_string())
}
