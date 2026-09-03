//! Ontic CLI: `check` a gen, `solve` it (hand candidates or forge), `bench`
//! survivors, and inspect the `vault`. Hand-rolled arg parsing — no clap.

use ontic::check;
use ontic::forge::{self, ForgeConfig};
use ontic::gen;
use ontic::interp;
use ontic::lower;
use ontic::pipeline;
use ontic::program;
use ontic::recipe;
use ontic::sieve::{self, SiegeConfig};
use ontic::sketch;
use ontic::vault::Vault;
use std::process::Command;

fn main() {
    // .env is the lowest-precedence source; real env always wins.
    let _ = ontic::dotenv::load(std::path::Path::new("."));
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
        Some("decompose") => cmd_decompose(args),
        Some("corpus") => cmd_corpus(args),
        Some("eval") => cmd_eval(args),
        Some("sweep") => cmd_sweep(args),
        Some("bench") => cmd_bench(args),
        Some("vault") => cmd_vault(args),
        Some("lint") => match args.get(2) {
            Some(path) => cmd_lint(path),
            None => usage("lint needs a .ont file"),
        },
        #[cfg(feature = "proven")]
        Some("prove") => cmd_prove(&args[2..]),
        Some("lib") => cmd_lib(args),
        Some("ablate") => cmd_ablate(args),
        Some("pack") => cmd_pack(args),
        Some("unpack") => cmd_unpack(args),
        Some("key") => match args.get(2) {
            Some(path) => cmd_key(path, None),
            None => usage("key needs a .ont file"),
        },
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
  ontic check <file.ont>                          validate a gen, report probe strength
  ontic solve <file.ont> [opts]                   sieve candidates; winner -> vault as MLIR
  ontic decompose <paper.txt|-> [opts]            paper text -> .ont tree (differential drafts,
                                                  one confirm gate, topo-solved with budgets)
    opts: --spec-backend gemini|openai|llama|file:<path>
          --candidate-backend B --candidate-samples N
          --repair-rounds K --recuts N --yes --outdir D
  ontic bench <file.ont> [opts]                   rank survivors with timings only
  ontic run <file.ont>                            execute a recipe over vaulted fns
  ontic vault [--dir D]                           list verified functions
  ontic lib [ls|promote <P>|demote <P>]           manage graduated stdlib entries
  ontic lib build <file.ont> [--sampler-backend B] [--samples N]  solve all gens -> one .so+header
  ontic ablate <file.ont> --samples N             uniform-vs-LLM control experiment
  ontic pack <key|Path> -o <name>.ous             bundle a kernel into .ous
  ontic unpack <x.ous> -d <dir>                   extract .so/.h/.mlir from bundle
  ontic key <file.ont> [--gen Path]               print canonical SHA-256 key
  ontic prove <file.ont> --hand <cand>            overflow-absence proof (build: --features proven)
  ontic corpus [backfill|stats|export]            training-corpus tooling
    export: --format chat|dpo --out F --exclude-key K1,K2  (ONTIC_COLLECT=1)

SOLVE OPTIONS:
  --hand <file>     candidate sketch file (repeatable; skips forge)
  --samples <N>     forge sample count (default 32)
  --forge <h:p>     llama-server endpoint (default env ONTIC_FORGE or {})",
        forge::DEFAULT_FORGE
    );
}

/// Load and fully validate a gen file.
/// Load an .ont file (single gen OR multi-gen + program) and validate
/// every gen in it.
fn load_file(path: &str) -> Result<recipe::OntFile, String> {
    let src = std::fs::read_to_string(path).map_err(|e| format!("read {}: {}", path, e))?;
    let f = recipe::parse_ont(&src)?;
    for w in &f.gens {
        sieve::validate_wish(w)?;
    }
    Ok(f)
}

/// Pick a gen by path (default: first).
fn pick_gen<'a>(f: &'a recipe::OntFile, want: Option<&str>) -> Result<gen::Gen, String> {
    match want {
        Some(p) => f
            .gens
            .iter()
            .find(|w| w.path == p)
            .map(|w| w.clone())
            .ok_or_else(|| format!("no gen `{}` in file", p)),
        None => f
            .gens
            .first()
            .map(|w| w.clone())
            .ok_or_else(|| "file has no gens".to_string()),
    }
}

fn cmd_check(path: &str) -> i32 {
    match load_file(path).and_then(|f| pick_gen(&f, None)) {
        Ok(w) => {
            println!("gen      : {}", w.path);
            println!("params    : {}", w.params.len());
            println!("invariants: {}", w.invariants.len());
            if !w.hints.is_empty() {
                println!("hints     : {} (advisory)", w.hints.len());
                for h in &w.hints {
                    println!("  - {}", h);
                }
            }
            println!("transparent examples: {}", w.transparent.len());
            println!(
                "opaque examples     : {}{}",
                w.opaque.len(),
                if w.auto_split { " (auto-split)" } else { "" }
            );
            let cfg = SiegeConfig::default();
            let ctx = interp::Ctx::checked();
            match probes::generate(&w, cfg.probe_count, cfg.seed, cfg.edge_budget, &ctx) {
                Ok(plan) => {
                    println!(
                        "probe plan: {} rows (seed 0x{:X}, {:?})",
                        plan.rows.len(),
                        cfg.seed,
                        plan.quality
                    );
                    if plan.quality == probes::PlanQuality::EdgesOnly {
                        println!("anomaly   : random sampling could not satisfy the contract in {} attempts — relational invariants defeat independent sampling", plan.attempts);
                        for (inv, n) in &plan.rejects {
                            println!("  rejected {}x by `{}`", n, inv);
                        }
                        println!("fix hint  : pass shape params explicitly (e.g. %n: Int with len relations) or provide more transparent examples; probe coverage is edge-only");
                    }
                }
                Err(_) => {
                    let invs: Vec<String> = w
                        .invariants
                        .iter()
                        .map(|i| lower::expr_display(i))
                        .collect();
                    println!(
                        "probe plan: 0 rows — ANOMALY: no input satisfies the declared contract [{}]",
                        invs.join("; ")
                    );
                    println!("fix hint  : the invariant set is contradictory over the type domain (or excludes all canonical edges). Loosen or correct an invariant.");
                }
            }
            if w.invariants.is_empty() {
                println!("note      : no invariants — probes check runtime errors only");
            }
            0
        }
        Err(e) => {
            eprintln!("invalid gen: {}", e);
            1
        }
    }
}

use ontic::probes;

/// Resolve forge config from flags/env.
fn forge_config(opts: &SolveOpts) -> ForgeConfig {
    let mut cfg = ForgeConfig::default();
    cfg.samples = opts.samples;
    cfg.seed = opts.seed;
    // Backend selection: flag > env > default(llama).
    if let Some(b) = &opts.sampler_backend {
        cfg.backend = forge::Backend::Llama; // placeholder replaced below
        let kind = match b.as_str() {
            "openai" | "openai-compat" => forge::Backend::OpenAICompat,
            "gemini" | "gemini-native" => forge::Backend::GeminiNative,
            "uniform" => forge::Backend::Uniform,
            _ => forge::Backend::Llama,
        };
        cfg.backend = kind;
        if matches!(
            kind,
            forge::Backend::OpenAICompat | forge::Backend::GeminiNative
        ) {
            cfg.model = opts
                .model
                .clone()
                .or_else(|| std::env::var("ONTIC_MODEL").ok())
                .unwrap_or_else(|| "gemini-3.5-flash-lite".to_string());
            let key_env = opts
                .api_key_env
                .clone()
                .unwrap_or_else(|| "GEMINI_API_KEY".to_string());
            if std::env::var(&key_env).is_err() {
                eprintln!(
                    "warning: ${} not set — cloud sampling will fail until it is",
                    key_env
                );
            }
        }
    } else if let Ok(b) = std::env::var("ONTIC_SAMPLER") {
        let kind = match b.as_str() {
            "openai" | "openai-compat" => Some(forge::Backend::OpenAICompat),
            "gemini" | "gemini-native" => Some(forge::Backend::GeminiNative),
            _ => None,
        };
        if let Some(kind) = kind {
            cfg.backend = kind;
            cfg.model = opts
                .model
                .clone()
                .or_else(|| std::env::var("ONTIC_MODEL").ok())
                .unwrap_or_else(|| "gemini-3.5-flash-lite".to_string());
        }
    }
    // Endpoint handling: llama uses host:port; cloud uses base URL.
    let endpoint = opts
        .endpoint
        .clone()
        .or_else(|| std::env::var("ONTIC_FORGE_ENDPOINT").ok());
    if matches!(
        cfg.backend,
        forge::Backend::OpenAICompat | forge::Backend::GeminiNative
    ) {
        if let Some(ep) = endpoint {
            cfg.endpoint = ep;
        }
    } else if let Some(ep) = endpoint.or_else(|| opts.forge.clone()) {
        let (h, p) = forge::parse_endpoint(&ep);
        cfg.host = h;
        cfg.port = p;
    } else if let Some(f) = &opts.forge {
        let (h, p) = forge::parse_endpoint(f);
        cfg.host = h;
        cfg.port = p;
    }
    cfg
}

/// Extract repeated --hand paths plus scalar options from raw args.
struct SolveOpts {
    wish_path: String,
    /// Optional `--gen Path` selector for multi-gen files.
    wish_sel: Option<String>,
    hand: Vec<String>,
    samples: usize,
    seed: u64,
    forge: Option<String>,
    sampler_backend: Option<String>,
    endpoint: Option<String>,
    model: Option<String>,
    api_key_env: Option<String>,
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
        sampler_backend: None,
        endpoint: None,
        model: None,
        api_key_env: None,
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
            "--sampler-backend" => {
                i += 1;
                opts.sampler_backend = Some(
                    args.get(i)
                        .ok_or_else(|| "--sampler-backend needs llama|openai|gemini".to_string())?
                        .clone(),
                );
            }
            "--endpoint" => {
                i += 1;
                opts.endpoint = Some(
                    args.get(i)
                        .ok_or_else(|| "--endpoint needs a URL".to_string())?
                        .clone(),
                );
            }
            "--model" => {
                i += 1;
                opts.model = Some(
                    args.get(i)
                        .ok_or_else(|| "--model needs a model name".to_string())?
                        .clone(),
                );
            }
            "--api-key-env" => {
                i += 1;
                opts.api_key_env = Some(
                    args.get(i)
                        .ok_or_else(|| "--api-key-env needs a variable name".to_string())?
                        .clone(),
                );
            }
            "--gen" => {
                i += 1;
                opts.wish_sel = Some(
                    args.get(i)
                        .ok_or_else(|| "--gen needs a gen path".to_string())?
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
        let text = std::fs::read_to_string(p).map_err(|e| format!("read {}: {}", p, e))?;
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
    let w = match load_file(&opts.wish_path).and_then(|f| pick_gen(&f, opts.wish_sel.as_deref())) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("invalid gen: {}", e);
            return 1;
        }
    };
    let cfg = SiegeConfig::default();

    let fcfg = forge_config(opts);
    let resolved = resolve_deps(&w);
    let candidates = if !opts.hand.is_empty() {
        match load_hand(&opts.hand) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("{}", e);
                return 1;
            }
        }
    } else {
        if matches!(fcfg.backend, forge::Backend::Llama) {
            println!(
                "forging {} candidates from {}:{} ...",
                fcfg.samples, fcfg.host, fcfg.port
            );
        } else {
            println!(
                "forging {} candidates via {} ({}) ...",
                fcfg.samples,
                fcfg.backend.label(),
                fcfg.model
            );
        }
        match forge::sample(&w, &fcfg, &[], &dep_block(&resolved)) {
            Ok((texts, usage)) => {
                println!(
                    "tokens  : prompt={} completion={}",
                    usage.prompt, usage.completion
                );
                texts
                    .into_iter()
                    .enumerate()
                    .map(|(i, t)| (format!("forge-{}", i), t))
                    .collect()
            }
            Err(e) => {
                eprintln!("forge failed: {}", e);
                return 1;
            }
        }
    };

    let first_prompt = forge::build_prompt(&w, &[], &dep_block(&resolved));
    let mut report = match sieve::run(&w, &candidates, &cfg, &resolved.map) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("gen rejected by sieve preconditions: {}", e);
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
        let fcfg = {
            let mut c = forge_config(opts);
            c.seed = c.seed.wrapping_add(1);
            // Repair mode: colder sampling turns exploration into refinement.
            c.temperature = 0.4;
            c
        };
        println!(
            "feedback round: {} resamples at T={} ...",
            fcfg.samples, fcfg.temperature
        );
        match forge::sample(&w, &fcfg, &feedback, &dep_block(&resolved)) {
            Ok((texts, usage)) => {
                println!(
                    "tokens  : prompt={} completion={} (retry)",
                    usage.prompt, usage.completion
                );
                let cands: Vec<(String, String)> = texts
                    .into_iter()
                    .enumerate()
                    .map(|(i, t)| (format!("retry-{}", i), t))
                    .collect();
                match sieve::run(&w, &cands, &cfg, &resolved.map) {
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
            native_rerank(&w, &resolved, &mut report.survivors);
            print_native_ranking(&report);
            // Corpus capture (opt-in via ONTIC_COLLECT=1 in .env): winners
            // plus killed candidates with machine reasons — SFT and DPO.
            if ontic::corpus::enabled() {
                let model_label = match fcfg.backend {
                    forge::Backend::Llama => {
                        format!("llama {}:{}", fcfg.host, fcfg.port)
                    }
                    _ => format!("{}/{}", fcfg.backend.label(), fcfg.model),
                };
                ontic::corpus::capture_solve(
                    &Vault::key_for(&w),
                    &fcfg.backend.label().to_string(),
                    &model_label,
                    &first_prompt,
                    &report,
                );
            }
            match report.survivors.first() {
                Some(winner) => {
                    if !store {
                        return 0;
                    }
                    emit_and_store(&w, winner, &resolved, &fcfg, &first_prompt)
                }
                None => {
                    eprintln!("no candidate survived native differential (all natives unproven)");
                    1
                }
            }
        }
    }
}

