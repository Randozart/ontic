//! Native pipeline: verified MLIR → LLVM IR → object → measured binary.
//!
//! Uses the system LLVM toolchain (mlir-opt, mlir-translate, llc, clang) via
//! subprocess. Tool discovery: ONTIC_MLIR_BIN dir override, then common
//! llvm-prefix dirs, then PATH. Every stage reports clean errors.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Per-invocation scratch dir uniquifier — parallel tests/processes must
/// never share staged files.
static SCRATCH_SEQ: AtomicUsize = AtomicUsize::new(0);

fn scratch_dir(tag: &str) -> PathBuf {
    let n = SCRATCH_SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("ontic-{}-{}-{}", tag, std::process::id(), n))
}

/// Harness-level ABI kind per function parameter / return.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CK {
    /// memref<?xi64> expanded flat (5 args)
    List,
    /// memref<?xf64> expanded flat (5 args)
    ListF64,
    /// i64 scalar
    I64,
    /// f64 scalar
    F64,
}

impl CK {
    fn proto(&self) -> &'static str {
        match self {
            CK::List | CK::ListF64 => "void*, void*, long, long, long",
            CK::I64 => "long",
            CK::F64 => "double",
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
    run(
        &translate,
        &[
            "--mlir-to-llvmir",
            out_ll.to_str().ok_or("bad ll path")?,
            "-o",
            out_ll.to_str().ok_or("bad ll path")?,
        ],
        "mlir-translate to textual LLVM IR",
    )?;
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
pub fn bench_c_source(fn_name: &str, kinds: &[CK], iters: usize) -> String {
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
        }
    }
    format!(
        r#"#include <stdio.h>
#include <stdlib.h>
#include <time.h>

extern long {fname}({proto});

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
    acc += {fname}({call_args_tail});
  }}
  clock_gettime(CLOCK_MONOTONIC, &t1);
  long ns = (t1.tv_sec - t0.tv_sec) * 1000000000L + (t1.tv_nsec - t0.tv_nsec);
  printf("%ld %ld\n", ns, acc);
  return acc == 42 ? 1 : 0;
}}
"#,
        fname = fn_name,
        proto = proto,
        iters = iters,
        decls = decls,
        init = init,
        call_args_tail = call_args.trim_end_matches(", "),
    )
}

