//! Ontic CLI: `check` a gen, `solve` it (hand candidates or forge), `bench`
//! survivors, and inspect the `vault`. Hand-rolled arg parsing — no clap.

use ontic::forge::{self, ForgeConfig};
use ontic::interp;
use ontic::lower;
use ontic::pipeline;
use ontic::program;
use ontic::recipe;
use ontic::sketch;
use ontic::sieve::{self, SiegeConfig};
use ontic::vault::Vault;
use ontic::gen;

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
        Some("bench") => cmd_bench(args),
        Some("vault") => cmd_vault(args),
        Some("lib") => cmd_lib(args),
        Some("ablate") => cmd_ablate(args),
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
  ontic bench <file.ont> [opts]                   rank survivors with timings only
  ontic run <file.ont>                            execute a recipe over vaulted fns
  ontic vault [--dir D]                           list verified functions
  ontic lib [ls|promote <Path>|demote <Path>]     manage graduated stdlib entries
  ontic ablate <file.ont> --samples N             uniform-vs-LLM control experiment
  ontic key <file.ont> [--gen Path]               print canonical SHA-256 key

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
            println!("tier      : {}", if w.wrapping { "wrapping" } else { "checked" });
            if !w.hints.is_empty() {
                println!("hints     : {} (advisory)", w.hints.len());
                for h in &w.hints {
                    println!("  - {}", h);
                }
            }
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
            eprintln!("invalid gen: {}", e);
            1
        }
    }
}

use ontic::probes;

fn probes_count(w: &gen::Gen, cfg: &SiegeConfig) -> usize {
    probes::generate(w, cfg.probe_count, cfg.seed, cfg.edge_budget).len()
}

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
        if matches!(kind, forge::Backend::OpenAICompat | forge::Backend::GeminiNative) {
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
    let w = match load_file(&opts.wish_path).and_then(|f| pick_gen(&f, opts.wish_sel.as_deref()))
    {
        Ok(w) => w,
        Err(e) => {
            eprintln!("invalid gen: {}", e);
            return 1;
        }
    };
    let cfg = SiegeConfig::default();

    let fcfg = forge_config(opts);
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
                fcfg.samples, fcfg.backend.label(), fcfg.model
            );
        }
        match forge::sample(&w, &fcfg, &[]) {
            Ok((texts, usage)) => {
                println!(
                    "tokens  : prompt={} completion={}",
                    usage.prompt, usage.completion
                );
                texts.into_iter().enumerate().map(|(i, t)| (format!("forge-{}", i), t)).collect()
            }
            Err(e) => {
                eprintln!("forge failed: {}", e);
                return 1;
            }
        }
    };

    let resolved = resolve_deps(&w);
    let first_prompt = forge::build_prompt(&w, &[]);
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
        println!("feedback round: {} resamples at T={} ...", fcfg.samples, fcfg.temperature);
        match forge::sample(&w, &fcfg, &feedback) {
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
            let winner = report.survivors.first().unwrap();
            if !store {
                return 0;
            }
            emit_and_store(&w, winner, &resolved, &fcfg, &first_prompt)
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
    for s in survivors.iter() {
        match lower::emit_fn(
            &s.candidate.name,
            &s.candidate.params,
            &s.candidate.ret,
            &s.candidate.body,
            w.wrapping,
            &resolved.calls,
        ) {
            Ok(cand_mlir) => {
                // Candidate + deps compile as ONE composite so intra-module
                // calls resolve at lowering time.
                let mut parts = dep_mlirs.clone();
                parts.push(cand_mlir);
                let mlir =
                    lower::compose_modules(&parts).expect("composite compose");
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
                match pipeline::bench_native(&mlir, &s.candidate.name, &kinds, 2_000, &[]) {
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
    let vault_dir =
        std::env::var("ONTIC_VAULT").unwrap_or_else(|_| ".ontic/vault".to_string());
    let v = match Vault::open(&vault_dir) {
        Ok(v) => v,
        Err(_) => return ResolvedDeps::empty(),
    };
    let mut map = interp::DepMap::new();
    let mut mlirs = Vec::new();
    let mut calls = lower::CallMap::new();
    for path in &w.deps {
        if let Some(entry) = v.find_by_path(path) {
            if let Ok(cand) = ontic::sketch::parse(&entry.sketch_text) {
                let tier = if entry.wrapping {
                    interp::Tier::wrapping()
                } else {
                    interp::Tier::checked()
                };
                // The call symbol is the func name inside the dep's module.
                let symbol = entry
                    .mlir
                    .split("func.func @")
                    .nth(1)
                    .and_then(|r| r.find('('))
                    .map(|i| entry.mlir[..].split("func.func @").nth(1).unwrap()[..i].trim().to_string());
                map.insert(
                    path.clone(),
                    interp::DepFn { cand: cand.clone(), tier },
                );
                if let Some(sym) = symbol {
                    calls.insert(
                        path.clone(),
                        lower::CallTarget {
                            symbol: sym,
                            params: cand.params.iter().map(|(_, t)| *t).collect(),
                            ret: cand.ret,
                        },
                    );
                }
                mlirs.push(entry.mlir.clone());
            }
        }
    }
    ResolvedDeps { map, mlirs, calls }
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
    let mlir = match lower::emit_fn(
        &survivor.candidate.name,
        &survivor.candidate.params,
        &survivor.candidate.ret,
        &survivor.candidate.body,
        w.wrapping,
        &resolved.calls,
    ) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("lowering failed (candidate verified but not emittable): {}", e);
            return 1;
        }
    };
    // Mandatory when the toolchain is present: unvalidated IR never vaults.
    // Candidates calling deps are validated as a COMPOSITE module.
    let staged = std::env::temp_dir().join("ontic-emit-check.mlir");
    let mut parts: Vec<String> = resolved.mlirs.clone();
    parts.push(mlir.clone());
    let validation_text = lower::compose_modules(&parts).unwrap_or(mlir.clone());
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
    let vault_dir = std::env::var("ONTIC_VAULT").unwrap_or_else(|_| ".ontic/vault".to_string());
    let v = match Vault::open(&vault_dir) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{}", e);
            return 1;
        }
    };
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
                        }
                        Err(e) => eprintln!("header write failed: {}", e),
                    },
                    Err(e) => eprintln!("header generation failed: {}", e),
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
        }
    });
    if !artifacts.is_empty() {
        meta_val["artifacts"] = serde_json::Value::Object(artifacts);
    }
    let meta = meta_val;
    match v.put_meta(w, &survivor.source_text, &mlir, &meta) {
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
            let promoted = read_lib_manifest();
            for e in entries {
                let path = {
                    let inner = e.signature.strip_prefix("fn ").unwrap_or(&e.signature);
                    match inner.find('(') {
                        Some(i) => inner[..i].trim().to_string(),
                        None => inner.trim().to_string(),
                    }
                };
                let badge = if promoted.iter().any(|p| *p == path) { " [LIB]" } else { "" };
                println!("{}  {}{}  {}", &e.key[..12.min(e.key.len())], e.name, badge, e.signature);
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
    std::fs::write(lib_manifest_path(), sorted.join("\n") + "\n")
        .map_err(|e| e.to_string())
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
    let w = match load_file(&opts.wish_path)
        .and_then(|f| pick_gen(&f, opts.wish_sel.as_deref()))
    {
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
        let texts = match forge::sample(&w, &fcfg, &[]) {
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