/// When the LLVM toolchain is present, re-time every survivor on real
/// compiled objects and re-rank by that. Interpreter timing remains the
/// fallback ordering; the native table is the honest one.
fn native_rerank(w: &gen::Gen, resolved: &ResolvedDeps, survivors: &mut Vec<sieve::Survivor>) {
    let dep_mlirs: Vec<String> = resolved.mlirs.clone();
    if pipeline::find_tool("mlir-opt").is_none() || pipeline::find_tool("llc").is_none() {
        println!("native bench: toolchain missing, interpreter ranking stands");
        return;
    }
    let mut measured: Vec<(sieve::Survivor, u64)> = Vec::new();
    let mut native_failed: Vec<String> = Vec::new();
    for s in survivors.iter() {
        match lower::emit_fn(
            &s.candidate.name,
            &s.candidate.params,
            &s.candidate.ret,
            &s.candidate.body,
            &resolved.calls,
        ) {
            Ok(cand_mlir) => {
                // Candidate + deps compile as ONE composite so intra-module
                // calls resolve at lowering time.
                let mut parts = dep_mlirs.clone();
                parts.push(cand_mlir);
                let mlir = lower::compose_modules(&parts).expect("composite compose");
                let kind_of = |t: &sketch::Ty| match t {
                    sketch::Ty::ListInt => pipeline::CK::List,
                    sketch::Ty::ListF64 => pipeline::CK::ListF64,
                    sketch::Ty::ListF32 => pipeline::CK::ListF32,
                    sketch::Ty::F64 => pipeline::CK::F64,
                    sketch::Ty::F32 => pipeline::CK::F32,
                    sketch::Ty::Bool | sketch::Ty::Tuple(_) | sketch::Ty::Int | sketch::Ty::Str => {
                        pipeline::CK::I64
                    }
                };
                let kinds: Vec<pipeline::CK> =
                    s.candidate.params.iter().map(|(_, t)| kind_of(t)).collect();
                let ret_kinds: Vec<pipeline::CK> = match &s.candidate.ret {
                    sketch::Ty::Tuple(cs) => cs.iter().map(kind_of).collect(),
                    one => vec![kind_of(one)],
                };
                // Real spec-shaped input row: transparent examples satisfy
                // the invariants by definition.
                let row: Vec<crate::gen::Value> = w
                    .transparent
                    .first()
                    .map(|ex| ex.inputs.clone())
                    .unwrap_or_default();
                // S7: bench on the real row, then GR6 differential parity —
                // native must reproduce the oracle's VALUE, not merely run.
                let benched = pipeline::bench_native(
                    &mlir,
                    &s.candidate.name,
                    &kinds,
                    2_000,
                    &[],
                    &ret_kinds,
                    &row,
                );
                if let Err(e) = benched {
                    eprintln!(
                        "KILLED {} native-exec / differential-unproven : {}",
                        s.candidate.name, e
                    );
                    native_failed.push(s.candidate.name.clone());
                    continue;
                }
                if let Err(e) = differential_parity(&mlir, &s.candidate, &resolved.map, &row) {
                    eprintln!("KILLED {} differential-mismatch : {}", s.candidate.name, e);
                    native_failed.push(s.candidate.name.clone());
                    continue;
                }
                let ns = pipeline::bench_native(
                    &mlir,
                    &s.candidate.name,
                    &kinds,
                    2_000,
                    &[],
                    &ret_kinds,
                    &row,
                )
                .unwrap_or_else(|_| s.ns_per_call);
                measured.push((s.clone(), ns));
            }
            Err(e) => {
                eprintln!("lowering failed during native rerank: {}", e);
                native_failed.push(s.candidate.name.clone());
            }
        }
    }
    if !native_failed.is_empty() {
        survivors.retain(|s| !native_failed.contains(&s.candidate.name));
    }
    if !measured.is_empty() {
        measured.sort_by_key(|(s, ns)| (*ns, sieve::ast_size(&s.candidate.body)));
        *survivors = measured
            .into_iter()
            .map(|(mut s, ns)| {
                s.ns_per_call = ns;
                s
            })
            .collect();
    }
}

/// Proven-vs-checked equivalence gate (P7 acceptance, non-negotiable):
/// the proven emission must reproduce the checked emission's VALUES on
/// the transparent-example row before flag-free code may land. Any
/// mismatch, exec failure, or unsupported driver shape returns the honest
/// reason — the caller falls back to checked. The oracle (interpreter)
/// stays the reference for both (GR6).
/// Returns Ok when the gate passes; `None` is the Ok-payload marker.
#[cfg(feature = "proven")]
fn proven_equivalence_gate(
    w: &gen::Gen,
    survivor: &sieve::Survivor,
    resolved: &ResolvedDeps,
    _checked_composite: &str,
) -> Option<String> {
    use crate::gen::Value;
    let cand = &survivor.candidate;
    if !matches!(cand.ret, sketch::Ty::Int | sketch::Ty::Bool) {
        return Some(format!(
            "return {} outside the proven subset (Int/Bool only)",
            cand.ret.name()
        ));
    }
    let row: Vec<Value> = w
        .transparent
        .first()
        .map(|ex| ex.inputs.clone())
        .unwrap_or_default();
    let checked_mlir = match lower::emit_fn(
        &cand.name,
        &cand.params,
        &cand.ret,
        &cand.body,
        &resolved.calls,
    ) {
        Ok(m) => m,
        Err(e) => return Some(format!("checked lowering failed: {e}")),
    };
    let proven_mlir = match lower::emit_fn_tier(
        &cand.name,
        &cand.params,
        &cand.ret,
        &cand.body,
        &resolved.calls,
        lower::Tier::Proven,
    ) {
        Ok(m) => m,
        Err(e) => return Some(format!("proven lowering failed: {e}")),
    };
    // Both tiers vs the oracle on the SAME row.
    let ictx = interp::Ctx {
        deps: std::sync::Arc::new(resolved.map.clone()),
    };
    let expect = match interp::eval_candidate(cand, &row, &ictx) {
        Ok(v) => v,
        Err(e) => return Some(format!("oracle eval failed: {e}")),
    };
    let kinds: Vec<pipeline::CK> = cand
        .params
        .iter()
        .map(|(_, t)| match t {
            sketch::Ty::Int | sketch::Ty::Bool => pipeline::CK::I64,
            _ => return Some("proven v1 covers scalar Int params only".to_string()),
        })
        .collect();
    let lists_i = Vec::new();
    let lists_f = Vec::new();
    let lists_f32 = Vec::new();
    let si_i: Vec<i64> = row
        .iter()
        .filter_map(|v| match v {
            Value::Int(x) => Some(*x),
            Value::Bool(b) => Some(*b as i64),
            _ => None,
        })
        .collect();
    let si_f = Vec::new();
    let si_f32 = Vec::new();
    for (label, mlir) in [("checked", &checked_mlir), ("proven", &proven_mlir)] {
        let got = match pipeline::eval_native(
            mlir,
            &cand.name,
            &kinds,
            &lists_i,
            &lists_f,
            &lists_f32,
            &si_i,
            &si_f,
            &si_f32,
            &[],
            pipeline::RetSpec::I64,
            &[],
        ) {
            Ok(g) => g,
            Err(e) => return Some(format!("{label} native exec failed: {e}")),
        };
        let g = got.first().copied().unwrap_or(f64::NAN);
        let e = match &expect {
            Value::Int(x) => *x as f64,
            Value::Bool(b) => *b as i64 as f64,
            _ => return Some("proven v1 covers Int/Bool returns only".to_string()),
        };
        if g != e {
            return Some(format!(
                "{label} native value {g} != oracle {e} on spec row"
            ));
        }
    }
    println!(
        "PROVEN    : equivalence gate passed (proven == checked == oracle on spec row)"
    );
    None
}