/// Build a C driver that calls the function once on FIXED inputs and prints
/// the result (%.17g). Used by differential tests: interpreter and native
/// must agree bit-for-bit under the wrapping tier.
pub fn eval_c_source(
    fn_name: &str,
    kinds: &[CK],
    list_vals: &[i64],
    list_f64_vals: &[f64],
    scalars_i64: &[i64],
    scalars_f64: &[f64],
    ret_f64: bool,
) -> String {
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
        }
    }
    let ret_t = if ret_f64 { "double" } else { "long" };
    let fmt = if ret_f64 { "%.17g" } else { "%ld" };
    format!(
        r#"#include <stdio.h>

extern {ret_t} {fname}({proto});

long ontic_trap(void) {{
  extern void abort(void);
  abort();
}}

int main(void) {{
{decls}
  printf("{fmt}\n", {fname}({call_args_tail}));
  return 0;
}}
"#,
        ret_t = ret_t,
        fname = fn_name,
        proto = proto,
        decls = decls,
        fmt = fmt,
        call_args_tail = call_args.trim_end_matches(", "),
    )
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
    ret_f64: bool,
) -> Result<f64, String> {
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
    std::fs::write(
        &c_p,
        eval_c_source(
            fn_name,
            kinds,
            list_vals,
            list_f64_vals,
            scalars_i64,
            scalars_f64,
            ret_f64,
        ),
    )
    .map_err(|e| e.to_string())?;
    let cc = find_tool("clang").unwrap_or_else(|| PathBuf::from("clang"));
    run(
        &cc,
        &[
            "-O2",
            c_p.to_str().ok_or("bad c path")?,
            o_p.to_str().ok_or("bad obj path")?,
            "-o",
            bin_p.to_str().ok_or("bad bin path")?,
        ],
        "differential link",
    )?;
    let out = Command::new(&bin_p)
        .output()
        .map_err(|e| format!("differential exec failed: {}", e))?;
    if !out.status.success() {
        return Err(format!(
            "differential exec failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse::<f64>()
        .map_err(|_| "bad differential output".to_string())
}

/// Full native measurement: write mlir, lower, emit object, build harness,
/// run ROUNDS times, return median ns-per-call as measured inside the binary.
pub fn bench_native(
    mlir_text: &str,
    fn_name: &str,
    kinds: &[CK],
    iters: usize,
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
    std::fs::write(&c_p, bench_c_source(fn_name, kinds, iters))
        .map_err(|e| e.to_string())?;
    let cc = find_tool("clang").unwrap_or_else(|| PathBuf::from("clang"));
    run(
        &cc,
        &[
            "-O2",
            c_p.to_str().ok_or("bad c path")?,
            o_p.to_str().ok_or("bad obj path")?,
            "-o",
            bin_p.to_str().ok_or("bad bin path")?,
        ],
        "harness link",
    )?;

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
    use crate::wish::Value;

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
        let mlir = lower::emit_fn(&cand.name, &cand.params, &cand.ret, &cand.body, true).unwrap();

        let inputs = vec![Value::List(vec![3, 1, 4, 1, 5, 9, 2, 6])];
        let expect = interp::eval_candidate(&cand, &inputs, interp::Ctx::wrapping())
            .expect("interp evaluates");
        let got = eval_native(
            &mlir,
            &cand.name,
            &[CK::List],
            &[3, 1, 4, 1, 5, 9, 2, 6],
            &[],
            &[],
            &[],
            false,
        )
        .expect("native evaluates");
        assert_eq!(got, 31.0, "sanity");
        assert_eq!(got, match expect {
            Value::Int(v) => v as f64,
            other => panic!("unexpected {:?}", other),
        });
    }

    #[test]
    fn test_bench_harness_source_shape() {
        let c = bench_c_source("f", &[CK::List, CK::I64], 10);
        assert!(c.contains("extern long f(void*, void*, long, long, long, long);"));
        assert!(c.contains("b0, b0, 0, N, 1, s1"));
        let c_scalar_only = bench_c_source("g", &[CK::I64], 5);
        assert!(c_scalar_only.contains("extern long g(long);"));
    }
}

#[cfg(test)]
mod trap_tests {
    use super::*;
    use crate::{check, interp, lower, sketch};
    use crate::wish::Value;

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
        let mlir = lower::emit_fn(&cand.name, &cand.params, &cand.ret, &cand.body, false).unwrap();

        // Clean inputs: both tiers agree.
        let got = eval_native(&mlir, "f", &[CK::List], &[3, 4, 5], &[], &[], &[], false)
            .expect("clean runs");
        assert_eq!(got, 12.0);

        // Overflowing inputs: interpreter kills...
        let killed = interp::eval_candidate(
            &cand,
            &[Value::List(vec![i64::MAX, 1])],
            interp::Ctx::checked(),
        );
        assert!(killed.is_err());
        // ...and native must not return a value either.
        assert!(
            eval_native(&mlir, "f", &[CK::List], &[i64::MAX, 1], &[], &[], &[], false).is_err(),
            "native returned a value where the oracle kills"
        );
    }
}

#[cfg(test)]
mod float_tests {
    use super::*;
    use crate::{check, interp, lower, sketch};
    use crate::wish::Value;

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
        let mlir = lower::emit_fn(&cand.name, &cand.params, &cand.ret, &cand.body, false).unwrap();
        assert!(!mlir.contains("i128"), "float math entered trap expansion");
        assert!(mlir.contains("arith.mulf"));

        let expect = interp::eval_candidate(
            &cand,
            &[Value::Float(1.5), Value::Float(2.5)],
            interp::Ctx::checked(),
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
            true,
        )
        .unwrap();
        match expect {
            Value::Float(f) => assert_eq!(got, f),
            other => panic!("unexpected {:?}", other),
        }
    }
}

#[cfg(test)]
mod listf64_tests {
    use super::*;
    use crate::{check, interp, lower, sketch};
    use crate::wish::Value;

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
            lower::emit_fn(&cand.name, &cand.params, &cand.ret, &cand.body, true).unwrap();
        assert!(mlir.contains("memref<?xf64>"), "param type not f64");
        assert!(mlir.contains("memref.load") && mlir.contains("arith.addf"));

        let inputs = vec![Value::FloatList(vec![1.5, 2.0, -0.5])];
        let expect = interp::eval_candidate(&cand, &inputs, interp::Ctx::wrapping()).unwrap();
        let got = eval_native(
            &mlir,
            "dot",
            &[CK::ListF64],
            &[],
            &[1.5, 2.0, -0.5],
            &[],
            &[],
            true,
        )
        .unwrap();
        match expect {
            Value::Float(f) => assert_eq!(got, f),
            other => panic!("unexpected {:?}", other),
        }
    }
}
