//! Ontic CLI: `check` a wish, `solve` it (hand candidates or forge), `bench`
//! survivors, and inspect the `vault`. Hand-rolled arg parsing — no clap.

use ontic::forge::{self, ForgeConfig};
use ontic::lower;
use ontic::pipeline;
use ontic::program;
use ontic::recipe;
use ontic::sketch;
use ontic::sieve::{self, SiegeConfig};
use ontic::vault::Vault;
use ontic::wish;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    std::process::exit(dispatch(&args));
}

/// Route to subcommand; returns process exit code.
fn dispatch(args: &[String]) -> i32 {
    match args.get(1).map(|s| s.as_str()) {
        Some("check") => match args.get(2) {
            Some(path) => cmd_check(path),
            None => usage("check needs a .ont file"),
        },
        Some("run") => match args.get(2) {
            Some(path) => cmd_run(path),
            None => usage("run needs a .ont file"),
        },
        Some("solve") => cmd_solve(args),
        Some("bench") => cmd_bench(args),
        Some("vault") => cmd_vault(args),
        Some("--help") | Some("-h") | Some("help") | None => {
            print_help();
            0
        }
        Some(other) => usage(&format!("unknown command `{}`", other)),
    }
}

fn usage(msg: &str) -> i32 {
    eprintln!("error: {}", msg);
    print_help();
    1
}

fn print_help() {
    println!(
        "ontic — stochastic specification compiler

USAGE:
  ontic check <file.ont>                          validate a wish, report probe strength
  ontic solve <file.ont> [opts]                   sieve candidates; winner -> vault as MLIR
  ontic bench <file.ont> [opts]                   rank survivors with timings only
  ontic run <file.ont>                            execute a recipe over vaulted fns
  ontic vault [--dir D]                           list verified functions

SOLVE OPTIONS:
  --hand <file>     candidate sketch file (repeatable; skips forge)
  --samples <N>     forge sample count (default 32)
  --forge <h:p>     llama-server endpoint (default env ONTIC_FORGE or {})",
        forge::DEFAULT_FORGE
    );
}

/// Load and fully validate a wish file.
/// Load an .ont file (single wish OR multi-wish + program) and validate
/// every wish in it.
fn load_file(path: &str) -> Result<recipe::OntFile, String> {
    let src = std::fs::read_to_string(path).map_err(|e| format!("read {}: {}", path, e))?;
    let f = recipe::parse_ont(&src)?;
    for w in &f.wishes {
        sieve::validate_wish(w)?;
    }
    Ok(f)
}

/// Pick a wish by path (default: first).
fn pick_wish<'a>(f: &'a recipe::OntFile, want: Option<&str>) -> Result<wish::Wish, String> {
    match want {
        Some(p) => f
            .wishes
            .iter()
            .find(|w| w.path == p)
            .map(|w| w.clone())
            .ok_or_else(|| format!("no wish `{}` in file", p)),
        None => f
            .wishes
            .first()
            .map(|w| w.clone())
            .ok_or_else(|| "file has no wishes".to_string()),
    }
}

fn cmd_check(path: &str) -> i32 {
    match load_file(path).and_then(|f| pick_wish(&f, None)) {
        Ok(w) => {
            println!("wish      : {}", w.path);
            println!("params    : {}", w.params.len());
            println!("invariants: {}", w.invariants.len());
            println!("tier      : {}", if w.wrapping { "wrapping" } else { "checked" });
            println!("transparent examples: {}", w.transparent.len());
            println!("opaque examples     : {}{}", w.opaque.len(), if w.auto_split { " (auto-split)" } else { "" });
            let cfg = SiegeConfig::default();
            let rows = probes_count(&w, &cfg);
            println!("probe plan: {} rows (seed 0x{:X})", rows, cfg.seed);
            if w.invariants.is_empty() {
                println!("note      : no invariants — probes check runtime errors only");
            }
            0
        }
        Err(e) => {
            eprintln!("invalid wish: {}", e);
            1
        }
    }
}

use ontic::probes;

fn probes_count(w: &wish::Wish, cfg: &SiegeConfig) -> usize {
    probes::generate(w, cfg.probe_count, cfg.seed, cfg.edge_budget).len()
}