/// Differential value parity: oracle vs native on one row. Ok when the
/// return shape has a supported driver AND values agree; Err kills.
fn differential_parity(
    mlir: &str,
    cand: &sketch::Candidate,
    deps: &interp::DepMap,
    row: &[crate::gen::Value],
) -> Result<(), String> {
    use crate::gen::Value;
    // Return-shape driver selection. Every expressible return type has a
    // driver; an unmatched shape is a BUG — fail closed, never skip.
    let ret_spec = match &cand.ret {
        sketch::Ty::Int | sketch::Ty::Bool => pipeline::RetSpec::I64,
        sketch::Ty::F64 => pipeline::RetSpec::F64,
        sketch::Ty::F32 => pipeline::RetSpec::F32,
        sketch::Ty::ListF64 => pipeline::RetSpec::ListF64,
        sketch::Ty::ListF32 => pipeline::RetSpec::ListF32,
        sketch::Ty::ListInt => pipeline::RetSpec::ListI64,
        sketch::Ty::Str => pipeline::RetSpec::Str,
        sketch::Ty::Tuple(cs) => pipeline::RetSpec::Tuple(
            cs.iter()
                .map(|t| match t {
                    sketch::Ty::ListInt => pipeline::CK::List,
                    sketch::Ty::ListF64 => pipeline::CK::ListF64,
                    sketch::Ty::ListF32 => pipeline::CK::ListF32,
                    sketch::Ty::F64 => pipeline::CK::F64,
                    sketch::Ty::F32 => pipeline::CK::F32,
                    _ => pipeline::CK::I64,
                })
                .collect(),
        ),
    };

    // Parameter streams, grouped per list occurrence; F32 params route to
    // their own f32-typed C arrays so buffers are never empty/vacuous.
    let mut lists_i: Vec<Vec<i64>> = Vec::new();
    let mut lists_f: Vec<Vec<f64>> = Vec::new();
    let mut lists_f32: Vec<Vec<f64>> = Vec::new();
    let mut si_i = Vec::new();
    let mut si_f = Vec::new();
    let mut si_f32 = Vec::new();
    let mut strs: Vec<String> = Vec::new();
    for (v, (_, t)) in row.iter().zip(cand.params.iter()) {
        match (v, t) {
            (Value::Int(x), _) => si_i.push(*x),
            (Value::Bool(b), _) => si_i.push(*b as i64),
            (Value::Float(f), sketch::Ty::F32) => si_f32.push(*f),
            (Value::Float(f), _) => si_f.push(*f),
            (Value::List(vs), _) => lists_i.push(vs.clone()),
            (Value::FloatList(vs), sketch::Ty::ListF32) => lists_f32.push(vs.clone()),
            (Value::FloatList(vs), _) => lists_f.push(vs.clone()),
            (Value::Tuple(_), _) => {}
            (Value::Str(s), _) => strs.push(s.clone()),
        }
    }

    let ictx = interp::Ctx {
        deps: std::sync::Arc::new(deps.clone()),
    };
    let expect = interp::eval_candidate(cand, row, &ictx)
        .map_err(|e| format!("oracle re-eval failed: {e}"))?;

    let kinds: Vec<pipeline::CK> = cand
        .params
        .iter()
        .map(|(_, t)| match t {
            sketch::Ty::ListInt => pipeline::CK::List,
            sketch::Ty::ListF64 => pipeline::CK::ListF64,
            sketch::Ty::ListF32 => pipeline::CK::ListF32,
            sketch::Ty::F64 => pipeline::CK::F64,
            sketch::Ty::F32 => pipeline::CK::F32,
            sketch::Ty::Str => pipeline::CK::Str,
            _ => pipeline::CK::I64,
        })
        .collect();

    let got = if matches!(ret_spec, pipeline::RetSpec::Str) {
        // Str return: compare raw bytes, not numbers.
        let got_str = pipeline::eval_native_str(
            mlir,
            &cand.name,
            &kinds,
            &lists_i,
            &lists_f,
            &lists_f32,
            &si_i,
            &si_f,
            &si_f32,
            &strs,
            ret_spec.clone(),
            &[],
        )
        .map_err(|e| format!("native eval failed: {e}"))?;
        if let Value::Str(want) = &expect {
            if got_str != *want {
                return Err(format!(
                    "oracle says {:?}, native says {:?}",
                    want, got_str
                ));
            }
        }
        return Ok(());
    } else {
        pipeline::eval_native(
            mlir,
            &cand.name,
            &kinds,
            &lists_i,
            &lists_f,
            &lists_f32,
            &si_i,
            &si_f,
            &si_f32,
            &strs,
            ret_spec.clone(),
            &[],
        )
        .map_err(|e| format!("native eval failed: {e}"))?
    };

    // Shape-aware comparison against the oracle value.
    let close = |g: f64, w: f64| (g - w).abs() <= 1e-6_f64.max(w.abs() * 1e-9);
    match (&expect, &cand.ret) {
        (Value::FloatList(vs), sketch::Ty::ListF64)
        | (Value::FloatList(vs), sketch::Ty::ListF32) => {
            for (i, w) in vs.iter().take(4).enumerate() {
                let g = got.get(i + 1).copied().unwrap_or(f64::NAN);
                if !close(g, *w) {
                    return Err(format!("elem {i}: oracle says {w}, native says {g}"));
                }
            }
            Ok(())
        }
        (Value::List(vs), sketch::Ty::ListInt) => {
            for (i, w) in vs.iter().take(4).enumerate() {
                let g = got.get(i + 1).copied().unwrap_or(f64::NAN);
                if !close(g, *w as f64) {
                    return Err(format!("elem {i}: oracle says {w}, native says {g}"));
                }
            }
            Ok(())
        }
        (Value::Tuple(ws), sketch::Ty::Tuple(_)) => {
            for (i, w) in ws.iter().enumerate() {
                let g = got.get(i).copied().unwrap_or(f64::NAN);
                let w = match w {
                    Value::Int(v) => *v as f64,
                    Value::Bool(b) => *b as i64 as f64,
                    Value::Float(f) => *f,
                    other => return Err(format!("tuple component {other:?} unsupported")),
                };
                if !close(g, w) {
                    return Err(format!("comp {i}: oracle says {w}, native says {g}"));
                }
            }
            Ok(())
        }
        _ => {
            let want = match &expect {
                Value::Int(v) => *v as f64,
                Value::Bool(b) => *b as i64 as f64,
                Value::Float(f) => *f,
                other => return Err(format!("unsupported oracle shape {other:?}")),
            };
            let g = got.first().copied().unwrap_or(f64::NAN);
            if close(g, want) {
                Ok(())
            } else {
                Err(format!("oracle says {want}, native says {g}"))
            }
        }
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

/// Resolve a gen's declared dependencies against the vault by path.
/// Flat closure: transitive calls must all be listed in the top gen.
/// Resolved dependency set: runtime table + raw MLIR modules for linking.
struct ResolvedDeps {
    map: interp::DepMap,
    mlirs: Vec<String>,
    calls: lower::CallMap,
}

impl ResolvedDeps {
    fn empty() -> Self {
        ResolvedDeps {
            map: interp::DepMap::new(),
            mlirs: Vec::new(),
            calls: lower::CallMap::new(),
        }
    }
}

fn resolve_deps(w: &gen::Gen) -> ResolvedDeps {
    let vault_dir = std::env::var("ONTIC_VAULT").unwrap_or_else(|_| ".ontic/vault".to_string());
    let w_key = Vault::key_for(w);
    let v = Vault::open(&vault_dir);
    let mut map = interp::DepMap::new();
    let mut mlirs = Vec::new();
    let mut calls = lower::CallMap::new();
    for path in &w.deps {
        if let Some(entry) = v.find_by_path(path) {
            if let Ok(cand) = ontic::sketch::parse(&entry.sketch_text) {
                // The call symbol is the func name inside the dep's module.
                let symbol = entry
                    .mlir
                    .split("func.func @")
                    .nth(1)
                    .and_then(|r| r.find('('))
                    .map(|i| {
                        entry.mlir[..].split("func.func @").nth(1).unwrap()[..i]
                            .trim()
                            .to_string()
                    });
                map.insert(path.clone(), interp::DepFn { cand: cand.clone() });
                ontic::vault::record_reuse(&vault_dir, &entry.key, &w_key);
                if let Some(sym) = symbol {
                    calls.insert(
                        path.clone(),
                        lower::CallTarget {
                            symbol: sym,
                            params: cand.params.iter().map(|(_, t)| t.clone()).collect(),
                            ret: cand.ret,
                        },
                    );
                }
                mlirs.push(entry.mlir.clone());
            }
        }
    }
    if std::env::var("ONTIC_DEBUG").is_ok() {
        eprintln!(
            "DEBUG resolve: gen {} deps {:?} -> resolved {} (calls {})",
            w.path,
            w.deps,
            map.len(),
            calls.len()
        );
    }
    ResolvedDeps { map, mlirs, calls }
}

/// Render the AVAILABLE FUNCTIONS block for forge prompts: each resolved
/// dependency's signature so the model can discover compositions itself.
fn dep_block(resolved: &ResolvedDeps) -> String {
    let mut out = String::new();
    for (path, dep) in &resolved.map {
        let params: Vec<String> = dep
            .cand
            .params
            .iter()
            .map(|(n, t)| format!("%{}: {}", n, t.name()))
            .collect();
        out.push_str(&format!(
            "fn {}({}) -> {}\n",
            path,
            params.join(", "),
            dep.cand.ret.name()
        ));
    }
    out
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
fn emit_and_store(
    w: &gen::Gen,
    survivor: &sieve::Survivor,
    resolved: &ResolvedDeps,
    fcfg: &ForgeConfig,
    first_prompt: &str,
) -> i32 {
    #[allow(unused_mut)]
    let mut mlir = match lower::emit_fn(
        &survivor.candidate.name,
        &survivor.candidate.params,
        &survivor.candidate.ret,
        &survivor.candidate.body,
        &resolved.calls,
    ) {
        Ok(m) => m,
        Err(e) => {
            eprintln!(
                "lowering failed (candidate verified but not emittable): {}",
                e
            );
            return 1;
        }
    };
    // Mandatory when the toolchain is present: unvalidated IR never vaults.
    // Candidates calling deps are validated as a COMPOSITE module.
    let staged = std::env::temp_dir().join("ontic-emit-check.mlir");
    let mut parts: Vec<String> = resolved.mlirs.clone();
    parts.push(mlir.clone());
    #[allow(unused_mut)]
    let mut validation_text = lower::compose_modules(&parts).unwrap_or(mlir.clone());
    match std::fs::write(&staged, &validation_text)
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
    // PROVEN TIER (feature-gated, GR11): a recorded z3 Unsat verdict is
    // the declaration for flag-free codegen. Emission is gated on the
    // machine proof — never on author claim — and the equivalence gate
    // below verifies proven vs checked on the spec row before anything
    // vaults. Any failure falls back to checked (never a weaker sieve).
    #[allow(unused_mut)]
    let mut proof_stamp: Option<ontic::vault::ProofStamp> = None;
    #[cfg(feature = "proven")]
    {
        match ontic::prove::proof_for(w, &survivor.candidate) {
            ontic::prove::Proof::Proven(summary) => {
                if let Some(pr_reason) = proven_equivalence_gate(
                    w,
                    survivor,
                    resolved,
                    &validation_text,
                ) {
                    eprintln!(
                        "PROVEN EMISSION REFUSED (falling back to checked): {}",
                        pr_reason
                    );
                } else {
                    if let Ok(pm) = lower::emit_fn_tier(
                        &survivor.candidate.name,
                        &survivor.candidate.params,
                        &survivor.candidate.ret,
                        &survivor.candidate.body,
                        &resolved.calls,
                        lower::Tier::Proven,
                    ) {
                        let mut pp: Vec<String> = resolved.mlirs.clone();
                        pp.push(pm.clone());
                        match lower::compose_modules(&pp) {
                            Ok(ptxt) => {
                                if let Err(e) = std::fs::write(&staged, &ptxt)
                                    .and_then(|_| pipeline::validate_mlir(&staged))
                                {
                                    if pipeline::find_tool("mlir-opt").is_some() {
                                        eprintln!(
                                            "PROVEN EMISSION REFUSED (falling back to checked): mlir-opt rejected proven IR: {}",
                                            e
                                        );
                                    } else {
                                        println!("PROVEN    : mlir-opt missing; validation skipped");
                                    }
                                } else {
                                    println!("PROVEN    : flag-free emission validated");
                                }
                                mlir = pm;
                                validation_text = ptxt;
                                proof_stamp = Some(ontic::vault::ProofStamp {
                                    reason: format!("z3-unsat: {}", summary),
                                    details: vec!["straight-line Int subset".to_string()],
                                    attested: true,
                                });
                            }
                            Err(e) => eprintln!(
                                "PROVEN EMISSION REFUSED (falling back to checked): compose failed: {}",
                                e
                            ),
                        }
                    }
                }
            }
            ontic::prove::Proof::Unproven(reason) => println!(
                "PROVEN    : unproven ({}); checked tier emits",
                reason
            ),
        }
    }
    let vault_dir = std::env::var("ONTIC_VAULT").unwrap_or_else(|_| ".ontic/vault".to_string());
    let mut v = Vault::open(&vault_dir);
    // Kernel artifacts: header + shared library, built from the composite
    // (candidate + deps) so linked .so files are self-contained.
    let key8 = {
        let k = ontic::vault::Vault::key_for(w);
        k[..8.min(k.len())].to_string()
    };
    let header_name = format!("{}-{}.h", survivor.candidate.name, key8);
    let lib_name = format!("lib{}-{}.so", survivor.candidate.name, key8);
    let mut artifacts = serde_json::Map::new();

    if pipeline::find_tool("llc").is_some() && pipeline::find_tool("clang").is_some() {
        match build_shared_lib(
            &validation_text,
            &survivor.candidate.name,
            &resolved.mlirs,
            &vault_dir,
            &lib_name,
        ) {
            Ok(()) => {
                let hdr_path = std::path::Path::new(&vault_dir).join(&header_name);
                match lower::emit_header(
                    &survivor.candidate.name,
                    &survivor.candidate.params,
                    &survivor.candidate.ret,
                    &key8,
                    false,
                ) {
                    Ok(h) => match std::fs::write(&hdr_path, h) {
                        Ok(_) => {
                            println!("HEADER  : {}", hdr_path.display());
                            println!("LIB     : {}/{}", vault_dir, lib_name);
                            artifacts.insert(
                                "header".to_string(),
                                serde_json::Value::String(header_name.clone()),
                            );
                            artifacts.insert(
                                "lib".to_string(),
                                serde_json::Value::String(lib_name.clone()),
                            );
                            // Contracted C++ twin: sieve-proven invariants as
                            // native pre() under ONTIC_CONTRACTS.
                            let hpp_name = format!("{}-{}.hpp", survivor.candidate.name, key8);
                            let hpp_path = std::path::Path::new(&vault_dir).join(&hpp_name);
                            match lower::emit_header_hpp(
                                &survivor.candidate.name,
                                &survivor.candidate.params,
                                &survivor.candidate.ret,
                                &key8,
                                &w.invariants,
                            ) {
                                Ok(hh) => match std::fs::write(&hpp_path, hh) {
                                    Ok(_) => {
                                        println!("HPP     : {}", hpp_path.display());
                                        artifacts.insert(
                                            "header_hpp".to_string(),
                                            serde_json::Value::String(hpp_name),
                                        );
                                    }
                                    Err(e) => eprintln!("hpp write failed: {}", e),
                                },
                                Err(e) => eprintln!("hpp generation failed: {}", e),
                            }
                        }
                        Err(e) => eprintln!("header write failed: {}", e),
                    },
                    Err(e) => eprintln!("header generation failed: {}", e),
                }
                // Guarded twin: C shim wrapping the kernel with runtime
                // precondition checks.  Non-fatal — raw .so always lands.
                let guarded_lib_name =
                    format!("lib{}-{}.guarded.so", survivor.candidate.name, key8);
                let guarded_so_path = std::path::Path::new(&vault_dir).join(&guarded_lib_name);
                match lower::emit_shim_c(
                    &survivor.candidate.name,
                    &survivor.candidate.params,
                    &survivor.candidate.ret,
                    &key8,
                    &w.invariants,
                ) {
                    Ok(shim_src) => {
                        match pipeline::build_shared_so_guarded(
                            &validation_text,
                            &survivor.candidate.name,
                            &shim_src,
                            &guarded_so_path,
                        ) {
                            Ok(_shim_text) => {
                                println!("GUARDED : {}/{}", vault_dir, guarded_lib_name);
                                let shim_name =
                                    format!("{}-{}.guarded.c", survivor.candidate.name, key8);
                                let shim_path = std::path::Path::new(&vault_dir).join(&shim_name);
                                let _ = std::fs::write(&shim_path, &shim_src);
                                artifacts.insert(
                                    "guarded_lib".to_string(),
                                    serde_json::Value::String(guarded_lib_name),
                                );
                                artifacts.insert(
                                    "guarded_shim".to_string(),
                                    serde_json::Value::String(shim_name),
                                );
                                // Regenerate header with guarded section.
                                if let Ok(guarded_hdr) = lower::emit_header(
                                    &survivor.candidate.name,
                                    &survivor.candidate.params,
                                    &survivor.candidate.ret,
                                    &key8,
                                    true,
                                ) {
                                    let _ = std::fs::write(&hdr_path, guarded_hdr);
                                }
                            }
                            Err(e) => eprintln!("guarded build warning: {}", e),
                        }
                    }
                    Err(e) => eprintln!(
                        "GUARDED SKIPPED (fail-closed): {e}\n\
                         raw .so is vaulted but enforces NOTHING at runtime; \
                         restate the invariant in translatable form or drop the guarded tier"
                    ),
                }
            }
            Err(e) => eprintln!("shared library build failed: {}", e),
        }
    }

    // Prompt provenance (rule 12 companion): recorded with the solve so
    // prompts become regression-testable artifacts.
    let model_label = if matches!(fcfg.backend, forge::Backend::Llama) {
        format!("llama {}:{}", fcfg.host, fcfg.port)
    } else {
        format!("{} {}", fcfg.backend.label(), fcfg.model)
    };
    let mut meta_val = serde_json::json!({
        "last_solve": {
            "sampler": fcfg.backend.label(),
            "model": model_label,
            "temperature": fcfg.temperature,
            "samples": fcfg.samples,
            "seed_base": fcfg.seed,
            "prompt_sha256": ontic::sha256::sha256_hex(first_prompt.as_bytes()),
            "prompt": first_prompt,
        },
        "quality": survivor.probe_quality,
    });
    if !artifacts.is_empty() {
        meta_val["artifacts"] = serde_json::Value::Object(artifacts);
    }
    let meta = meta_val;
    // New vault API: key is derived by the caller; prompt provenance rides
    // beside the entry as {key}.meta.json (Entry has no meta field).
    let key = Vault::key_for(w);
    {
        let params: Vec<String> = w
            .params
            .iter()
            .map(|(n, t)| format!("%{}: {}", n, t.name()))
            .collect();
        let signature = format!("fn {}({}) -> {}", w.path, params.join(", "), w.ret.name());
        if let Some(stamp) = &proof_stamp {
            // Recorded z3 verdict: attested + tier=proven (GR11 — the
            // proof IS the contract word).
            if let Err(e) = v.put_proven(
                &key,
                &w.name,
                &signature,
                &survivor.source_text,
                &mlir,
                Some(w.source.as_str()),
                stamp,
            ) {
                eprintln!("{}", e);
                return 1;
            }
        } else if let Err(e) = v.put(
            &key,
            &w.name,
            &signature,
            &survivor.source_text,
            &mlir,
            Some(w.source.as_str()),
        ) {
            eprintln!("{}", e);
            return 1;
        }
        let _ = std::fs::write(
            std::path::Path::new(&vault_dir).join(format!("{key}.meta.json")),
            serde_json::to_string_pretty(&meta).unwrap_or_default(),
        );
    }
    println!("VAULTED {} ({})", w.path, key);
    emit_ous(w, survivor, resolved, &key, &vault_dir);
    0
}

fn cmd_vault(args: &[String]) -> i32 {
    // Subcommands: `vault export/import/status`; bare `vault` lists.
    // args = [prog, vault, SUB, …]
    match args.get(2).map(|s| s.as_str()) {
        Some("export") => return cmd_vault_export(&args[3..]),
        Some("import") => return cmd_vault_import(&args[3..]),
        Some("status") => return cmd_vault_status(&args[3..]),
        Some("rm") => return cmd_vault_rm(&args[3..]),
        Some("doctor") => return cmd_vault_doctor(&args[3..]),
        Some("gc") => return cmd_vault_gc(&args[3..]),
        _ => {}
    }
    let json = args.iter().any(|a| a == "--json");
    let dir = match args.iter().position(|a| a == "--dir") {
        Some(i) => match args.get(i + 1) {
            Some(d) => d.clone(),
            None => return usage("--dir needs a path"),
        },
        None => std::env::var("ONTIC_VAULT").unwrap_or_else(|_| ".ontic/vault".to_string()),
    };
    let v = Vault::open(&dir);
    let entries = v.list();
    if entries.is_empty() {
        println!("vault empty at {}", dir);
    }
    let promoted = read_lib_manifest();
    let reuse = ontic::vault::reuse_counts(&dir);
    if json {
        let mut arr = Vec::new();
        for e in &entries {
            let path = sig_path_of(e);
            arr.push(serde_json::json!({
                "key": e.key,
                "name": e.name,
                "path": path,
                "signature": e.signature,
                "trust": trust_label(&v, &e.key),
                "reuse": reuse.get(&e.key).copied().unwrap_or(0),
            }));
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::Value::Array(arr))
                .unwrap_or_else(|_| "[]".to_string())
        );
        return 0;
    }
    for e in entries {
        let path = {
            let inner = e.signature.strip_prefix("fn ").unwrap_or(&e.signature);
            match inner.find('(') {
                Some(i) => inner[..i].trim().to_string(),
                None => inner.trim().to_string(),
            }
        };
        let badge = if promoted.iter().any(|p| *p == path) {
            " [LIB]"
        } else {
            ""
        };
        let hits = reuse.get(&e.key).copied().unwrap_or(0);
        println!(
            "{}  {}{}  [{}]  [reuse {}]  {}",
            &e.key[..12.min(e.key.len())],
            e.name,
            badge,
            trust_label(&v, &e.key),
            hits,
            e.signature
        );
    }
    0
}

/// `ontic vault status <name>` — all versions of one path with trust and
/// on-disk artifact inventory.
fn cmd_vault_status(args: &[String]) -> i32 {
    let name = match args.iter().find(|a| !a.starts_with("--")) {
        Some(n) => n.clone(),
        None => return usage("status needs a kernel name"),
    };
    let dir = match args.iter().position(|a| a == "--dir") {
        Some(i) => match args.get(i + 1) {
            Some(d) => d.clone(),
            None => return usage("--dir needs a path"),
        },
        None => std::env::var("ONTIC_VAULT").unwrap_or_else(|_| ".ontic/vault".to_string()),
    };
    let v = Vault::open(&dir);
    let entries: Vec<&VaultEntry> = v
        .list()
        .into_iter()
        .filter(|e| sig_path_of(e) == name)
        .collect();
    if entries.is_empty() {
        return die(&format!("no vault entry for `{name}`"));
    }
    let dirp = std::path::Path::new(&dir);
    for e in &entries {
        let k8 = e.key[..8.min(e.key.len())].to_string();
        println!(
            "{}  {}  [{}]",
            &e.key[..12.min(e.key.len())],
            e.signature,
            trust_label(&v, &e.key)
        );
        let mut artifacts = Vec::new();
        for (label, file) in [
            ("so", format!("lib{}-{}.so", e.name, k8)),
            ("guarded_so", format!("lib{}-{}.guarded.so", e.name, k8)),
            ("h", format!("{}-{}.h", e.name, k8)),
            ("hpp", format!("{}-{}.hpp", e.name, k8)),
            ("guarded_c", format!("{}-{}.guarded.c", e.name, k8)),
            ("ous", format!("{}-{}.ous", e.name, k8)),
            ("obj", format!("{}-{}.o", e.name, k8)),
        ] {
            if dirp.join(&file).exists() {
                artifacts.push(label);
            }
        }
        let artifacts_s = if artifacts.is_empty() {
            "(none)".to_string()
        } else {
            artifacts.join(" ")
        };
        println!("  artifacts: {artifacts_s}");
    }
    0
}

fn cmd_vault_rm(args: &[String]) -> i32 {
    let key = match args.iter().find(|a| !a.starts_with("--")) {
        Some(k) => k.clone(),
        None => return usage("vault rm needs a key prefix"),
    };
    let dir = match args.iter().position(|a| a == "--dir") {
        Some(i) => match args.get(i + 1) {
            Some(d) => d.clone(),
            None => return usage("--dir needs a path"),
        },
        None => std::env::var("ONTIC_VAULT").unwrap_or_else(|_| ".ontic/vault".to_string()),
    };
    let mut v = Vault::open(&dir);
    // Resolve prefix to full key.
    let full_key = match v.get(&key) {
        Some(e) => e.key.clone(),
        None => return die(&format!("no vault entry matching prefix `{key}`")),
    };
    match v.delete(&full_key) {
        Ok(()) => {
            println!("removed {} entry", &full_key[..12.min(full_key.len())]);
            0
        }
        Err(e) => die(&e),
    }
}

fn cmd_vault_doctor(args: &[String]) -> i32 {
    let dir = match args.iter().position(|a| a == "--dir") {
        Some(i) => match args.get(i + 1) {
            Some(d) => d.clone(),
            None => return usage("--dir needs a path"),
        },
        None => std::env::var("ONTIC_VAULT").unwrap_or_else(|_| ".ontic/vault".to_string()),
    };
    let v = Vault::open(&dir);
    let findings = v.doctor();
    if findings.is_empty() {
        println!("vault doctor: clean");
    } else {
        println!("vault doctor: {} findings", findings.len());
        for (key, msg) in &findings {
            println!("  {} {key}: {}", &key[..12.min(key.len())], msg);
        }
    }
    0
}

fn cmd_vault_gc(args: &[String]) -> i32 {
    let dir = match args.iter().position(|a| a == "--dir") {
        Some(i) => match args.get(i + 1) {
            Some(d) => d.clone(),
            None => return usage("--dir needs a path"),
        },
        None => std::env::var("ONTIC_VAULT").unwrap_or_else(|_| ".ontic/vault".to_string()),
    };
    let mut v = Vault::open(&dir);
    let entries = v.list();
    let mut removed = 0u32;
    let mut entries_ref = Vec::new();
    for e in &entries {
        let k8 = e.key[..8.min(e.key.len())].to_string();
        let dirp = std::path::Path::new(&dir);
        let ous = dirp.join(format!("{}-{}.ous", e.name, k8));
        let so = dirp.join(format!("lib{}-{}.so", e.name, k8));
        let obj = dirp.join(format!("{}-{}.o", e.name, k8));
        let orphan = !ous.exists() || (!so.exists() && !obj.exists()) || e.gen_text.is_none();
        if orphan {
            entries_ref.push((e.key.clone(), e.name.clone()));
        }
    }
    for (key, name) in entries_ref {
        if v.delete(&key).is_ok() {
            println!("gc: removed {} ({})", &key[..12.min(key.len())], name);
            removed += 1;
        }
    }
    if removed == 0 {
        println!("vault gc: nothing to remove");
    } else {
        println!("vault gc: removed {removed} orphan(s)");
    }
    0
}

/// Flag lookup for vault subcommands: `--dir`, `--out`, boolean flags.
struct VaultSubOpts {
    dir: String,
    out: String,
    names: Vec<String>,
    all: bool,
    verify: bool,
    dry_run: bool,
    force: bool,
}

fn parse_vault_sub(args: &[String]) -> Result<VaultSubOpts, String> {
    let mut o = VaultSubOpts {
        dir: std::env::var("ONTIC_VAULT").unwrap_or_else(|_| ".ontic/vault".to_string()),
        out: "vault.nous".to_string(),
        names: Vec::new(),
        all: false,
        verify: false,
        dry_run: false,
        force: false,
    };
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--dir" => {
                o.dir = args.get(i + 1).ok_or("--dir needs a path")?.clone();
                i += 2;
            }
            "--out" => {
                o.out = args.get(i + 1).ok_or("--out needs a path")?.clone();
                i += 2;
            }
            "--all" => {
                o.all = true;
                i += 1;
            }
            "--verify" => {
                o.verify = true;
                i += 1;
            }
            "--dry-run" => {
                o.dry_run = true;
                i += 1;
            }
            "--force" => {
                o.force = true;
                i += 1;
            }
            other => {
                o.names.push(other.to_string());
                i += 1;
            }
        }
    }
    Ok(o)
}

