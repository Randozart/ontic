//! Native pipeline: verified MLIR → LLVM IR → object → measured binary.
//!
//! Uses the system LLVM toolchain (mlir-opt, mlir-translate, llc, clang) via
//! subprocess. Tool discovery: ONTIC_MLIR_BIN dir override, then common
//! llvm-prefix dirs, then PATH. Every stage reports clean errors.

use std::path::PathBuf;
use std::process::Command;


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
pub fn bench_c_source(fn_name: &str, params_is_list: &[bool], iters: usize) -> String {
    let mut proto = String::new();
    let mut decls = String::new();
    let mut init = String::new();
    let mut call_args = String::new();
    for (i, is_list) in params_is_list.iter().enumerate() {
        if *is_list {
            if !proto.is_empty() {
                proto.push_str(", ");
            }
            proto.push_str("void*, void*, long, long, long");
            decls.push_str(&format!("  long* b{} = malloc(N * sizeof(long));\n", i));
            init.push_str(&format!(
                "    for (long i = 0; i < N; i++) b{0}[i] = (i * 7 + 3) % 97;\n",
                i
            ));
            call_args.push_str(&format!("b{}, b{}, 0, N, 1, ", i, i));
        } else {
            if !proto.is_empty() {
                proto.push_str(", ");
            }
            proto.push_str("long");
            decls.push_str(&format!("  long s{} = 3;\n", i));
            call_args.push_str(&format!("s{}, ", i));
        }
    }
    format!(
        r#"#include <stdio.h>
#include <stdlib.h>
#include <time.h>

extern long {fname}({proto});

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

/// Full native measurement: write mlir, lower, emit object, build harness,
/// run ROUNDS times, return median ns-per-call as measured inside the binary.
pub fn bench_native(
    mlir_text: &str,
    fn_name: &str,
    params_is_list: &[bool],
    iters: usize,
) -> Result<u64, String> {
    let dir = std::env::temp_dir().join(format!("ontic-bench-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mlir_p = dir.join("cand.mlir");
    let ll_mlir = dir.join("cand_llvm.mlir");
    let o_p = dir.join("cand.o");
    let c_p = dir.join("bench.c");
    let bin_p = dir.join("bench");

    std::fs::write(&mlir_p, mlir_text).map_err(|e| e.to_string())?;
    mlir_to_llvmir(&mlir_p, &ll_mlir)?;
    object_from_ll(&ll_mlir, &o_p)?;
    std::fs::write(&c_p, bench_c_source(fn_name, params_is_list, iters))
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