/// Resolve forge config from flags/env.
fn forge_config(forge_flag: Option<&str>, samples: usize, seed: u64) -> ForgeConfig {
    let mut cfg = ForgeConfig::default();
    let endpoint = forge_flag
        .map(|s| s.to_string())
        .or_else(|| std::env::var("ONTIC_FORGE").ok());
    if let Some(ep) = endpoint {
        let (h, p) = forge::parse_endpoint(&ep);
        cfg.host = h;
        cfg.port = p;
    }
    cfg.samples = samples;
    cfg.seed = seed;
    cfg
}

/// Extract repeated --hand paths plus scalar options from raw args.
struct SolveOpts {
    wish_path: String,
    /// Optional `--wish Path` selector for multi-wish files.
    wish_sel: Option<String>,
    hand: Vec<String>,
    samples: usize,
    seed: u64,
    forge: Option<String>,
}

fn parse_solve_args(args: &[String]) -> Result<SolveOpts, String> {
    let wish_path = match args.get(2) {
        Some(p) => p.clone(),
        None => return Err("solve needs a .ont file".to_string()),
    };
    let mut opts = SolveOpts {
        wish_path,
        wish_sel: None,
        hand: Vec::new(),
        samples: 32,
        seed: 0x5EED,
        forge: None,
    };
    let mut i = 3;
    while i < args.len() {
        match args[i].as_str() {
            "--hand" => {
                // Greedy: consume consecutive non-flag paths as candidates.
                i += 1;
                while let Some(f) = args.get(i) {
                    if f.starts_with("--") {
                        break;
                    }
                    opts.hand.push(f.clone());
                    i += 1;
                }
                if opts.hand.is_empty() {
                    return Err("--hand needs at least one file path".to_string());
                }
                continue;
            }
            "--samples" => {
                i += 1;
                opts.samples = args
                    .get(i)
                    .and_then(|v| v.parse().ok())
                    .ok_or_else(|| "--samples needs a number".to_string())?;
            }
            "--seed" => {
                i += 1;
                opts.seed = args
                    .get(i)
                    .and_then(|v| v.parse().ok())
                    .ok_or_else(|| "--seed needs a number".to_string())?;
            }
            "--wish" => {
                i += 1;
                opts.wish_sel = Some(
                    args.get(i)
                        .ok_or_else(|| "--wish needs a wish path".to_string())?
                        .clone(),
                );
            }
            "--forge" => {
                i += 1;
                opts.forge = Some(
                    args.get(i)
                        .ok_or_else(|| "--forge needs host:port".to_string())?
                        .clone(),
                );
            }
            other => return Err(format!("unknown option `{}`", other)),
        }
        i += 1;
    }
    Ok(opts)
}

/// Read hand-written candidate files into labeled texts.
fn load_hand(paths: &[String]) -> Result<Vec<(String, String)>, String> {
    let mut out = Vec::new();
    for (i, p) in paths.iter().enumerate() {
        let text =
            std::fs::read_to_string(p).map_err(|e| format!("read {}: {}", p, e))?;
        out.push((format!("hand-{}:{}", i, p), text));
    }
    Ok(out)
}

fn print_report(r: &sieve::SieveReport) {
    for (label, rej) in &r.rejections {
        println!(
            "KILLED {:<28} {} / {} : {}",
            label,
            rej.stage.label(),
            rej.kind.label(),
            rej.reason
        );
    }
    for (rank, s) in r.survivors.iter().enumerate() {
        println!(
            "PASS   #{:<2} {:>8} ns/call  {:>4} AST nodes",
            rank,
            s.ns_per_call,
            sieve::ast_size(&s.candidate.body)
        );
    }
}

fn cmd_solve(args: &[String]) -> i32 {
    let opts = match parse_solve_args(args) {
        Ok(o) => o,
        Err(e) => return usage(&e),
    };
    run_solve(&opts, true)
}

fn cmd_bench(args: &[String]) -> i32 {
    let opts = match parse_solve_args(args) {
        Ok(o) => o,
        Err(e) => return usage(&e),
    };
    run_solve(&opts, false)
}