/// Extract the gen path from a signature (`fn A.b(…) -> T` → `A.b`).
/// Trust badge for listings: `Some(verdict)` renders the tier, `None` = NONE.
fn trust_label(v: &Vault, key: &str) -> String {
    match v.trust(key) {
        Some(t) => format!("{:?}", t.status),
        None => "NONE".to_string(),
    }
}

fn sig_path_of(entry: &VaultEntry) -> String {    let inner = entry
        .signature
        .strip_prefix("fn ")
        .unwrap_or(&entry.signature);
    match inner.find('(') {
        Some(i) => inner[..i].trim().to_string(),
        None => inner.trim().to_string(),
    }
}

use ontic::vault::Entry as VaultEntry;

/// Depth-first dep closure in topological order (deps before dependents).
fn export_closure(v: &Vault, wanted: &[String]) -> Result<Vec<VaultEntry>, String> {
    let mut ordered: Vec<VaultEntry> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut missing: Vec<String> = Vec::new();

    fn visit(
        v: &Vault,
        path: &str,
        ordered: &mut Vec<VaultEntry>,
        seen: &mut std::collections::HashSet<String>,
        missing: &mut Vec<String>,
    ) {
        if seen.contains(path) || !missing.is_empty() {
            return;
        }
        // All versions of this path; prefer a re-verifiable manifest
        // (gen_text present), then the greatest key (latest content).
        let mut candidates: Vec<&VaultEntry> = v
            .list()
            .into_iter()
            .filter(|e| sig_path_of(e) == path)
            .collect();
        candidates.sort_by_key(|e| (e.gen_text.is_some() as i32, e.key.clone()));
        let entry = match candidates.pop() {
            Some(e) => e.clone(),
            None => {
                missing.push(path.to_string());
                return;
            }
        };
        // Deps come from the spec's `use` lines; entries without gen_text
        // have unknown deps — shipped with a warning by the caller.
        let deps: Vec<String> = entry
            .gen_text
            .as_deref()
            .and_then(|t| crate::gen::parse(t).ok())
            .map(|g| g.deps)
            .unwrap_or_default();
        seen.insert(path.to_string());
        for d in deps {
            visit(v, &d, ordered, seen, missing);
            if !missing.is_empty() {
                return;
            }
        }
        // Dedup by key: same kernel reachable via several paths.
        if !ordered.iter().any(|e| e.key == entry.key) {
            ordered.push(entry);
        }
    }

    for name in wanted {
        visit(v, name, &mut ordered, &mut seen, &mut missing);
    }
    if !missing.is_empty() {
        return Err(format!(
            "missing vault dependencies: {}",
            missing.join(", ")
        ));
    }
    Ok(ordered)
}

/// Gather everything an export needs for one entry from the vault dir.
/// The .ous blob is the only carrier of compiled object bytes, so it is
/// required; guarded twins / hpp are optional extras when present.
fn gather_nous_entry(vault_dir: &str, entry: VaultEntry) -> Result<ontic::nous::NousEntry, String> {
    let k8 = entry.key[..8.min(entry.key.len())].to_string();
    let dirp = std::path::Path::new(vault_dir);
    let ous_path = dirp.join(format!("{}-{}.ous", entry.name, k8));
    if !ous_path.exists() {
        return Err(format!(
            "{}-{}.ous not found in {} (re-solve to regenerate)",
            entry.name, k8, vault_dir
        ));
    }
    let raw =
        std::fs::read(&ous_path).map_err(|e| format!("read {}: {}", ous_path.display(), e))?;
    let un = ontic::ous::unpack(&raw)?;
    if un.manifest["key"].as_str() != Some(entry.key.as_str()) {
        return Err(format!("key mismatch inside {} ", ous_path.display()));
    }

    let mut quality = "unknown".to_string();
    let mut manifest_raw = "{}".to_string();
    if let Ok(mraw) = std::fs::read_to_string(dirp.join(format!("{}.json", entry.key))) {
        if let Ok(m) = serde_json::from_str::<serde_json::Value>(&mraw) {
            if let Some(q) = m["quality"].as_str() {
                quality = q.to_string();
            }
        }
        manifest_raw = mraw;
    }
    let manifest_val: serde_json::Value =
        serde_json::from_str(&manifest_raw).unwrap_or_else(|_| serde_json::json!({}));

    // The full vault manifest ships as an extra so imports restore
    // provenance verbatim (.ous carries only a 4-field summary).
    let mut extras: Vec<(String, Vec<u8>)> =
        vec![("manifest".to_string(), manifest_raw.into_bytes())];
    for (kind, file) in [
        ("guarded_so", format!("lib{}-{}.guarded.so", entry.name, k8)),
        ("guarded_c", format!("{}-{}.guarded.c", entry.name, k8)),
        ("hpp", format!("{}-{}.hpp", entry.name, k8)),
    ] {
        let p = dirp.join(&file);
        if p.exists() {
            let bytes = std::fs::read(&p).map_err(|e| format!("read {}: {}", p.display(), e))?;
            extras.push((kind.to_string(), bytes));
        }
    }

    Ok(ontic::nous::NousEntry {
        manifest: manifest_val,
        entry,
        obj: un.obj_bytes,
        header: un.header_text,
        quality,
        extras,
    })
}

/// `ontic vault export [names…|--all] [--out pkg.nous] [--dir d]`
fn cmd_vault_export(args: &[String]) -> i32 {
    let opts = match parse_vault_sub(args) {
        Ok(o) => o,
        Err(e) => return usage(&e),
    };
    if !opts.all && opts.names.is_empty() {
        return usage("export needs kernel names or --all");
    }
    let v = Vault::open(&opts.dir);
    let wanted: Vec<String> = if opts.all {
        v.list().iter().map(|e| sig_path_of(e)).collect()
    } else {
        opts.names.clone()
    };
    let chain = match export_closure(&v, &wanted) {
        Ok(c) => c,
        Err(e) => return die(&e),
    };

    let mut nous_entries = Vec::new();
    let mut warned_unverifiable = false;
    for entry in chain {
        if entry.gen_text.is_none() && !warned_unverifiable {
            eprintln!("warning: some manifests lack gen_text; those kernels export attest-only");
            warned_unverifiable = true;
        }
        match gather_nous_entry(&opts.dir, entry) {
            Ok(ne) => nous_entries.push(ne),
            Err(e) => return die(&e),
        }
    }

    let packed = match ontic::nous::pack(&nous_entries) {
        Ok(p) => p,
        Err(e) => return die(&e),
    };
    let out_path = std::path::Path::new(&opts.out);
    if let Err(e) = ontic::nous::write_to(out_path, &packed) {
        return die(&e);
    }
    println!(
        "PACKED   : {} ({} kernels, {} bytes)",
        opts.out,
        nous_entries.len(),
        packed.len()
    );
    for ne in &nous_entries {
        println!(
            "  {}  {}  [{}]  {}",
            &ne.entry.key[..12.min(ne.entry.key.len())],
            ne.entry.name,
            ne.quality,
            ne.entry.signature
        );
    }
    0
}

/// Land one unpacked entry into the local vault (files + trust status).
fn land_entry(
    v: &mut Vault,
    dir: &str,
    ne: &ontic::nous::NousEntry,
    status: &str,
) -> Result<(), String> {
    let k8 = ne.entry.key[..8.min(ne.entry.key.len())].to_string();
    let dirp = std::path::Path::new(dir);
    std::fs::write(dirp.join(format!("{}.mlir", ne.entry.key)), &ne.entry.mlir)
        .map_err(|e| format!("mlir write failed: {}", e))?;
    // Full manifest ships as a "manifest" extra; fall back to a minimal
    // reconstruction for foreign packages that lack it.
    let manifest_json = match ne.extras.iter().find(|(k, _)| k == "manifest") {
        Some((_, bytes)) => bytes.clone(),
        None => serde_json::to_string_pretty(&serde_json::json!({
            "name": ne.entry.name,
            "signature": ne.entry.signature,
            "key": ne.entry.key,
            "sketch": ne.entry.sketch_text,
            "gen_text": ne.entry.gen_text,
        }))
        .map_err(|e| e.to_string())?
        .into_bytes(),
    };
    std::fs::write(dirp.join(format!("{}.json", ne.entry.key)), manifest_json)
        .map_err(|e| format!("manifest write failed: {}", e))?;
    // Header + object-derived artifacts.
    std::fs::write(dirp.join(format!("{}-{}.h", ne.entry.name, k8)), &ne.header)
        .map_err(|e| format!("header write failed: {}", e))?;
    let obj_name = dirp.join(format!("{}-{}.o", ne.entry.name, k8));
    std::fs::write(&obj_name, &ne.obj).map_err(|e| format!("obj write failed: {}", e))?;
    for (kind, bytes) in &ne.extras {
        let name = match kind.as_str() {
            "manifest" => continue, // already landed as {key}.json
            "guarded_so" => format!("lib{}-{}.guarded.so", ne.entry.name, k8),
            "guarded_c" => format!("{}-{}.guarded.c", ne.entry.name, k8),
            "hpp" => format!("{}-{}.hpp", ne.entry.name, k8),
            other => return Err(format!("unknown extra kind `{other}`")),
        };
        std::fs::write(dirp.join(name), bytes).map_err(|e| format!("extra write failed: {}", e))?;
    }
    // Import provenance is vouching, not a machine proof: `attested`
    // stays false until a z3 stamp lands locally (GR1 wall).
    let stamp = ontic::vault::ProofStamp {
        reason: format!("nous import: {status}"),
        details: Vec::new(),
        attested: false,
    };
    v.set_trust(&ne.entry.key, &stamp)
}

/// Build callable binaries for a landed entry: raw `.so` from the shipped
/// object, guarded twin from the shim source + a `__raw`-renamed re-lower
/// of the shipped MLIR (mirrors solve-time guarded builds; the shim owns
/// the public symbol, so the raw object must expose `name__raw`).
/// Warnings never fail the landing.
fn build_import_binaries(ne: &ontic::nous::NousEntry, dirp: &std::path::Path) -> Vec<String> {
    let mut warnings = Vec::new();
    let no_cc = pipeline::find_tool("clang").is_none() && pipeline::find_tool("cc").is_none();
    if no_cc {
        return vec!["no C compiler; shared libraries not built".to_string()];
    }
    let k8 = ne.entry.key[..8.min(ne.entry.key.len())].to_string();
    let obj = dirp.join(format!("{}-{}.o", ne.entry.name, k8));
    let so = dirp.join(format!("lib{}-{}.so", ne.entry.name, k8));
    if let Err(e) = pipeline::link_shared_so(&obj, &[], &so) {
        warnings.push(format!(".so build failed: {e}"));
    }
    if ne.extras.iter().any(|(k, _)| k == "guarded_c") {
        let has_chain =
            pipeline::find_tool("mlir-opt").is_some() && pipeline::find_tool("llc").is_some();
        if !has_chain {
            warnings
                .push("guarded .so skipped: mlir-opt/llc missing for __raw re-lower".to_string());
            return warnings;
        }
        // Rename the kernel to `__raw` exactly like solve-time guard builds.
        let needle = format!("@{}(", ne.entry.name);
        let repl = format!("@{}__raw(", ne.entry.name);
        let renamed = ne.entry.mlir.replacen(&needle, &repl, 1);
        let dir = pipeline::scratch_dir_pub("import_guarded");
        if std::fs::create_dir_all(&dir).is_err() {
            warnings.push("guarded .so skipped: temp dir unavailable".to_string());
            return warnings;
        }
        let mlir_p = dir.join("raw.mlir");
        let ll_p = dir.join("raw_llvm.mlir");
        let o_p = dir.join("raw.o");
        if std::fs::write(&mlir_p, &renamed).is_err() {
            warnings.push("guarded .so skipped: temp write failed".to_string());
            return warnings;
        }
        match pipeline::mlir_to_llvmir(&mlir_p, &ll_p)
            .and_then(|_| pipeline::object_from_ll(&ll_p, &o_p))
        {
            Ok(_) => {
                let shim = dirp.join(format!("{}-{}.guarded.c", ne.entry.name, k8));
                let gso = dirp.join(format!("lib{}-{}.guarded.so", ne.entry.name, k8));
                if let Err(e) = pipeline::link_shared_so(&o_p, &[&shim], &gso) {
                    warnings.push(format!("guarded .so build failed: {e}"));
                }
            }
            Err(e) => warnings.push(format!("guarded .so skipped: re-lower failed: {e}")),
        }
    }
    warnings
}

/// Re-run the sieve over a shipped gen+candidate. Deterministic verdicts:
/// the winner must reproduce the package's content-addressed key.
fn verify_entry(ne: &ontic::nous::NousEntry, v: &Vault) -> Result<(), String> {
    let gen_text = ne
        .entry
        .gen_text
        .as_ref()
        .ok_or("manifest lacks gen_text; cannot verify")?;
    let w = crate::gen::parse(gen_text).map_err(|e| format!("gen reparse failed: {e}"))?;
    let expected = Vault::key_for(&w);
    if expected != ne.entry.key {
        return Err(format!(
            "canonical key drift: package claims {}, spec hashes to {}",
            ne.entry.key, expected
        ));
    }
    // Resolve declared deps against the LOCAL vault so chained gens verify.
    let mut deps: interp::DepMap = std::collections::HashMap::new();
    for d in &w.deps {
        if let Some(e) = v.find_by_path(d) {
            if let Ok(cand) = crate::sketch::parse(&e.sketch_text) {
                deps.insert(d.clone(), interp::DepFn { cand });
            }
        } else {
            return Err(format!("dependency `{d}` not present locally"));
        }
    }
    let texts = vec![(ne.entry.name.clone(), ne.entry.sketch_text.clone())];
    let report = sieve::run(&w, &texts, &sieve::SiegeConfig::default(), &deps)
        .map_err(|e| format!("gen invalid on this machine: {e:?}"))?;
    if report.survivors.is_empty() {
        let why = report
            .rejections
            .first()
            .map(|(_, r)| format!("{:?} / {:?}", r.stage, r.kind))
            .unwrap_or_else(|| "no survivors".to_string());
        return Err(format!("sieve rejected: {why}"));
    }
    Ok(())
}

/// `ontic vault import pkg.nous [--verify] [--dry-run] [--force] [--dir d]`
fn cmd_vault_import(args: &[String]) -> i32 {
    let opts = match parse_vault_sub(args) {
        Ok(o) => o,
        Err(e) => return usage(&e),
    };
    let pkg_path = match opts.names.first() {
        Some(p) => p.clone(),
        None => return usage("import needs a .nous package path"),
    };
    let raw = match std::fs::read(&pkg_path) {
        Ok(r) => r,
        Err(e) => return die(&format!("read {pkg_path}: {e}")),
    };
    let pkg = match ontic::nous::unpack(&raw) {
        Ok(p) => p,
        Err(e) => return die(&format!("{pkg_path}: {e}")),
    };
    println!(
        "PACKAGE  : {} | generator {} | target {}",
        pkg_path, pkg.generator, pkg.target
    );
    if opts.dry_run {
        for ne in &pkg.entries {
            println!(
                "  {}  {}  [{}]  verifiable={}  guarded={}",
                &ne.entry.key[..12.min(ne.entry.key.len())],
                ne.entry.name,
                ne.quality,
                ne.entry.gen_text.is_some(),
                ne.extras.iter().any(|(k, _)| k == "guarded_so"),
            );
        }
        return 0;
    }
    let mut v = Vault::open(&opts.dir);
    // Topo order is preserved from export; deps land before dependents so
    // --verify can resolve chains locally.
    let mut landed = 0usize;
    let mut verified = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;
    for ne in &pkg.entries {
        if v.get(&ne.entry.key).is_some() && !opts.force {
            println!("SKIP     : {} (key already present)", ne.entry.name);
            skipped += 1;
            continue;
        }
        if opts.verify {
            if let Err(e) = verify_entry(ne, &v) {
                eprintln!("REJECTED : {} — {}", ne.entry.name, e);
                failed += 1;
                continue;
            }
        }
        let status = if opts.verify { "verified" } else { "attested" };
        if let Err(e) = land_entry(&mut v, &opts.dir, ne, status) {
            eprintln!("FAILED   : {} — {}", ne.entry.name, e);
            failed += 1;
            continue;
        }
        let dirp = std::path::Path::new(&opts.dir);
        for w in build_import_binaries(ne, dirp) {
            eprintln!("warning: {}: {}", ne.entry.name, w);
        }
        landed += 1;
        if opts.verify {
            verified += 1;
        }
        println!("IMPORTED : {} [{status}]", ne.entry.name);
    }
    println!(
        "SUMMARY  : {landed} imported ({verified} verified), {skipped} skipped, {failed} rejected"
    );
    if failed > 0 {
        1
    } else {
        0
    }
}

/// Print error to stderr and return failure exit code.
fn die(msg: &str) -> i32 {
    eprintln!("{msg}");
    1
}

/// `ontic prove <file.ont> --hand <cand>` — overflow-absence analysis for
/// the proven tier (feature-gated). Reports Proven/Unproven with honest
/// reasons or a trapping parameter witness. Runs only after a candidate
/// exists; it never substitutes for the sieve.
#[cfg(feature = "proven")]
fn cmd_prove(args: &[String]) -> i32 {
    let path = match args.iter().find(|a| !a.starts_with("--")) {
        Some(p) => p.clone(),
        None => return usage("prove needs a .ont file"),
    };
    let hand_idx = match args.iter().position(|a| a == "--hand") {
        Some(i) => i,
        None => return usage("prove needs --hand <candidate-file>"),
    };
    let cand_path = match args.get(hand_idx + 1) {
        Some(p) => p.clone(),
        None => return usage("--hand needs a candidate file path"),
    };
    let src = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => return die(&format!("read {path}: {e}")),
    };
    let file = match recipe::parse_ont(&src) {
        Ok(f) => f,
        Err(e) => return die(&format!("{path}: invalid gen: {e}")),
    };
    let gen = match pick_gen(&file, None) {
        Ok(g) => g,
        Err(e) => return die(&format!("{path}: {e}")),
    };
    let cand_text = match std::fs::read_to_string(&cand_path) {
        Ok(s) => s,
        Err(e) => return die(&format!("read {cand_path}: {e}")),
    };
    let cand = match sketch::parse(&cand_text) {
        Ok(c) => c,
        Err(e) => return die(&format!("{cand_path}: S1 parse failed: {e:?}")),
    };
    if let Err(e) = check::check(&cand) {
        return die(&format!("{cand_path}: S2 typecheck failed: {e}"));
    }
    match ontic::prove::proof_for(&gen, &cand) {
        ontic::prove::Proof::Proven(how) => {
            println!("PROVEN   {}", gen.name);
            println!("  {how}");
            println!("  flag-free codegen eligible (GR11 declared fast tier)");
            0
        }
        ontic::prove::Proof::Unproven(why) => {
            println!("UNPROVEN {}", gen.name);
            println!("  {why}");
            1
        }
    }
}

/// `ontic lint <file.ont>` — static spec-quality findings before forge spend.
fn cmd_lint(path: &str) -> i32 {
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => return die(&format!("read {path}: {e}")),
    };
    let file = match recipe::parse_ont(&src) {
        Ok(f) => f,
        Err(e) => return die(&format!("{path}: invalid gen: {e}")),
    };
    let vault =
        Vault::open(std::env::var("ONTIC_VAULT").unwrap_or_else(|_| ".ontic/vault".to_string()));
    let report = ontic::lint::lint_file(&file.gens, Some(&vault));
    let findings = &report;
    if findings.is_empty() {
        println!("lint: {} gen(s) clean", file.gens.len());
        return 0;
    }
    let mut errs = 0;
    for f in findings.iter() {
        if f.severity == ontic::lint::Severity::Err {
            errs += 1;
        }
        println!("{} [{}] {}: {}", f.severity, f.rule, f.path, f.detail);
    }
    let (info, warn): (usize, usize) = findings.iter().fold((0, 0), |(i, w), f| match f.severity {
        ontic::lint::Severity::Info => (i + 1, w),
        ontic::lint::Severity::Warn => (i, w + 1),
        ontic::lint::Severity::Err => (i, w),
    });
    println!(
        "lint: {} finding(s) — {errs} err, {warn} warn, {info} info",
        findings.len()
    );
    if errs > 0 {
        1
    } else {
        0
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

/// Auto-emit .ous bundle after successful vault.
fn emit_ous(
    w: &gen::Gen,
    survivor: &sieve::Survivor,
    resolved: &ResolvedDeps,
    key: &str,
    vault_dir: &str,
) {
    if pipeline::find_tool("llc").is_none() {
        return;
    }
    let cand_m = match lower::emit_fn(
        &survivor.candidate.name,
        &survivor.candidate.params,
        &survivor.candidate.ret,
        &survivor.candidate.body,
        &resolved.calls,
    ) {
        Ok(m) => m,
        Err(_) => return,
    };
    let mut parts = resolved.mlirs.clone();
    parts.push(cand_m.clone());
    let composite = lower::compose_modules(&parts).unwrap_or_else(|_| cand_m.clone());

    let dir = std::env::temp_dir().join(format!("ontic-ous-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let mlir_p = dir.join("composite.mlir");
    let ll_p = dir.join("c_llvm.mlir");
    let o_p = dir.join("c.o");
    if std::fs::write(&mlir_p, &composite).is_err() {
        return;
    }
    if pipeline::mlir_to_llvmir(&mlir_p, &ll_p).is_err() {
        return;
    }
    if pipeline::object_from_ll(&ll_p, &o_p).is_err() {
        return;
    }
    let obj_bytes = match std::fs::read(&o_p) {
        Ok(b) => b,
        Err(_) => return,
    };

    let hdr = lower::emit_header(
        &survivor.candidate.name,
        &survivor.candidate.params,
        &survivor.candidate.ret,
        &key[..8.min(key.len())],
        false,
    )
    .unwrap_or_default();

    let entry = ontic::vault::Entry {
        key: key.to_string(),
        name: survivor.candidate.name.clone(),
        signature: String::new(),
        sketch_text: survivor.source_text.clone(),
        gen_text: Some(w.source.clone()),
        mlir: cand_m.clone(),
        proof: None,
        tier: "checked".to_string(),
    };
    let ous_data = ontic::ous::pack_full(&entry, &obj_bytes, &hdr);
    let ous_name = format!(
        "{}-{}.ous",
        survivor.candidate.name,
        &key[..8.min(key.len())]
    );
    let ous_path = std::path::Path::new(vault_dir).join(&ous_name);
    match std::fs::write(&ous_path, &ous_data) {
        Ok(()) => println!("OUS     : {}", ous_path.display()),
        Err(e) => eprintln!("ous write: {}", e),
    }
}

/// Compile a composite MLIR module into a self-contained shared library
/// inside `vault_dir` (thin wrapper over pipeline::build_shared_so).
fn build_shared_lib(
    composite_mlir: &str,
    _fn_name: &str,
    _dep_mlirs: &[String],
    vault_dir: &str,
    lib_name: &str,
) -> Result<(), String> {
    let so_p = std::path::Path::new(vault_dir).join(lib_name);
    ontic::pipeline::build_shared_so(composite_mlir, &so_p)
}

/// Extract individual func.func chunks from a full module text.
/// Strips the outer module{} wrapper, then chunks at each top-indented
/// func.func boundary. Chunks retain their indentation.
fn split_module_funcs(full_module: &str) -> Vec<String> {
    let inner = full_module
        .trim()
        .strip_prefix("module {")
        .and_then(|x| x.strip_suffix('}'))
        .unwrap_or(full_module);
    let mut chunks: Vec<String> = Vec::new();
    let mut cur = String::new();
    for line in inner.lines() {
        if line.starts_with("  func.func ") && !cur.trim().is_empty() {
            chunks.push(std::mem::take(&mut cur));
        }
        if !cur.trim().is_empty() || line.starts_with("  func.func ") {
            cur.push_str(line);
            cur.push('\n');
        }
    }
    if !cur.trim().is_empty() {
        chunks.push(cur);
    }
    chunks
}

/// `ontic lib build <file.ont>` — solve ALL gens in the file sequentially
/// and emit ONE composite shared library + combined header.
fn cmd_lib_build(args: &[String]) -> i32 {
    let path = match args
        .iter()
        .position(|a| a == "build")
        .and_then(|p| args.get(p + 1))
    {
        Some(p) => p.clone(),
        None => return usage("lib build needs a .ont file"),
    };
    let samples = args
        .iter()
        .position(|a| a == "--samples")
        .and_then(|i| args.get(i + 1).and_then(|v| v.parse::<usize>().ok()))
        .unwrap_or(32);
    let backend = args
        .iter()
        .position(|a| a == "--sampler-backend")
        .and_then(|i| args.get(i + 1).cloned())
        .or_else(|| std::env::var("ONTIC_SAMPLER").ok());

    let file = match load_file(&path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("invalid gen file: {}", e);
            return 1;
        }
    };
    if file.gens.is_empty() {
        eprintln!("no gens in {}", path);
        return 1;
    }

    let opts = SolveOpts {
        wish_path: path.clone(),
        wish_sel: None,
        hand: Vec::new(),
        samples,
        seed: 0x5EED,
        forge: None,
        sampler_backend: backend,
        endpoint: None,
        model: None,
        api_key_env: None,
    };
    let _ = &opts.wish_sel;
    let fcfg = forge_config(&opts);
    let cfg = SiegeConfig::default();

    let vault_dir = std::env::var("ONTIC_VAULT").unwrap_or_else(|_| ".ontic/vault".to_string());
    let mut v = Vault::open(&vault_dir);

    let dir = std::env::temp_dir().join(format!(
        "ontic-libbuild-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0)
    ));
    let _ = std::fs::create_dir_all(&dir);

    let mut all_funcs: Vec<String> = Vec::new();
    let mut headers: Vec<String> = Vec::new();
    let mut members: Vec<String> = Vec::new();

    // Solve each gen: cache-first, forge fallback.
    for g in &file.gens {
        println!("== gen {} ==", g.path);
        let key = Vault::key_for(g);

        // Cache hit: use stored mlir + sketch.
        if v.get(&key).is_some() {
            let entry = v.get(&key).expect("checked");
            let cand = match crate::sketch::parse(&entry.sketch_text) {
                Ok(c) => c,
                Err(pe) => {
                    eprintln!(
                        "cached sketch unparsable for {}: {} at {}",
                        g.path, pe.message, pe.offset
                    );
                    return 1;
                }
            };
            check::check(&cand).unwrap();
            let m = lower::emit_fn(
                &cand.name,
                &cand.params,
                &cand.ret,
                &cand.body,
                &lower::CallMap::new(),
            )
            .unwrap();
            for chunk in split_module_funcs(&m) {
                if !all_funcs.contains(&chunk) {
                    all_funcs.push(chunk);
                }
            }
            let h = lower::emit_header(&g.name, &g.params, &g.ret, &key[..8.min(key.len())], false)
                .unwrap();
            headers.push(h);
            members.push(key.clone());
            println!("  cache hit ({})", &key[..12.min(key.len())]);
            continue;
        }

        // Cache miss: forge + sieve.
        println!("  solving {} ...", g.path);
        let texts = match forge::sample(g, &fcfg, &[], "") {
            Ok((t, usage)) => {
                println!(
                    "  tokens : prompt={} completion={}",
                    usage.prompt, usage.completion
                );
                t.into_iter()
                    .enumerate()
                    .map(|(i, x)| (format!("forge-{}", i), x))
                    .collect::<Vec<_>>()
            }
            Err(e) => {
                eprintln!("forge failed for {}: {}", g.path, e);
                return 1;
            }
        };
        let empty_deps = interp::DepMap::new();
        let report = match sieve::run(g, &texts, &cfg, &empty_deps) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("sieve precondition failed: {}", e);
                return 1;
            }
        };
        let survivor = match report.survivors.first() {
            Some(s) => s,
            None => {
                for (label, rej) in &report.rejections {
                    eprintln!(
                        "  KILLED {} {} / {} : {}",
                        label,
                        rej.stage.label(),
                        rej.kind.label(),
                        rej.reason
                    );
                }
                eprintln!("no survivor for {} — library build aborted", g.path);
                return 1;
            }
        };
        let cand = &survivor.candidate;
        let m = lower::emit_fn(
            &cand.name,
            &cand.params,
            &cand.ret,
            &cand.body,
            &lower::CallMap::new(),
        )
        .unwrap();
        let inner = m
            .strip_prefix("module {\n")
            .and_then(|x| x.strip_suffix('}'))
            .unwrap_or(&m);
        for chunk in split_module_funcs(&format!("module {{\n{}}}", inner)) {
            if !all_funcs.contains(&chunk) {
                all_funcs.push(chunk);
            }
        }
        let h = lower::emit_header(&g.name, &g.params, &g.ret, &key[..8.min(key.len())], false)
            .unwrap();
        headers.push(h);
        members.push(key.clone());
        let k2 = Vault::key_for(g);
        let params2: Vec<String> = g
            .params
            .iter()
            .map(|(n, t)| format!("%{}: {}", n, t.name()))
            .collect();
        let sig2 = format!("fn {}({}) -> {}", g.path, params2.join(", "), g.ret.name());
        if let Err(e) = v.put(
            &k2,
            &g.name,
            &sig2,
            &survivor.source_text,
            &m,
            Some(g.source.as_str()),
        ) {
            eprintln!("vault put failed: {e}");
        }
        println!("  solved+vaulted ({})", &k2[..12.min(k2.len())]);
    }

    // Compose final module.
    let mut composite = String::from("module {\n");
    for f in &all_funcs {
        composite.push_str(f);
        composite.push('\n');
    }
    composite.push('}');

    let stem = std::path::Path::new(&path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "lib".into());
    let so_name = format!("lib{}.so", stem);
    let hdr_name = format!("{}.h", stem);
    let so_path = std::path::Path::new(&vault_dir).join(&so_name);

    if let Err(e) = pipeline::build_shared_so(&composite, &so_path) {
        eprintln!("shared library build failed: {}", e);
        return 1;
    }

    // Combined header: guards from stem, all signatures inside extern C.
    let guard = format!(
        "ONTIC_{}_H",
        stem.chars()
            .map(|c| if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            })
            .collect::<String>()
    );
    let mut header_out = format!(
        "// Ontic library bundle (verified; do not edit - re-solve instead)\n// ABI v1 Flat-MemRef\n#ifndef {guard}\n#define {guard}\n\n#ifdef __cplusplus\nextern \"C\" {{\n#endif\n\n"
    );
    for h in &headers {
        // Strip per-kernel guards/extern wrappers; keep declarations.
        for line in h.lines() {
            let t = line.trim();
            if t.starts_with("//")
                || t.starts_with("#ifndef")
                || t.starts_with("#define ONTIC")
                || t.starts_with("#ifdef __cplusplus")
                || t.starts_with("extern \"C\"")
                || t == "}"
                || t.starts_with("#endif")
                || t.is_empty()
            {
                continue;
            }
            header_out.push_str(t);
            header_out.push('\n');
        }
    }
    header_out.push_str("\n#ifdef __cplusplus\n}\n#endif\n\n#endif /* ");
    header_out.push_str(&guard);
    header_out.push_str(" */\n");

    let hdr_path = std::path::Path::new(&vault_dir).join(&hdr_name);
    if let Err(e) = std::fs::write(&hdr_path, &header_out) {
        eprintln!("header write failed: {}", e);
        return 1;
    }

    // Bundle manifest.
    let bundle = serde_json::json!({
        "bundle": stem,
        "members": members,
        "lib": so_name,
        "header": hdr_name,
    });
    let bp = std::path::Path::new(&vault_dir).join(format!("{}.bundle.json", stem));
    if let Err(e) = std::fs::write(
        &bp,
        serde_json::to_string_pretty(&bundle).unwrap_or_default(),
    ) {
        eprintln!("bundle manifest write failed: {}", e);
        return 1;
    }

    println!("LIBRARY : {}/{}", vault_dir, so_name);
    println!("HEADER  : {}/{}", vault_dir, hdr_name);
    0
}