/// Shared solve/bench pipeline. When `store` is true the winner is lowered
/// to MLIR and written to the vault.
fn run_solve(opts: &SolveOpts, store: bool) -> i32 {
    let w = match load_file(&opts.wish_path).and_then(|f| pick_wish(&f, opts.wish_sel.as_deref()))
    {
        Ok(w) => w,
        Err(e) => {
            eprintln!("invalid wish: {}", e);
            return 1;
        }
    };
    let cfg = SiegeConfig::default();

    let candidates = if !opts.hand.is_empty() {
        match load_hand(&opts.hand) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("{}", e);
                return 1;
            }
        }
    } else {
        let fcfg = forge_config(opts.forge.as_deref(), opts.samples, opts.seed);
        println!(
            "forging {} candidates from {}:{} ...",
            fcfg.samples, fcfg.host, fcfg.port
        );
        match forge::sample(&w, &fcfg, &[]) {
            Ok(texts) => texts.into_iter().enumerate().map(|(i, t)| (format!("forge-{}", i), t)).collect(),
            Err(e) => {
                eprintln!("forge failed: {}", e);
                return 1;
            }
        }
    };

    let mut report = match sieve::run(&w, &candidates, &cfg) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("wish rejected by sieve preconditions: {}", e);
            return 1;
        }
    };
    print_report(&report);

    // One feedback round when forging produced nothing usable.
    if report.survivors.is_empty() && opts.hand.is_empty() {
        let feedback: Vec<String> = report
            .rejections
            .iter()
            .take(16)
            .map(|(l, r)| format!("{}/{}/{}: {}", l, r.stage.label(), r.kind.label(), r.reason))
            .collect();
        let fcfg = forge_config(opts.forge.as_deref(), opts.samples, opts.seed.wrapping_add(1));
        println!("feedback round: {} resamples ...", fcfg.samples);
        match forge::sample(&w, &fcfg, &feedback) {
            Ok(texts) => {
                let cands: Vec<(String, String)> = texts
                    .into_iter()
                    .enumerate()
                    .map(|(i, t)| (format!("retry-{}", i), t))
                    .collect();
                match sieve::run(&w, &cands, &cfg) {
                    Ok(r2) => {
                        print_report(&r2);
                        merge_reports(&mut report, r2);
                    }
                    Err(e) => eprintln!("retry sieving failed: {}", e),
                }
            }
            Err(e) => eprintln!("forge retry failed: {}", e),
        }
    }

    match report.survivors.first() {
        None => {
            eprintln!("no candidate survived the sieve");
            1
        }
        Some(_) => {
            native_rerank(&w, &mut report.survivors);
            print_native_ranking(&report);
            let winner = report.survivors.first().unwrap();
            if !store {
                return 0;
            }
            emit_and_store(&w, winner)
        }
    }
}

/// When the LLVM toolchain is present, re-time every survivor on real
/// compiled objects and re-rank by that. Interpreter timing remains the
/// fallback ordering; the native table is the honest one.
fn native_rerank(w: &wish::Wish, survivors: &mut Vec<sieve::Survivor>) {
    if pipeline::find_tool("mlir-opt").is_none() || pipeline::find_tool("llc").is_none() {
        println!("native bench: toolchain missing, interpreter ranking stands");
        return;
    }
    let mut measured: Vec<(sieve::Survivor, u64)> = Vec::new();
    for s in survivors.iter() {
        match lower::emit_fn(
            &s.candidate.name,
            &s.candidate.params,
            &s.candidate.ret,
            &s.candidate.body,
            w.wrapping,
        ) {
            Ok(mlir) => {
                let kinds: Vec<pipeline::CK> = s
                    .candidate
                    .params
                    .iter()
                    .map(|(_, t)| match t {
                        sketch::Ty::ListInt => pipeline::CK::List,
                        sketch::Ty::ListF64 => pipeline::CK::ListF64,
                        sketch::Ty::F64 => pipeline::CK::F64,
                        _ => pipeline::CK::I64,
                    })
                    .collect();
                // S7 input sizing: fixed 1024-element probe buffer, 2000 iters.
                match pipeline::bench_native(&mlir, &s.candidate.name, &kinds, 2_000) {
                    Ok(ns) => measured.push((s.clone(), ns)),
                    Err(e) => eprintln!(
                        "native bench failed for {}: {} (interpreter ranking stands for this candidate)",
                        s.candidate.name, e
                    ),
                }
            }
            Err(e) => eprintln!("lowering failed during native rerank: {}", e),
        }
    }
    if !measured.is_empty() {
        measured.sort_by_key(|(s, ns)| (*ns, sieve::ast_size(&s.candidate.body)));
        *survivors = measured.into_iter().map(|(mut s, ns)| {
            s.ns_per_call = ns;
            s
        }).collect();
    }
}