/// `ontic key <file.ont> [--gen Path]` — print the canonical SHA-256 for
/// a gen. Sole key authority: external tools (pyous) shell out to this
/// instead of reimplementing canonical serialization.
fn cmd_key(path: &str, sel: Option<&str>) -> i32 {
    match load_file(path).and_then(|f| pick_gen(&f, sel)) {
        Ok(g) => {
            println!("{}", Vault::key_for(&g));
            0
        }
        Err(e) => {
            eprintln!("invalid gen: {}", e);
            1
        }
    }
}

/// Path of the graduation manifest (which gens form the stdlib).
fn lib_manifest_path() -> String {
    let dir = std::env::var("ONTIC_LIB_DIR").unwrap_or_else(|_| ".ontic".to_string());
    format!("{}/lib.manifest", dir.trim_end_matches('/'))
}

fn read_lib_manifest() -> Vec<String> {
    std::fs::read_to_string(lib_manifest_path())
        .map(|s| {
            s.lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn write_lib_manifest(entries: &[String]) -> Result<(), String> {
    let mut sorted: Vec<String> = entries.to_vec();
    sorted.sort();
    sorted.dedup();
    std::fs::create_dir_all(
        std::path::Path::new(&lib_manifest_path())
            .parent()
            .unwrap_or(std::path::Path::new(".")),
    )
    .map_err(|e| e.to_string())?;
    std::fs::write(lib_manifest_path(), sorted.join("\n") + "\n").map_err(|e| e.to_string())
}

/// `ontic lib ...` — graduation of verified gens into the stdlib.
fn cmd_lib(args: &[String]) -> i32 {
    match args.get(2).map(|s| s.as_str()) {
        Some("ls") | None => {
            let promoted = read_lib_manifest();
            if promoted.is_empty() {
                println!("stdlib empty (promote with: ontic lib promote <Path>)");
            }
            for p in promoted {
                println!("{}", p);
            }
            0
        }
        Some("build") => cmd_lib_build(args),
        Some("promote") => match args.get(3) {
            Some(p) => {
                let mut m = read_lib_manifest();
                if m.iter().any(|x| x == p) {
                    println!("already promoted: {}", p);
                    return 0;
                }
                m.push(p.clone());
                match write_lib_manifest(&m) {
                    Ok(_) => {
                        println!("PROMOTED {}", p);
                        0
                    }
                    Err(e) => {
                        eprintln!("{}", e);
                        1
                    }
                }
            }
            None => usage("lib promote needs a gen path"),
        },
        Some("demote") => match args.get(3) {
            Some(p) => {
                let m = read_lib_manifest();
                let kept: Vec<String> = m.into_iter().filter(|x| x != p).collect();
                match write_lib_manifest(&kept) {
                    Ok(_) => {
                        println!("DEMOTED {}", p);
                        0
                    }
                    Err(e) => {
                        eprintln!("{}", e);
                        1
                    }
                }
            }
            None => usage("lib demote needs a gen path"),
        },
        Some(other) => usage(&format!("unknown lib command `{}`", other)),
    }
}

/// `ontic ablate <file> --samples K` — run the uniform enumeration baseline
/// against the configured LLM sampler on identical evidence, and print the
/// per-stage survival comparison. The control experiment for THE WALL.
fn cmd_ablate(args: &[String]) -> i32 {
    let mut opts = match parse_solve_args(args) {
        Ok(o) => o,
        Err(e) => return usage(&e),
    };
    // Ablation always includes uniform as one arm.
    if opts.sampler_backend.is_none() {
        opts.sampler_backend = Some("llama".to_string());
    }
    let w = match load_file(&opts.wish_path).and_then(|f| pick_gen(&f, opts.wish_sel.as_deref())) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("invalid gen: {}", e);
            return 1;
        }
    };
    let cfg = SiegeConfig::default();
    let resolved = resolve_deps(&w);

    let arms: Vec<(String, forge::Backend)> = vec![
        ("uniform".to_string(), forge::Backend::Uniform),
        (
            opts.sampler_backend.clone().unwrap_or_default(),
            match opts.sampler_backend.as_deref() {
                Some("openai") => forge::Backend::OpenAICompat,
                Some("gemini") | Some("gemini-native") => forge::Backend::GeminiNative,
                _ => forge::Backend::Llama,
            },
        ),
    ];

    let mut rows: Vec<(String, usize, [usize; 7], usize)> = Vec::new();
    for (_label, backend) in &arms {
        let mut fcfg = forge_config(&opts);
        fcfg.backend = *backend;
        println!(
            "== arm {:<8} : {} samples ==",
            fcfg.backend.label(),
            fcfg.samples
        );
        let texts = match forge::sample(&w, &fcfg, &[], &dep_block(&resolved)) {
            Ok((t, _u)) => t,
            Err(e) => {
                eprintln!("sampler failed: {}", e);
                rows.push((fcfg.backend.label().to_string(), 0, [0; 7], 0));
                continue;
            }
        };
        let labeled: Vec<(String, String)> = texts
            .into_iter()
            .enumerate()
            .map(|(i, t)| (format!("{}-{}", fcfg.backend.label(), i), t))
            .collect();
        let report = match sieve::run(&w, &labeled, &cfg, &resolved.map) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("sieve precondition failed: {}", e);
                return 1;
            }
        };
        let mut stages = [0usize; 7];
        for (_, rej) in &report.rejections {
            let idx = match rej.stage {
                sieve::Stage::Parse => 0,
                sieve::Stage::WellFormed => 1,
                sieve::Stage::Transparent => 2,
                sieve::Stage::HeldOut => 3,
                sieve::Stage::Probe => 4,
                sieve::Stage::Shape => 5,
                sieve::Stage::Bench => 6,
            };
            stages[idx] += 1;
        }
        rows.push((
            fcfg.backend.label().to_string(),
            report.rejections.len(),
            stages,
            report.survivors.len(),
        ));
    }

    println!("\nABLATION ({}, {} samples/arm)", w.path, opts.samples);
    println!(
        "{:<10} {:>6} {:>4} {:>4} {:>4} {:>4} {:>4} {:>4} {:>4} {:>6}",
        "sampler", "cands", "S1", "S2", "S3", "S4", "S5", "S6", "surv", "best"
    );
    for (label, killed, stages, surv) in &rows {
        // Best bench among survivors when toolchain present is already
        // printed per-arm above; here we show survival counts only.
        let _ = killed;
        println!(
            "{:<10} {:>6} {:>4} {:>4} {:>4} {:>4} {:>4} {:>4} {:>4}",
            label,
            opts.samples,
            stages[0],
            stages[1],
            stages[2],
            stages[3],
            stages[4],
            stages[5],
            surv
        );
    }
    0
}

/// `ontic pack <key|Path> -o name.ous` — bundle a vault entry.
fn cmd_pack(args: &[String]) -> i32 {
    let key_or_path = match args
        .iter()
        .position(|a| a == "pack")
        .and_then(|p| args.get(p + 1))
    {
        Some(k) => k.clone(),
        None => return usage("pack needs a key or wish path"),
    };
    let out_path = match args
        .iter()
        .position(|a| a == "-o")
        .and_then(|p| args.get(p + 1))
    {
        Some(o) => o.clone(),
        None => return usage("pack needs -o <output.ous>"),
    };

    let vault_dir = std::env::var("ONTIC_VAULT").unwrap_or_else(|_| ".ontic/vault".to_string());
    let v = Vault::open(&vault_dir);

    // Resolve by key or by path.
    let by_path = v.find_by_path(&key_or_path);
    let entry = if key_or_path.len() >= 16 && !key_or_path.contains('.') {
        v.get(&key_or_path)
    } else {
        by_path.as_ref()
    };
    let entry = match entry {
        Some(e) => e,
        None => {
            eprintln!("kernel `{}` not found in vault", key_or_path);
            return 1;
        }
    };

    let obj_name = format!(
        "lib{}-{}.o",
        entry.name,
        &entry.key[..8.min(entry.key.len())]
    );
    let obj_path = std::path::Path::new(&vault_dir).join(&obj_name);
    let obj_bytes = match std::fs::read(&obj_path) {
        Ok(b) => b,
        Err(_) => {
            // Object not stored separately; rebuild from mlir.
            let dir = std::env::temp_dir().join(format!("ontic-pack-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap_or_default();
            let m_p = dir.join("k.mlir");
            let ll_p = dir.join("k_llvm.mlir");
            let o_p = dir.join("k.o");
            if std::fs::write(&m_p, &entry.mlir).is_err() {
                return 1;
            }
            if ontic::pipeline::mlir_to_llvmir(&m_p, &ll_p).is_err()
                || ontic::pipeline::object_from_ll(&ll_p, &o_p).is_err()
            {
                eprintln!("failed to lower kernel for packing");
                return 1;
            }
            std::fs::read(&o_p).unwrap_or_default()
        }
    };

    // Generate header on-the-fly by parsing the stored candidate sketch.
    let hdr_text = match crate::sketch::parse(&entry.sketch_text)
        .map_err(|e| format!("sketch parse: {} at {}", e.message, e.offset))
        .and_then(|cand| {
            crate::lower::emit_header(
                &cand.name,
                &cand.params,
                &cand.ret,
                &entry.key[..8.min(entry.key.len())],
                false,
            )
        }) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("header generation: {}", e);
            return 1;
        }
    };

    let packed = ontic::ous::pack_full(&entry, &obj_bytes, &hdr_text);
    match std::fs::write(&out_path, &packed) {
        Ok(_) => {
            println!("PACKED {} ({} bytes)", out_path, packed.len());
            0
        }
        Err(e) => {
            eprintln!("write {}: {}", out_path, e);
            1
        }
    }
}

/// `ontic unpack x.ous -d dir` — extract artifacts from an .ous bundle.
fn cmd_unpack(args: &[String]) -> i32 {
    let src_path = match args
        .iter()
        .position(|a| a == "unpack")
        .and_then(|p| args.get(p + 1))
    {
        Some(s) => s.clone(),
        None => return usage("unpack needs an .ous file"),
    };
    let out_dir = match args
        .iter()
        .position(|a| a == "-d")
        .and_then(|p| args.get(p + 1))
    {
        Some(d) => d.clone(),
        None => ".".to_string(),
    };

    let data = match std::fs::read(&src_path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("read {}: {}", src_path, e);
            return 1;
        }
    };
    let unpacked = match ontic::ous::unpack(&data) {
        Ok(u) => u,
        Err(e) => {
            eprintln!("unpack: {}", e);
            return 1;
        }
    };

    std::fs::create_dir_all(&out_dir).unwrap_or_default();

    let name = unpacked.manifest["name"]
        .as_str()
        .unwrap_or("kernel")
        .to_string();
    let h_name = format!("{}.h", name);
    let files: Vec<(&str, Vec<u8>)> = vec![
        (
            "manifest.json",
            serde_json::to_vec_pretty(&unpacked.manifest).unwrap_or_default(),
        ),
        ("sketch.sketch", unpacked.sketch_text.clone().into_bytes()),
        ("kernel.mlir", unpacked.mlir.clone().into_bytes()),
        ("kernel.o", unpacked.obj_bytes.clone()),
        (&h_name, unpacked.header_text.clone().into_bytes()),
    ];
    for (fname, bytes) in &files {
        let p = std::path::Path::new(&out_dir).join(fname);
        if std::fs::write(&p, bytes).is_err() {
            return 1;
        }
        println!("EXTRACTED {}", p.display());
    }

    // Build .so directly from the embedded object.
    let so_path = std::path::Path::new(&out_dir).join(format!("lib{}.so", name));
    let cc = match ontic::pipeline::find_tool("clang") {
        Some(c) => c,
        None => std::path::PathBuf::from("clang"),
    };
    let obj_path = std::path::Path::new(&out_dir).join("kernel.o");
    let out = Command::new(&cc)
        .arg("-shared")
        .arg("-O2")
        .arg(obj_path.to_str().unwrap())
        .arg("-o")
        .arg(so_path.to_str().unwrap())
        .output()
        .map_err(|e| format!("cc spawn: {}", e));
    match out {
        Ok(o) if o.status.success() => println!("LIB     : {}", so_path.display()),
        Ok(o) => eprintln!("link: {}", String::from_utf8_lossy(&o.stderr)),
        _ => {}
    }
    0
}

// ============================ decompose ==================================

use ontic::ask::{self, SpecSource};

/// `ontic decompose <paper.txt|-> [flags]` — paper text to a tree of .ont
/// files, gated once by a human, then solved leaves-first with budgeted
/// per-node repair. THE WALL holds: model output is spec TEXT that passes
/// gen::parse + validate_wish before anything else touches it.
fn cmd_decompose(args: &[String]) -> i32 {
    let input = match args.get(2) {
        Some(p) => p.clone(),
        None => return usage("decompose needs <paper.txt|->"),
    };
    let flag = |name: &str| -> Option<String> {
        args.iter()
            .position(|a| a == name)
            .and_then(|p| args.get(p + 1))
            .cloned()
    };
    let yes = args.iter().any(|a| a == "--yes");
    let outdir = flag("--outdir").unwrap_or_else(|| "decomposed".to_string());
    let repair_rounds: usize = flag("--repair-rounds")
        .and_then(|v| v.parse().ok())
        .unwrap_or(2);
    let recuts: usize = flag("--recuts").and_then(|v| v.parse().ok()).unwrap_or(2);

    let paper = read_paper(&input);
    let mut entries: Vec<String> = Vec::new();
    let vault_dir = std::env::var("ONTIC_VAULT").unwrap_or_else(|_| ".ontic/vault".into());
    let v = Vault::open(&vault_dir);
    for e in v.list() {
        entries.push(format!("{}  # {}", e.name, e.signature));
    }
    entries.sort();

    let sopts = SolveOpts {
        wish_path: String::new(),
        wish_sel: None,
        hand: vec![],
        samples: 1,
        seed: flag("--seed")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0x5EED),
        forge: flag("--forge"),
        sampler_backend: flag("--spec-backend").filter(|s| !s.starts_with("file:")),
        endpoint: flag("--endpoint"),
        model: flag("--model"),
        api_key_env: None,
    };
    let fcfg = forge_config(&sopts);
    let src = match ask::resolve_spec_source(flag("--spec-backend").as_deref(), fcfg.clone()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("decompose: {}", e);
            return 1;
        }
    };

    let prompt = ask::build_decompose_prompt(&paper, &ask::inventory_block(&entries));
    println!(
        "decompose: drafting (backend {})…",
        spec_backend_label(&src)
    );

    let nodes_a = match fetch_validated(&src, &prompt) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("decompose: draft A failed: {}", e);
            return 1;
        }
    };
    // Differential: B compared on normalized signatures; unusable B gets
    // bounded resamples, never silent acceptance.
    let mut b_attempts = 0usize;
    let nodes_b = loop {
        match fetch_validated(&src, &prompt) {
            Ok(nb) => break Some(nb),
            Err(e) => {
                b_attempts += 1;
                if b_attempts > recuts {
                    println!(
                        "differential: draft B unusable after {} attempts ({}); proceeding on A alone",
                        b_attempts, e
                    );
                    break None;
                }
            }
        }
    };
    report_diff(&nodes_a, nodes_b.as_deref());

    print_gate_table(&nodes_a);
    if !yes {
        print!("proceed with this tree? [y/N] ");
        use std::io::Write;
        std::io::stdout().flush().unwrap();
        let mut ans = String::new();
        if std::io::stdin().read_line(&mut ans).is_err() {
            return 1;
        }
        let t = ans.trim().to_ascii_lowercase();
        if !(t == "y" || t == "yes") {
            println!("aborted at gate");
            return 1;
        }
    }

    if std::fs::create_dir_all(&outdir).is_err() {
        eprintln!("decompose: cannot create {}", outdir);
        return 1;
    }
    for (spec, _) in &nodes_a {
        let p = std::path::Path::new(&outdir).join(&spec.filename);
        if let Err(e) = std::fs::write(&p, &spec.text) {
            eprintln!("decompose: write {}: {}", p.display(), e);
            return 1;
        }
        let sidecar = serde_json::json!({
            "backend": spec_backend_label(&src),
            "seed": fcfg.seed,
            "repair_budget": repair_rounds,
            "recut_budget": recuts,
            "prompt_sha256": ontic::sha256::sha256_hex(prompt.as_bytes()),
        });
        let _ = std::fs::write(
            p.with_extension("ask.json"),
            serde_json::to_string_pretty(&sidecar).unwrap(),
        );
        println!("wrote {}", p.display());
    }

    let order = match ask::topo_order(&nodes_a) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("decompose: {}", e);
            return 1;
        }
    };
    let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("ontic"));
    let mut solved = 0usize;
    let mut repair_log: Vec<String> = Vec::new();
    for idx in order {
        let (spec, _) = &nodes_a[idx];
        let path = std::path::Path::new(&outdir).join(&spec.filename);
        println!("\n=== solving {} ===", spec.filename);
        let mut repairs = 0usize;
        loop {
            let sb = flag("--candidate-backend").unwrap_or_else(|| "gemini".into());
            let csamples = flag("--candidate-samples").unwrap_or_else(|| "32".into());
            match std::process::Command::new(&exe)
                .arg("solve")
                .arg(&path)
                .arg("--sampler-backend")
                .arg(&sb)
                .arg("--samples")
                .arg(&csamples)
                .output()
            {
                Ok(o) if o.status.success() => {
                    solved += 1;
                    break;
                }
                Ok(o) => {
                    let stderr_all = String::from_utf8_lossy(&o.stderr).to_string();
                    let tail: String = stderr_all
                        .lines()
                        .rev()
                        .take(6)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect::<Vec<_>>()
                        .join("\n");
                    // Candidate-side failures (sampler found nothing valid)
                    // are not spec defects — spec repair cannot help.
                    if stderr_all.contains("no candidate survived") {
                        eprintln!("{}: no candidate survived the sieve (candidate-side); skipping spec repair", spec.filename);
                        break;
                    }
                    if repairs >= repair_rounds {
                        eprintln!(
                            "{}: solve failed; repair budget exhausted\n{}",
                            spec.filename, tail
                        );
                        break;
                    }
                    repairs += 1;
                    println!(
                        "{}: failed (repair {}/{}); asking spec backend",
                        spec.filename, repairs, repair_rounds
                    );
                    repair_log.push(format!("{} round {}: {}", spec.filename, repairs, tail));
                    match repair_node(&src, &prompt, &spec.filename, &tail) {
                        Some(new_text) => {
                            let _ = std::fs::write(&path, new_text);
                        }
                        None => println!("repair unavailable; retrying solve unchanged"),
                    }
                }
                Err(e) => {
                    eprintln!("{}: solver spawn failed: {}", spec.filename, e);
                    break;
                }
            }
        }
    }

    if ontic::corpus::enabled() {
        let paper_key: String = {
            let full = ontic::sha256::sha256_hex(prompt.as_bytes());
            full[..16].to_string()
        };
        let blocks: String = nodes_a
            .iter()
            .map(|(s, _)| format!("=== file: {} ===\n{}=== end ===\n", s.filename, s.text))
            .collect();
        let mut rec = ontic::corpus::Record::new(
            ontic::corpus::Kind::Spec,
            paper_key,
            spec_backend_label(&src),
            fcfg.model.clone(),
            prompt.clone(),
        )
        .with_winner(&blocks);
        rec.rejects = repair_log
            .iter()
            .map(|t| ontic::corpus::RejectRec {
                text: t.clone(),
                stage: "repair".into(),
                kind: "solve-failed".into(),
                reason: t.clone(),
            })
            .collect();
        ontic::corpus::append(&rec);
    }
    println!("\ndecompose: {}/{} nodes solved", solved, nodes_a.len());
    if solved == nodes_a.len() {
        println!("roots ready — bind via pyous.gen(open(<file>).read())");
        0
    } else {
        1
    }
}

fn read_paper(input: &str) -> String {
    if input == "-" {
        use std::io::Read;
        let mut s = String::new();
        let _ = std::io::stdin().read_to_string(&mut s);
        s
    } else {
        std::fs::read_to_string(input).unwrap_or_default()
    }
}

fn fetch_validated(
    src: &SpecSource,
    prompt: &str,
) -> Result<Vec<(ask::NodeSpec, gen::Gen)>, String> {
    let d = ask::fetch_draft(src, prompt)?;
    let nodes = ask::parse_tree(&d)?;
    let (ok, errs) = ask::validate_nodes_lenient(&nodes);
    if ok.is_empty() {
        return Err(errs.join("; "));
    }
    for e in &errs {
        println!("draft file dropped: {}", e);
    }
    Ok(ok)
}

fn report_diff(a: &[(ask::NodeSpec, gen::Gen)], b: Option<&[(ask::NodeSpec, gen::Gen)]>) {
    match b {
        Some(b) => {
            let diff = ask::draft_diff(&ask::normalize_tree(a), &ask::normalize_tree(b));
            if diff.is_empty() {
                println!("differential: drafts agree on signatures");
            } else {
                println!("differential: DRAFTS DISAGREE\n{}", diff);
            }
        }
        None => println!("differential: skipped (no usable second draft)"),
    }
}

fn repair_node(
    src: &SpecSource,
    prompt: &str,
    filename: &str,
    failure_tail: &str,
) -> Option<String> {
    let rp = format!(
        "{prompt}\n\nYOUR FILE {f} FAILED VALIDATION OR SOLVE:\n{tail}\n\n\
         Re-emit ONLY that file, corrected, in the same === file: === block format.",
        prompt = prompt,
        f = filename,
        tail = failure_tail
    );
    let raw = ask::fetch_draft(src, &rp).ok()?;
    let nodes = ask::parse_tree(&raw).ok()?;
    let text = nodes.into_iter().find(|n| n.filename == filename)?.text;
    ask::validate_nodes(&[ask::NodeSpec {
        filename: filename.to_string(),
        text: text.clone(),
    }])
    .ok()?;
    Some(text)
}

fn print_gate_table(nodes: &[(ask::NodeSpec, gen::Gen)]) {
    println!("\nPROPOSED TREE ({} files):", nodes.len());
    for (spec, g) in nodes {
        let params: Vec<String> = g
            .params
            .iter()
            .map(|(n, t)| format!("%{}: {}", n, t.name()))
            .collect();
        println!(
            "  {:<22} {}({}) -> {}   uses:[{}]  ex={} inv={}",
            spec.filename,
            g.path,
            params.join(", "),
            g.ret.name(),
            g.deps.join(","),
            g.transparent.len(),
            g.invariants.len(),
        );
    }
}

fn spec_backend_label(src: &SpecSource) -> String {
    match src {
        SpecSource::File(p) => format!("file:{}", p),
        SpecSource::Model(c) => c.backend.label().to_string(),
    }
}

// ============================== corpus ===================================

/// `ontic corpus [backfill|stats|export]` — training-corpus tooling.
fn cmd_corpus(args: &[String]) -> i32 {
    match args.get(2).map(|s| s.as_str()) {
        Some("backfill") => corpus_backfill(),
        Some("stats") => corpus_stats(),
        Some("export") => corpus_export(args),
        _ => usage("corpus needs [backfill|stats|export]"),
    }
}