/// Print the post-native-ranking table with a mode label.
fn print_native_ranking(report: &sieve::SieveReport) {
    println!("native ranking:");
    for (rank, s) in report.survivors.iter().enumerate() {
        println!(
            "  #{:<2} {:>8} ns/call  {:>4} AST nodes",
            rank,
            s.ns_per_call,
            sieve::ast_size(&s.candidate.body)
        );
    }
}

/// Fold retry results into the primary report deterministically.
fn merge_reports(base: &mut sieve::SieveReport, extra: sieve::SieveReport) {
    base.rejections.extend(extra.rejections);
    base.survivors.extend(extra.survivors);
    // Re-rank after merge so the printed winner is global.
    base.survivors
        .sort_by_key(|s| (s.ns_per_call, sieve::ast_size(&s.candidate.body)));
}

/// Lower the winner, best-effort validate with mlir-opt, store in vault.
fn emit_and_store(w: &wish::Wish, survivor: &sieve::Survivor) -> i32 {
    let mlir = match lower::emit_fn(
        &survivor.candidate.name,
        &survivor.candidate.params,
        &survivor.candidate.ret,
        &survivor.candidate.body,
        w.wrapping,
    ) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("lowering failed (candidate verified but not emittable): {}", e);
            return 1;
        }
    };
    // Mandatory when the toolchain is present: unvalidated IR never vaults.
    let staged = std::env::temp_dir().join("ontic-emit-check.mlir");
    match std::fs::write(&staged, &mlir)
        .map_err(|e| e.to_string())
        .and_then(|_| pipeline::validate_mlir(&staged))
    {
        Ok(()) => println!("MLIR    : validated by mlir-opt"),
        Err(e) => {
            if pipeline::find_tool("mlir-opt").is_none() {
                println!("MLIR    : mlir-opt not installed; validation skipped");
            } else {
                eprintln!("FATAL   : mlir-opt rejected emission:\n{}", e);
                return 1;
            }
        }
    }
    let vault_dir = std::env::var("ONTIC_VAULT").unwrap_or_else(|_| ".ontic/vault".to_string());
    let v = match Vault::open(&vault_dir) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{}", e);
            return 1;
        }
    };
    match v.put(w, &survivor.source_text, &mlir) {
        Ok(key) => {
            println!("VAULTED {} ({})", w.path, key);
            0
        }
        Err(e) => {
            eprintln!("{}", e);
            1
        }
    }
}

fn cmd_vault(args: &[String]) -> i32 {
    let dir = match args.iter().position(|a| a == "--dir") {
        Some(i) => match args.get(i + 1) {
            Some(d) => d.clone(),
            None => return usage("--dir needs a path"),
        },
        None => std::env::var("ONTIC_VAULT").unwrap_or_else(|_| ".ontic/vault".to_string()),
    };
    let v = match Vault::open(&dir) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{}", e);
            return 1;
        }
    };
    match v.list() {
        Ok(entries) => {
            if entries.is_empty() {
                println!("vault empty at {}", dir);
            }
            for e in entries {
                println!("{}  {}  {}", &e.key[..12.min(e.key.len())], e.name, e.signature);
            }
            0
        }
        Err(e) => {
            eprintln!("{}", e);
            1
        }
    }
}

/// `ontic run <file.ont>` — execute a recipe over vault-verified functions.
fn cmd_run(path: &str) -> i32 {
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("read {}: {}", path, e);
            return 1;
        }
    };
    let file = match recipe::parse_ont(&src) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("invalid ont file: {}", e);
            return 1;
        }
    };
    if file.program.is_none() {
        eprintln!("no program block in {}", path);
        return 1;
    }
    match program::run(&file) {
        Ok(lines) => {
            for l in lines {
                println!("{}", l);
            }
            0
        }
        Err(e) => {
            eprintln!("{}", e);
            1
        }
    }
}