fn corpus_backfill() -> i32 {
    let vault_dir = std::env::var("ONTIC_VAULT").unwrap_or_else(|_| ".ontic/vault".into());
    let v = Vault::open(&vault_dir);
    let entries = v.list();
    let mut n = 0usize;
    // Idempotence: never append a reconstructed record twice.
    let corpus_file = std::path::Path::new(&vault_dir)
        .parent()
        .unwrap_or(std::path::Path::new(".ontic"))
        .join("corpus")
        .join("train.jsonl");
    let mut have_keys: std::collections::HashSet<String> = Default::default();
    if let Ok(raw) = std::fs::read_to_string(&corpus_file) {
        for line in raw.lines() {
            if let Ok(r) = serde_json::from_str::<ontic::corpus::Record>(line) {
                if r.reconstructed {
                    have_keys.insert(r.gen_key);
                }
            }
        }
    }
    for e in &entries {
        // Manifests hold both halves: canonical spec (prompt side) and
        // winning sketch text (completion side).
        let man = std::path::Path::new(&vault_dir).join(format!("{}.json", e.key));
        let man_v: serde_json::Value = match std::fs::read_to_string(&man)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
        {
            Some(v) => v,
            None => continue,
        };
        let canonical = match man_v.get("canonical").and_then(|c| c.as_str()) {
            Some(c) => c.to_string(),
            None => continue,
        };
        let sketch = match man_v.get("sketch").and_then(|c| c.as_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        let g = match gen::parse(&canonical) {
            Ok(g) => g,
            Err(e) => {
                eprintln!("backfill skip: parse: {}", &e);
                continue;
            }
        };
        let resolved = resolve_deps(&g);
        let prompt = forge::build_prompt(&g, &[], &dep_block(&resolved));
        let k = Vault::key_for(&g);
        if have_keys.contains(&k) {
            continue;
        }
        let rec = ontic::corpus::Record::new(
            ontic::corpus::Kind::Solve,
            k.clone(),
            "backfill".to_string(),
            "reconstructed".to_string(),
            prompt,
        )
        .with_winner(&sketch)
        .reconstructed();
        ontic::corpus::append(&rec);
        n += 1;
    }
    println!("corpus backfill: {} records appended", n);
    0
}

fn corpus_stats() -> i32 {
    match ontic::corpus::stats() {
        Ok(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            for k in keys {
                println!("{:<28} {}", k, map[k]);
            }
            0
        }
        Err(e) => {
            eprintln!("corpus stats: {}", e);
            1
        }
    }
}

fn corpus_export(args: &[String]) -> i32 {
    let flag = |name: &str| -> Option<String> {
        args.iter()
            .position(|a| a == name)
            .and_then(|p| args.get(p + 1))
            .cloned()
    };
    let format = flag("--format").unwrap_or_else(|| "chat".into());
    let out_path = match flag("--out") {
        Some(p) => p,
        None => return usage("export needs --out <file>"),
    };
    let excludes: Vec<String> = args
        .iter()
        .position(|a| a == "--exclude-key")
        .and_then(|p| args.get(p + 1))
        .map(|v| v.split(',').map(|s| s.trim().to_string()).collect())
        .unwrap_or_default();

    let vault_dir = std::env::var("ONTIC_VAULT").unwrap_or_else(|_| ".ontic/vault".into());
    let path = std::path::Path::new(&vault_dir)
        .parent()
        .unwrap_or(std::path::Path::new(".ontic"))
        .join("corpus")
        .join("train.jsonl");
    let raw = match std::fs::read_to_string(&path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("corpus export: {}: {}", path.display(), e);
            return 1;
        }
    };
    use std::io::Write;
    let mut out = std::fs::File::create(&out_path).ok();
    let mut written = 0usize;
    let mut skipped = 0usize;
    for line in raw.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let rec: ontic::corpus::Record = match serde_json::from_str(line) {
            Ok(r) => r,
            Err(_) => continue,
        };
        if excludes.iter().any(|k| rec.gen_key.starts_with(k.as_str())) {
            skipped += 1;
            continue;
        }
        let winner = match &rec.winner {
            Some(w) => w.clone(),
            None => continue,
        };
        let obj = match format.as_str() {
            "dpo" => serde_json::json!({
                "gen_key": rec.gen_key,
                "prompt": rec.prompt,
                "chosen": winner,
                "rejected": rec.rejects.first().map(|r| r.text.clone()),
                "reconstructed": rec.reconstructed,
            }),
            _ => serde_json::json!({
                "messages": [
                    {"role": "system", "content": "You author Ontic artifacts. Implementations are proven by the sieve, not by you; follow the contract exactly."},
                    {"role": "user", "content": rec.prompt},
                    {"role": "assistant", "content": winner}
                ],
                "kind": rec.kind,
                "gen_key": rec.gen_key,
                "reconstructed": rec.reconstructed,
            }),
        };
        if let Some(f) = out.as_mut() {
            let _ = writeln!(f, "{}", obj);
            written += 1;
        }
    }
    println!(
        "corpus export: {} records -> {} ({} excluded by key)",
        written, out_path, skipped
    );
    0
}

// ================================ eval ===================================

/// `ontic eval --suite DIR --tag NAME [opts]` — solve held-out gens fresh,
/// score pass@N + best ns/call, persist `.ontic/eval/<tag>.json` for
/// before/after comparison of sampler quality behind the same wall.
fn cmd_eval(args: &[String]) -> i32 {
    let flag = |name: &str| -> Option<String> {
        args.iter()
            .position(|a| a == name)
            .and_then(|p| args.get(p + 1))
            .cloned()
    };
    let suite = match flag("--suite") {
        Some(d) => d,
        None => return usage("eval needs --suite <dir>"),
    };
    let tag = flag("--tag").unwrap_or_else(|| "untagged".into());
    let backend = flag("--sampler-backend").unwrap_or_else(|| "gemini".into());
    let samples = flag("--samples").unwrap_or_else(|| "6".into());
    let trained_on = flag("--trained-on");

    // Contamination guard: keys present in the training corpus are skipped.
    let mut trained_keys: std::collections::HashSet<String> = Default::default();
    if let Some(path) = &trained_on {
        if let Ok(raw) = std::fs::read_to_string(path) {
            for line in raw.lines() {
                if let Ok(r) = serde_json::from_str::<ontic::corpus::Record>(line) {
                    trained_keys.insert(r.gen_key);
                }
            }
        }
    }

    let mut files: Vec<std::path::PathBuf> = match std::fs::read_dir(&suite) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().map(|x| x == "ont").unwrap_or(false))
            .collect(),
        Err(e) => {
            eprintln!("eval: {}: {}", suite, e);
            return 1;
        }
    };
    files.sort();

    let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("ontic"));
    let mut results: Vec<serde_json::Value> = Vec::new();
    let mut passed = 0usize;
    let mut contaminated = 0usize;
    for f in &files {
        let text = match std::fs::read_to_string(f) {
            Ok(t) => t,
            Err(_) => continue,
        };
        // Files may hold several gens; evaluate each independently.
        let gens = match ontic::recipe::parse_ont(&text) {
            Ok(of) => of.gens,
            Err(e) => {
                eprintln!("eval skip {}: invalid gen: {}", f.display(), e);
                continue;
            }
        };
        for g in gens {
            let key = Vault::key_for(&g);
            if !trained_keys.is_empty() && trained_keys.contains(&key) {
                println!("{:<28} SKIP (contaminated: key in training data)", g.path);
                contaminated += 1;
                results.push(serde_json::json!({
                    "file": f.display().to_string(), "gen_key": key,
                    "path": g.path, "passed": null, "contaminated": true,
                }));
                continue;
            }
            print!("{:<28} ", g.path);
            use std::io::Write;
            std::io::stdout().flush().ok();
            // Collection stays OFF during eval children regardless of .env:
            // held-out solves must never leak into training records.
            let out = std::process::Command::new(&exe)
                .arg("solve")
                .arg(f)
                .arg("--sampler-backend")
                .arg(&backend)
                .arg("--samples")
                .arg(&samples)
                .env("ONTIC_COLLECT", "0")
                .output();
            let (ok, ns) = match out {
                Ok(o) => {
                    let stdout = String::from_utf8_lossy(&o.stdout).to_string();
                    let success = o.status.success();
                    // Best survivor timing from PASS lines.
                    let mut best_ns: Option<u64> = None;
                    for line in stdout.lines() {
                        if line.starts_with("PASS") {
                            let toks: Vec<&str> = line.split_whitespace().collect();
                            if toks.len() >= 3 {
                                if let Ok(v) = toks[2].parse::<u64>() {
                                    best_ns = Some(best_ns.map_or(v, |b: u64| b.min(v)));
                                }
                            }
                        }
                    }
                    (success, best_ns)
                }
                Err(e) => {
                    eprintln!("spawn failed: {}", e);
                    (false, None)
                }
            };
            if ok {
                passed += 1;
            }
            println!(
                "{}{}",
                if ok { "PASS" } else { "FAIL" },
                ns.map(|v| format!(" ({:.1}µs)", v as f64 / 1000.0))
                    .unwrap_or_default()
            );
            results.push(serde_json::json!({
                "file": f.display().to_string(), "gen_key": key,
                "path": g.path, "passed": ok, "best_ns": ns,
            }));
        }
    }

    let scored = results.len() - contaminated;
    let rate = if scored > 0 {
        passed as f64 / scored as f64
    } else {
        0.0
    };
    println!(
        "\neval [{tag}]: {passed}/{scored} passed ({rate:.1}%), {contaminated} contaminated-skips"
    );
    let report = serde_json::json!({
        "tag": tag, "backend": backend, "samples": samples,
        "pass_rate": rate, "results": results,
    });
    let dir = std::path::Path::new(".ontic").join("eval");
    let _ = std::fs::create_dir_all(&dir);
    let out_path = dir.join(format!("{}.json", tag));
    match serde_json::to_string_pretty(&report)
        .map_err(|e| e.to_string())
        .and_then(|s| std::fs::write(&out_path, s).map_err(|e| e.to_string()))
    {
        Ok(_) => println!("eval: persisted {}", out_path.display()),
        Err(e) => eprintln!("eval: persist failed: {}", e),
    }
    0
}

// ================================ sweep ==================================

/// `ontic sweep <topics.txt> [opts]` — corpus growth: one-line kernel
/// requests through the full gate chain (draft → validate → dedup →
/// solve). Records land in the corpus automatically via ONTIC_COLLECT.
fn cmd_sweep(args: &[String]) -> i32 {
    let flag = |name: &str| -> Option<String> {
        args.iter()
            .position(|a| a == name)
            .and_then(|p| args.get(p + 1))
            .cloned()
    };
    let topics_path = match args.get(2) {
        Some(p) => p.clone(),
        None => return usage("sweep needs <topics.txt>"),
    };
    let outdir = flag("--outdir").unwrap_or_else(|| "swept".into());
    let limit: usize = flag("--limit")
        .and_then(|v| v.parse().ok())
        .unwrap_or(usize::MAX);
    let spec_backend = flag("--spec-backend");
    let candidate_backend = flag("--candidate-backend").unwrap_or_else(|| "gemini".into());

    let topics = match std::fs::read_to_string(&topics_path) {
        Ok(t) => t
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|l| l.to_string())
            .collect::<Vec<_>>(),
        Err(e) => {
            eprintln!("sweep: {}: {}", topics_path, e);
            return 1;
        }
    };
    println!("sweep: {} topics", topics.len());

    // Keys already in the corpus: skip duplicates by construction.
    let mut have_keys: std::collections::HashSet<String> = Default::default();
    {
        let vault_dir = std::env::var("ONTIC_VAULT").unwrap_or_else(|_| ".ontic/vault".into());
        let corpus = std::path::Path::new(&vault_dir)
            .parent()
            .unwrap_or(std::path::Path::new(".ontic"))
            .join("corpus")
            .join("train.jsonl");
        if let Ok(raw) = std::fs::read_to_string(&corpus) {
            for line in raw.lines() {
                if let Ok(r) = serde_json::from_str::<ontic::corpus::Record>(line) {
                    have_keys.insert(r.gen_key);
                }
            }
        }
    }

    let sopts = SolveOpts {
        wish_path: String::new(),
        wish_sel: None,
        hand: vec![],
        samples: 1,
        seed: flag("--seed")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0x5EED),
        forge: flag("--forge"),
        sampler_backend: spec_backend.clone().filter(|s| !s.starts_with("file:")),
        endpoint: flag("--endpoint"),
        model: flag("--model"),
        api_key_env: None,
    };
    let fcfg = forge_config(&sopts);
    let src = match ask::resolve_spec_source(spec_backend.as_deref(), fcfg.clone()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("sweep: {}", e);
            return 1;
        }
    };

    if std::fs::create_dir_all(&outdir).is_err() {
        eprintln!("sweep: cannot create {}", outdir);
        return 1;
    }

    let vault_dir = std::env::var("ONTIC_VAULT").unwrap_or_else(|_| ".ontic/vault".into());
    let mut entries: Vec<String> = Vec::new();
    let v = Vault::open(&vault_dir);
    for e in v.list() {
        entries.push(format!("{}  # {}", e.name, e.signature));
    }

    let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("ontic"));
    let mut solved = 0usize;
    let mut drafted = 0usize;
    let mut dupes = 0usize;
    'topics: for (idx, topic) in topics.iter().enumerate() {
        if drafted >= limit {
            break;
        }
        let prompt = format!(
            "{}\n\n{}\n\nREQUEST:\n{}\n",
            include_str!("ask_langref.txt"),
            ask::inventory_block(&entries),
            topic
        );
        let _ = prompt;
        let nodes = loop {
            match ask::fetch_draft(&src, &prompt).and_then(|d| ask::parse_tree(&d)) {
                Ok(n) => break n,
                Err(e) => {
                    eprintln!("sweep[{}] draft failed ({}); retrying once", idx, e);
                    if let Ok(n) = ask::fetch_draft(&src, &prompt).and_then(|d| ask::parse_tree(&d))
                    {
                        break n;
                    }
                    println!("sweep[{}] draft unavailable; skipping", idx);
                    continue 'topics;
                }
            }
        };
        let valid = match ask::validate_nodes_lenient(&nodes) {
            (v, _) if !v.is_empty() => v,
            (_, errs) => {
                eprintln!(
                    "sweep[{}] invalid draft: {}",
                    idx,
                    errs.first().cloned().unwrap_or_default()
                );
                continue;
            }
        };
        for (spec, g) in valid {
            let key = Vault::key_for(&g);
            if have_keys.contains(&key) {
                dupes += 1;
                println!("sweep[{}] duplicate key — skipped", idx);
                continue;
            }
            have_keys.insert(key.clone());
            let path = std::path::Path::new(&outdir).join(&spec.filename);
            std::fs::write(&path, &spec.text).ok();
            // Spec-kind record for the authored contract.
            if ontic::corpus::enabled() {
                let rec = ontic::corpus::Record::new(
                    ontic::corpus::Kind::Spec,
                    format!("{}-{}", &key[..12.min(key.len())], idx),
                    spec_backend_label(&src),
                    fcfg.model.clone(),
                    format!("REQUEST:\n{}\n", topic),
                )
                .with_winner(&spec.text);
                ontic::corpus::append(&rec);
            }
            drafted += 1;
            print!("sweep[{}] {:<28} ", idx, g.path);
            use std::io::Write;
            std::io::stdout().flush().ok();
            let out = std::process::Command::new(&exe)
                .arg("solve")
                .arg(&path)
                .arg("--sampler-backend")
                .arg(&candidate_backend)
                .arg("--samples")
                .arg("6")
                .output();
            match out {
                Ok(o) if o.status.success() => {
                    solved += 1;
                    println!("PASS");
                }
                Ok(o) => {
                    let tail: String = String::from_utf8_lossy(&o.stderr)
                        .lines()
                        .rev()
                        .take(2)
                        .collect::<Vec<_>>()
                        .join(" | ");
                    println!("FAIL ({})", tail);
                }
                Err(e) => println!("spawn failed: {}", e),
            }
        }
    }
    println!(
        "\nsweep: {}/{} topics drafted, {} solved, {} duplicates",
        drafted,
        topics.len(),
        solved,
        dupes
    );
    0
}
