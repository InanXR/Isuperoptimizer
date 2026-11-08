//! `isuperopt` — the end-to-end CLI: `enumerate` (exhaustive search with
//! optimality certificates), `search` (MCMC), `optimize` (full pipeline),
//! and `beat` (compiler comparison). All results go to the JSONL artifact
//! of record; nothing in any report is ever retyped from memory.

use std::time::Instant;

use isuperoptimizer::benchmarks::{self, Benchmark, Strategy};
use isuperoptimizer::enumerator::{enumerate_benchmark, EnumConfig};
use isuperoptimizer::instr::{parse_program, Program};
use isuperoptimizer::mcmc::{run_chain, McmcConfig};
use isuperoptimizer::reportlog::{JVal, ReportLog};
use isuperoptimizer::state::eval_program;
use isuperoptimizer::verify::{verify, Spec, Verdict};
use isuperoptimizer::z3proc::Z3Session;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        usage();
        std::process::exit(2);
    }
    let cmd = args[0].as_str();
    let out = flag(&args, "--out").unwrap_or_else(|| "artifacts/isuperopt-log.jsonl".to_string());
    let seed: u64 = flag(&args, "--seed").and_then(|s| s.parse().ok()).unwrap_or(0xB0B_0000_0001);

    match cmd {
        "enumerate" => {
            let which = flag(&args, "--bench").unwrap_or_else(|| "enum-all".to_string());
            let names: Vec<&'static Benchmark> = if which == "enum-all" {
                benchmarks::BENCHMARKS
                    .iter()
                    .filter(|b| b.strategy == Strategy::Enumerate { max_length: 0 } || matches!(b.strategy, Strategy::Enumerate { .. }))
                    .collect()
            } else {
                let b = benchmarks::by_name(&which)
                    .unwrap_or_else(|| panic!("unknown benchmark `{}`", which));
                vec![b]
            };
            let mut log = ReportLog::open(&out);
            let mut z3 = Z3Session::new().expect("z3 must be available");
            let mut failures = 0u32;
            let t0 = Instant::now();
            for b in names {
                let r = run_enumeration(b.into(), &mut z3, &mut log, seed);
                if r.is_err() {
                    failures += 1;
                }
            }
            log.event(&[
                ("kind", JVal::S("run_summary".into())),
                ("status", JVal::S(if failures == 0 { "all_passed".into() } else { "failures".into() })),
                ("modes_failed", JVal::U(failures as u64)),
                ("millis", JVal::U(t0.elapsed().as_millis() as u64)),
            ]);
            log.flush();
            println!("\nenumeration run: {} failures", failures);
            println!("artifact of record: {}", log.path);
            std::process::exit(if failures == 0 { 0 } else { 1 });
        }
        "search" => {
            let which = flag(&args, "--bench").unwrap_or_else(|| "mcmc-all".to_string());
            let names: Vec<&'static Benchmark> = if which == "mcmc-all" {
                benchmarks::BENCHMARKS
                    .iter()
                    .filter(|b| b.strategy == Strategy::Mcmc)
                    .collect()
            } else {
                let b = benchmarks::by_name(&which)
                    .unwrap_or_else(|| panic!("unknown benchmark `{}`", which));
                vec![b]
            };
            let opt_seconds: u64 = flag(&args, "--seconds")
                .and_then(|s| s.parse().ok())
                .unwrap_or(90);
            let synth_seconds: u64 = flag(&args, "--synth-seconds")
                .and_then(|s| s.parse().ok())
                .unwrap_or(60);
            let mut log = ReportLog::open(&out);
            let mut failures = 0u32;
            let t0 = Instant::now();
            for b in names {
                let cfg = McmcConfig::paper(seed, opt_seconds, synth_seconds);
                if run_search(b.into(), &cfg, &mut log).is_err() {
                    failures += 1;
                }
            }
            log.event(&[
                ("kind", JVal::S("run_summary".into())),
                ("status", JVal::S(if failures == 0 { "all_passed".into() } else { "failures".into() })),
                ("modes_failed", JVal::U(failures as u64)),
                ("millis", JVal::U(t0.elapsed().as_millis() as u64)),
            ]);
            log.flush();
            println!("\nsearch run: {} failures", failures);
            println!("artifact of record: {}", log.path);
            std::process::exit(if failures == 0 { 0 } else { 1 });
        }
        "optimize" => {
            // The optimize pipeline: every benchmark through its strategy, one
            // certificate artifact for the whole run. recheck --from-log
            // validates every claimed result against the logged testcases.
            let only = flag(&args, "--bench");
            let opt_seconds: u64 = flag(&args, "--seconds")
                .and_then(|s| s.parse().ok())
                .unwrap_or(240);
            let synth_seconds: u64 = flag(&args, "--synth-seconds")
                .and_then(|s| s.parse().ok())
                .unwrap_or(120);
            let mut log = ReportLog::open(&out);
            let mut failures = 0u32;
            let t0 = Instant::now();
            for b in benchmarks::BENCHMARKS {
                if let Some(name) = &only {
                    if *name != b.name {
                        continue;
                    }
                }
                let r = match b.strategy {
                    Strategy::Enumerate { max_length } => {
                        let mut cfg = EnumConfig::for_benchmark(b);
                        cfg.max_length = max_length;
                        match Z3Session::new() {
                            Ok(mut z3) => run_enumeration(b.into(), &mut z3, &mut log, seed),
                            Err(e) => Err(e),
                        }
                    }
                    Strategy::Mcmc => {
                        let cfg = McmcConfig::paper(seed, opt_seconds, synth_seconds);
                        run_search(b.into(), &cfg, &mut log)
                    }
                };
                if r.is_err() {
                    failures += 1;
                }
            }
            log.event(&[
                ("kind", JVal::S("run_summary".into())),
                ("status", JVal::S(if failures == 0 { "all_passed".into() } else { "failures".into() })),
                ("modes_failed", JVal::U(failures as u64)),
                ("millis", JVal::U(t0.elapsed().as_millis() as u64)),
            ]);
            log.flush();
            println!(
                "\noptimize run: {} benchmarks, {} failures ({:.0}s)",
                benchmarks::BENCHMARKS.len(),
                failures,
                t0.elapsed().as_secs_f64()
            );
            println!("artifact of record: {}", log.path);
            println!("verify it: recheck --from-log {}", log.path);
            std::process::exit(if failures == 0 { 0 } else { 1 });
        }
        "beat" => {
            // Run the optimize pipeline against real gcc output. Every
            // baseline is logged (compiler_baseline event) before it is
            // challenged; outcomes are logged honestly whichever way they go.
            // Runs at BOTH -O2 and -O3. `--batch` selects the candidate
            // batch (core|full); `--stragglers` names functions that get the
            // documented second attempt (seeded synthesis from structural
            // variants of the compiler's own form, longer budget).
            let opt_seconds: u64 = flag(&args, "--seconds")
                .and_then(|s| s.parse().ok())
                .unwrap_or(120);
            let synth_seconds: u64 = flag(&args, "--synth-seconds")
                .and_then(|s| s.parse().ok())
                .unwrap_or(60);
            let batch = flag(&args, "--batch").unwrap_or_else(|| "core".to_string());
            let stragglers: Vec<String> = flag(&args, "--stragglers")
                .map(|s| s.split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect())
                .unwrap_or_default();
            let straggler_seconds: u64 = flag(&args, "--straggler-seconds")
                .and_then(|s| s.parse().ok())
                .unwrap_or(300);
            let straggler_variants: usize = flag(&args, "--straggler-variants")
                .and_then(|s| s.parse().ok())
                .unwrap_or(8);
            let repo_root = std::env::current_dir().expect("run from the repo root");
            let mut log = ReportLog::open(&out);
            let mut failures = 0u32;
            let t0 = Instant::now();
            let compiler_version = {
                let o = std::process::Command::new("gcc")
                    .arg("--version")
                    .output()
                    .expect("gcc must be installed");
                String::from_utf8_lossy(&o.stdout).lines().next().unwrap_or("gcc").to_string()
            };
            let (batch_dir, parse_batch, registry): (
                &str,
                fn(&str) -> Result<Vec<(&'static str, Program)>, String>,
                &[isuperoptimizer::compiler_core::CompilerFunc],
            ) = match batch.as_str() {
                "core" => (
                    "fixtures/compiler-core",
                    isuperoptimizer::compiler_core::parse_survivors,
                    isuperoptimizer::compiler_core::CORE_FUNCS,
                ),
                "full" => (
                    "fixtures/compiler-full",
                    isuperoptimizer::compiler_full::parse_survivors,
                    isuperoptimizer::compiler_full::FULL_FUNCS,
                ),
                other => {
                    eprintln!("unknown --batch `{}` (expected core or full)", other);
                    std::process::exit(2);
                }
            };
            for (level, opt_flag) in [("O2", "-O2"), ("O3", "-O3")] {
                let asm_text = std::fs::read_to_string(repo_root.join(batch_dir).join(format!("candidates-{}.s", level)))
                    .expect("batch assembly file");
                let survivors = match parse_batch(&asm_text) {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("FAIL beat ({}): {}", level, e);
                        std::process::exit(1);
                    }
                };
                let mut z3 = match Z3Session::new() {
                    Ok(z) => z,
                    Err(e) => {
                        eprintln!("FAIL beat: {}", e);
                        std::process::exit(1);
                    }
                };
                for (fname, prog) in &survivors {
                    let meta = registry
                        .iter()
                        .find(|m| m.name == *fname)
                        .unwrap();
                    let asm_text_fn = prog.to_string();
                    log.event(&[
                        ("kind", JVal::S("compiler_baseline".into())),
                        ("function", JVal::S(fname.to_string())),
                        ("level", JVal::S(level.into())),
                        ("batch", JVal::S(batch.clone())),
                        ("compiler", JVal::S(compiler_version.clone())),
                        ("flags", JVal::S(format!("{} {}", opt_flag, isuperoptimizer::compiler_core::GCC_FLAGS_BASE))),
                        ("source", JVal::S(meta.source.into())),
                        ("asm", JVal::S(asm_text_fn.clone())),
                        ("nargs", JVal::U(meta.nargs as u64)),
                    ]);
                    println!(
                        "beat {} {}: gcc baseline is {} instructions\n{}",
                        fname, level, prog.0.len(), asm_text_fn
                    );
                    let db = isuperoptimizer::benchmarks::DynBenchmark {
                        name: format!("{}_{}", fname, level),
                        desc: format!("gcc {} {} — {}", level, meta.source, meta.source),
                        spec: asm_text_fn,
                        live_outs: vec![isuperoptimizer::compiler_core::RET_REG],
                        strategy: match meta.enumerate_max {
                            Some(ml) => Strategy::Enumerate { max_length: ml },
                            None => Strategy::Mcmc,
                        },
                    };
                    let r = match db.strategy {
                        Strategy::Enumerate { .. } => {
                            run_enumeration(db.view(), &mut z3, &mut log, seed)
                        }
                        Strategy::Mcmc => {
                            let cfg = McmcConfig::paper(seed, opt_seconds, synth_seconds);
                            run_search(db.view(), &cfg, &mut log)
                        }
                    };
                    if r.is_err() {
                        failures += 1;
                    }
                }
                // The documented straggler second attempt for the
                // named stragglers (must be MCMC-scoped survivors of this
                // batch; each gets seeded-synthesis chains from structural
                // variants of the compiler's own form, longer budget).
                for name in &stragglers {
                    let meta = registry.iter().find(|m| m.name == *name);
                    let prog = survivors.iter().find(|(n, _)| n == name).map(|(_, p)| p);
                    let (meta, prog) = match (meta, prog) {
                        (Some(m), Some(p)) => (m, p),
                        _ => {
                            eprintln!("FAIL straggler {}: not a survivor of batch {} at {}", name, batch, level);
                            failures += 1;
                            continue;
                        }
                    };
                    if meta.enumerate_max.is_some() {
                        eprintln!("NOTE straggler {} is enumerate-scoped; skipping seeded pass (it holds an optimality proof)", name);
                        continue;
                    }
                    let bench = format!("{}_{}", name, level);
                    if let Err(e) = run_seeded_pass(
                        bench,
                        prog.to_string(),
                        meta.source,
                        straggler_seconds,
                        straggler_variants,
                        seed,
                        &mut log,
                    ) {
                        eprintln!("FAIL straggler pass {}: {}", name, e);
                        failures += 1;
                    }
                }
            }
            log.event(&[
                ("kind", JVal::S("run_summary".into())),
                ("status", JVal::S(if failures == 0 { "all_passed".into() } else { "failures".into() })),
                ("modes_failed", JVal::U(failures as u64)),
                ("millis", JVal::U(t0.elapsed().as_millis() as u64)),
            ]);
            log.flush();
            println!("\nbeat run: {} failures", failures);
            println!("artifact of record: {}", log.path);
            std::process::exit(if failures == 0 { 0 } else { 1 });
        }
        other => {
            eprintln!("unknown command `{}`", other);
            usage();
            std::process::exit(2);
        }
    }
}

fn run_enumeration(
    b: isuperoptimizer::benchmarks::AnyBenchmark<'_>,
    z3: &mut Z3Session,
    log: &mut ReportLog,
    seed: u64,
) -> Result<(), String> {
    let t0 = Instant::now();
    println!("enumerating {} — {}", b.name, b.desc);
    let max_length = match b.strategy {
        Strategy::Enumerate { max_length } => max_length,
        _ => return Err(format!("benchmark {} is not an enumeration benchmark", b.name)),
    };
    let mut cfg = EnumConfig::for_spec(b.name, b.spec, b.live_outs, max_length);    cfg.max_length = max_length;
    let outcome = match enumerate_benchmark(&b, &cfg, z3, log, seed) {
        Ok(o) => o,
        Err(e) => {
            log.event(&[
                ("kind", JVal::S("enumeration_summary".into())),
                ("benchmark", JVal::S(b.name.into())),
                ("status", JVal::S(format!("error: {}", e))),
                ("millis", JVal::U(t0.elapsed().as_millis() as u64)),
            ]);
            eprintln!("FAIL {}: {}", b.name, e);
            return Err(e);
        }
    };
    let millis = t0.elapsed().as_millis() as u64;
    let status = if outcome.optimum.is_some() { "pass" } else { "no_optimum_found" };
    log.event(&[
        ("kind", JVal::S("enumeration_summary".into())),
        ("benchmark", JVal::S(b.name.into())),
        ("status", JVal::S(status.into())),
        ("optimal_length", JVal::U(outcome.optimal_length as u64)),
        ("spec_length", JVal::U(outcome.spec_length as u64)),
        ("candidates_total", JVal::U(outcome.candidates_total)),
        ("z3_queries", JVal::U(outcome.z3_queries)),
        ("witnesses_logged", JVal::U(outcome.witnesses_logged)),
        ("testcases_final", JVal::U(outcome.testcases.len() as u64)),
        ("millis", JVal::U(millis)),
    ]);
    match &outcome.optimum {
        Some(p) => {
            println!(
                "  PASS {}: spec {} instr -> optimum {} instr (candidates {}, z3 {}, {} ms)\n    {}",
                b.name,
                outcome.spec_length,
                outcome.optimal_length,
                outcome.candidates_total,
                outcome.z3_queries,
                millis,
                p.to_string().replace('\n', "\n    ")
            );
            Ok(())
        }
        None => {
            println!(
                "  FAIL {}: no equivalent program of length <= {} found (candidates {}, z3 {})",
                b.name, max_length, outcome.candidates_total, outcome.z3_queries
            );
            Err("no optimum".into())
        }
    }
}

fn log_chain_outcomes(
    log: &mut ReportLog,
    b: &isuperoptimizer::benchmarks::AnyBenchmark<'_>,
    outcomes: &[isuperoptimizer::mcmc::ChainOutcome],
    tc_start: u64,
) -> u64 {
    let mut tc_index = tc_start;
    for o in outcomes {
        for (i, tc) in o.all_cases.iter().enumerate() {
            log.event(&[
                ("kind", JVal::S("testcase".into())),
                ("benchmark", JVal::S(b.name.into())),
                ("index", JVal::U(tc_index)),
                ("origin", JVal::S(if i < o.seed_count { "seed".into() } else { "z3_counterexample".into() })),
                ("phase", JVal::S(o.phase.into())),
                ("chain", JVal::U(o.chain as u64)),
                ("inputs", JVal::ArrS(tc.inputs.iter().map(|v| format!("0x{:016x}", v)).collect())),
                ("spec_out", JVal::ArrS(tc.outputs.iter().map(|v| format!("0x{:016x}", v)).collect())),
            ]);
            tc_index += 1;
        }
        log.event(&[
            ("kind", JVal::S("chain_result".into())),
            ("benchmark", JVal::S(b.name.into())),
            ("phase", JVal::S(o.phase.into())),
            ("chain", JVal::U(o.chain as u64)),
            ("found", JVal::B(o.best.is_some())),
            ("best_perf", JVal::U(o.best_perf)),
            ("proposals", JVal::U(o.stats.proposals)),
            ("accepts", JVal::U(o.stats.accepts)),
            ("move_opcode", JVal::U(o.stats.move_opcode)),
            ("move_operand", JVal::U(o.stats.move_operand)),
            ("move_swap", JVal::U(o.stats.move_swap)),
            ("move_instruction", JVal::U(o.stats.move_instruction)),
            ("z3_queries", JVal::U(o.stats.z3_queries)),
            ("z3_equivalent", JVal::U(o.stats.z3_equivalent)),
            ("z3_unknown", JVal::U(o.stats.z3_unknown)),
            ("cegis_additions", JVal::U(o.stats.cegis_additions)),
            ("dropped_testcases", JVal::U(o.dropped_testcases)),
            ("millis", JVal::U(o.millis)),
        ]);
    }
    tc_index
}

fn join_all(
    b: &isuperoptimizer::benchmarks::AnyBenchmark<'_>,
    log: &mut ReportLog,
    handles: Vec<std::thread::JoinHandle<Result<isuperoptimizer::mcmc::ChainOutcome, String>>>,
) -> Result<Vec<isuperoptimizer::mcmc::ChainOutcome>, String> {
    let mut outcomes = Vec::new();
    for h in handles {
        match h.join() {
            Ok(Ok(o)) => outcomes.push(o),
            Ok(Err(e)) => {
                log.event(&[
                    ("kind", JVal::S("mcmc_summary".into())),
                    ("benchmark", JVal::S(b.name.into())),
                    ("status", JVal::S(format!("chain error: {}", e))),
                ]);
                eprintln!("FAIL {}: chain error: {}", b.name, e);
                return Err(e);
            }
            Err(_) => {
                eprintln!("FAIL {}: chain panicked", b.name);
                return Err("chain panicked".into());
            }
        }
    }
    Ok(outcomes)
}

fn run_search(
    b: isuperoptimizer::benchmarks::AnyBenchmark<'_>,
    cfg: &McmcConfig,
    log: &mut ReportLog,
) -> Result<(), String> {
    use isuperoptimizer::mcmc::{program_latency, ChainOutcome};

    println!("searching {} — {} ({} opt chains x {}s, {} synth chains x {}s)",
        b.name, b.desc, cfg.opt_chains, cfg.opt_seconds, cfg.synth_chains, cfg.synth_seconds);

    log.event(&[
        ("kind", JVal::S("bench_start".into())),
        ("benchmark", JVal::S(b.name.into())),
        ("spec", JVal::S(b.spec.into())),
        ("desc", JVal::S(b.desc.into())),
        ("live_outs", JVal::ArrS(b.live_outs.iter().map(|g| g.name().to_string()).collect())),
        ("strategy", JVal::S("mcmc".into())),
    ]);
    log.event(&[
        ("kind", JVal::S("mcmc_config".into())),
        ("benchmark", JVal::S(b.name.into())),
        ("ell", JVal::U(cfg.ell as u64)),
        ("p_opcode", JVal::S(cfg.p_opcode.to_string())),
        ("p_operand", JVal::S(cfg.p_operand.to_string())),
        ("p_swap", JVal::S(cfg.p_swap.to_string())),
        ("p_instruction", JVal::S(cfg.p_instruction.to_string())),
        ("p_unused", JVal::S(cfg.p_unused.to_string())),
        ("beta", JVal::S(cfg.beta.to_string())),
        ("testcases", JVal::U(cfg.testcases as u64)),
        ("immediate_policy", JVal::S(cfg.immediate_policy.into())),
        ("latency_model", JVal::S("static per-opcode table (ADR-007)".into())),
        ("phases", JVal::S("synthesis (eq' only) then optimization (eq' + perf), per ASPLOS'13 4.4".into())),
    ]);

    // Round 1: optimization chains start from the spec; synthesis chains
    // start from random programs (the paper's two phases, §4.4).
    // Owned: `thread::spawn` needs 'static.
    let spec_src: String = b.spec.to_string();
    let mut handles = Vec::new();
    for phase in ["optimization", "synthesis"] {
        let n = if phase == "optimization" { cfg.opt_chains } else { cfg.synth_chains };
        for chain in 0..n {
            let live_outs = b.live_outs.to_vec();
            let cfg2 = cfg.clone();
            let seconds = if phase == "optimization" { cfg.opt_seconds } else { cfg.synth_seconds };
            let seed = cfg.seed.wrapping_add((chain as u64 + 1) * 0x9E37_79B9)
                ^ ((phase.len() as u64) << 32);
            let phase_static: &'static str = if phase == "optimization" { "optimization" } else { "synthesis" };
            let spec_src2 = spec_src.clone();
            let mut start_slots = parse_program(&spec_src2)
                .map_err(|e| e.to_string())?
                .0;
            start_slots.resize(cfg.ell, isuperoptimizer::instr::Slot::Unused);
            let start = Program(start_slots);
            handles.push(std::thread::spawn(move || {
                let spec = Spec {
                    program: parse_program(&spec_src2).map_err(|e| e.to_string())?,
                    live_outs,
                };
                run_chain(&spec, &cfg2, phase_static, chain, seed, seconds, start)
            }));
        }
    }
    let mut outcomes = join_all(&b, log, handles)?;
    let tc_index = log_chain_outcomes(log, &b, &outcomes, 0);

    // Round 2 (the paper's pipeline): each synthesis result seeds a fresh
    // optimization run — crossing algorithm regions is synthesis's job;
    // stripping the noise inside the new region is then all downhill.
    let mut seen_prog = std::collections::HashSet::new();
    let synth_programs: Vec<Program> = outcomes
        .iter()
        .filter(|o| o.phase == "synthesis")
        .filter_map(|o| o.best.clone())
        .filter(|p| seen_prog.insert(p.to_string()))
        .take(2)
        .collect();
    for (si, p) in synth_programs.iter().enumerate() {
        let mut handles2 = Vec::new();
        for chain in 0..2 {
            let cfg2 = cfg.clone();
            let seed = cfg.seed ^ 0x5EED ^ ((si as u64 + 1) << 16) ^ (chain as u64);
            let p2 = p.clone();
            let live_outs = b.live_outs.to_vec();
            let seconds = cfg.opt_seconds;
            let spec_src2 = spec_src.clone();
            handles2.push(std::thread::spawn(move || {
                let spec = Spec {
                    program: parse_program(&spec_src2).map_err(|e| e.to_string())?,
                    live_outs,
                };
                run_chain(&spec, &cfg2, "optimization_from_synthesis", chain, seed, seconds, p2)
            }));
        }
        let o2 = join_all(&b, log, handles2)?;
        let _ = log_chain_outcomes(log, &b, &o2, tc_index);
        outcomes.extend(o2);
    }

    // Headline results. Final claims are re-verified with a long-timeout
    // session before being logged — chain verdicts used a short per-query
    // budget, and unknowns there were skipped, never trusted.
    let spec = Spec {
        program: parse_program(b.spec).map_err(|e| e.to_string())?,
        live_outs: b.live_outs.to_vec(),
    };
    let spec_perf = program_latency(&spec.program);
    let mut final_z3 = Z3Session::new()?;
    let mut improved: Option<&ChainOutcome> = None;
    let mut synthesis_found: Option<&ChainOutcome> = None;
    for o in &outcomes {
        if o.phase != "synthesis" {
            if let Some(p) = &o.best {
                if program_latency(p) < spec_perf
                    && improved.map(|i| program_latency(p) < program_latency(i.best.as_ref().unwrap())).unwrap_or(true)
                {
                    improved = Some(o);
                }
            }
        } else if o.best.is_some() && synthesis_found.is_none() {
            synthesis_found = Some(o);
        }
    }

    let mut status = "no_improvement";
    if let Some(o) = improved {
        let p = o.best.as_ref().unwrap();
        let verdict = match verify(p, &spec, &mut final_z3) {
            Ok(Verdict::Equivalent) => "equivalent (z3, final long-timeout re-verification; solver claim)",
            Ok(Verdict::Counterexample(_)) => {
                eprintln!("FAIL {}: final re-verification produced a counterexample — NOT claiming", b.name);
                "counterexample on final re-verification — NOT claimed"
            }
            Err(_) => "unknown on final re-verification — NOT claimed",
        };
        let claimed = verdict.starts_with("equivalent");
        log.event(&[
            ("kind", JVal::S("search_result".into())),
            ("benchmark", JVal::S(b.name.into())),
            ("phase", JVal::S("optimization".into())),
            ("program", JVal::S(p.to_string())),
            ("length", JVal::U(p.0.iter().filter(|s| matches!(s, isuperoptimizer::instr::Slot::Instr(_))).count() as u64)),
            ("spec_length", JVal::U(spec.program.0.len() as u64)),
            ("perf", JVal::U(program_latency(p))),
            ("spec_perf", JVal::U(spec_perf)),
            ("verdict", JVal::S(verdict.into())),
        ]);
        if claimed {
            status = "improved";
            println!(
                "  PASS {}: spec {} instr (perf {}) -> {} instr (perf {}) via optimization:\n    {}",
                b.name,
                spec.program.0.len(),
                spec_perf,
                p.0.iter().filter(|s| matches!(s, isuperoptimizer::instr::Slot::Instr(_))).count(),
                program_latency(p),
                p.to_string().replace('\n', "\n    ")
            );
        } else {
            return Err("final verification failed".into());
        }
    }
    if let Some(o) = synthesis_found {
        let p = o.best.as_ref().unwrap();
        let verdict = match verify(p, &spec, &mut final_z3) {
            Ok(Verdict::Equivalent) => "equivalent (z3, final long-timeout re-verification; solver claim)",
            Ok(Verdict::Counterexample(_)) => "counterexample on final re-verification — NOT claimed",
            Err(_) => "unknown on final re-verification — NOT claimed",
        };
        let claimed = verdict.starts_with("equivalent");
        log.event(&[
            ("kind", JVal::S("search_result".into())),
            ("benchmark", JVal::S(b.name.into())),
            ("phase", JVal::S("synthesis".into())),
            ("program", JVal::S(p.to_string())),
            ("length", JVal::U(p.0.iter().filter(|s| matches!(s, isuperoptimizer::instr::Slot::Instr(_))).count() as u64)),
            ("verdict", JVal::S(verdict.into())),
        ]);
        if claimed {
            println!(
                "  PASS {}: synthesis found a verified-equivalent program from a random start (chain {}):\n    {}",
                b.name, o.chain, p.to_string().replace('\n', "\n    ")
            );
            if status == "no_improvement" {
                status = "synthesis_only";
            }
        } else {
            eprintln!("FAIL {}: synthesis result failed final re-verification", b.name);
            return Err("final verification failed".into());
        }
    }
    if status == "no_improvement" {
        println!("  NOTE {}: no verified improvement found this run (chains ran to budget; stats above)", b.name);
    }

    log.event(&[
        ("kind", JVal::S("mcmc_summary".into())),
        ("benchmark", JVal::S(b.name.into())),
        ("status", JVal::S(status.into())),
        ("spec_perf", JVal::U(spec_perf)),
        ("best_perf", JVal::U(improved.map(|o| o.best_perf).unwrap_or(spec_perf))),
        ("chains", JVal::U(outcomes.len() as u64)),
        ("testcases_logged", JVal::U(tc_index)),
    ]);
    Ok(())
}

/// The straggler second attempt (ADR-011).
///
/// Chains start from STRUCTURAL VARIATIONS of the compiler's own form
/// (identity, single-instruction deletions, adjacent-slot swaps) instead of
/// pure random restarts, under a longer budget. Rationale, logged in the
/// `straggler_plan` event: synthesis spends most of its budget crossing
/// algorithmic regions; a perturbation of gcc's form starts inside or
/// adjacent to the right region, so the same budget explores deeper. The
/// variant programs are NOT equivalent to the spec (deletions change
/// behavior) — every equivalence claim still goes through the same
/// long-timeout z3 re-verification as any other search_result.
fn run_seeded_pass(
    bench: String,
    spec_src: String,
    source: &str,
    seconds: u64,
    n_variants: usize,
    seed: u64,
    log: &mut ReportLog,
) -> Result<(), String> {
    use isuperoptimizer::instr::Slot;
    use isuperoptimizer::mcmc::{program_latency, run_chain, ChainOutcome, McmcConfig};

    println!(
        "straggler second attempt: {} (seeded synthesis from gcc's form, {}s per chain, {} chains)",
        bench, seconds, n_variants
    );
    log.event(&[
        ("kind", JVal::S("straggler_plan".into())),
        ("benchmark", JVal::S(bench.clone())),
        ("source", JVal::S(source.into())),
        ("approach", JVal::S(
            "seeded synthesis: chains start from structural variations of the compiler's own form \
             (identity, single-instruction deletions, adjacent-slot swaps) instead of pure random \
             restarts, under a longer budget".into(),
        )),
        ("seconds_per_chain", JVal::U(seconds)),
        ("variant_chains", JVal::U(n_variants as u64)),
        ("rationale", JVal::S(
            "Honest no-improvement at the 120s/60s budget; the second attempt either finds \
             a verified improvement from informed starts or strengthens the no-improvement claim \
             at 5x the budget. Synthesis spends most of its budget crossing algorithmic regions; \
             perturbations of gcc's form start inside or adjacent to the right region".into(),
        )),
    ]);

    let live_outs = vec![isuperoptimizer::compiler_core::RET_REG];
    let spec = Spec {
        program: parse_program(&spec_src).map_err(|e| e.to_string())?,
        live_outs: live_outs.clone(),
    };
    let spec_perf = program_latency(&spec.program);
    let cfg = McmcConfig::paper(seed, seconds, seconds);

    // Deterministic variant construction from the compiler's own form.
    let base = spec.program.clone();
    let mut variants: Vec<Program> = Vec::new();
    variants.push(base.clone()); // identity: the compiler's own form
    for i in 0..base.0.len() {
        let mut v = base.0.clone();
        v.remove(i); // single-instruction deletion
        if !v.is_empty() {
            variants.push(Program(v));
        }
    }
    for i in 0..base.0.len().saturating_sub(1) {
        let mut v = base.0.clone();
        v.swap(i, i + 1); // adjacent-slot swap
        variants.push(Program(v));
    }
    variants.truncate(n_variants.max(1));

    for (i, v) in variants.iter().enumerate() {
        log.event(&[
            ("kind", JVal::S("seeded_start".into())),
            ("benchmark", JVal::S(bench.clone())),
            ("chain", JVal::U(i as u64)),
            ("program", JVal::S(v.to_string())),
            ("derivation", JVal::S(if i == 0 {
                "identity: the compiler's own form".to_string()
            } else {
                "single structural mutation of the compiler's form (deletion or adjacent swap)".to_string()
            })),
        ]);
    }

    let mut handles = Vec::new();
    for (chain, v) in variants.iter().enumerate() {
        let spec_src2 = spec_src.clone();
        let cfg2 = cfg.clone();
        let cseed = seed
            .wrapping_add((chain as u64 + 1) * 0x9E37_79B9)
            ^ 0xA11_CE5EED;
        let live = live_outs.clone();
        let v2 = v.clone();
        let seconds2 = seconds;
        handles.push(std::thread::spawn(move || {
            let s = Spec {
                program: parse_program(&spec_src2).map_err(|e| e.to_string())?,
                live_outs: live,
            };
            run_chain(&s, &cfg2, "seeded_optimization", chain, cseed, seconds2, v2)
        }));
    }
    let outcomes: Vec<ChainOutcome> = join_all_plain(&bench, log, handles)?;

    // Register the bench (same spec) and log the chains' testcases +
    // stats, exactly like the main MCMC pass.
    let db = isuperoptimizer::benchmarks::DynBenchmark {
        name: bench.clone(),
        desc: format!("{} — straggler second attempt (seeded synthesis)", bench),
        spec: spec_src.clone(),
        live_outs: live_outs.clone(),
        strategy: Strategy::Mcmc,
    };
    log.event(&[
        ("kind", JVal::S("bench_start".into())),
        ("benchmark", JVal::S(bench.clone())),
        ("spec", JVal::S(spec_src.clone())),
        ("desc", JVal::S(format!("{} — straggler second attempt (seeded synthesis)", bench))),
        ("live_outs", JVal::ArrS(live_outs.iter().map(|g| g.name().to_string()).collect())),
        ("strategy", JVal::S("mcmc_seeded".into())),
    ]);
    log_chain_outcomes(log, &db.view(), &outcomes, 0);

    // Final claims — same long-timeout re-verification discipline as the
    // main pass; only z3-equivalent, latency-improving results are claimed.
    let mut final_z3 = Z3Session::new()?;
    let mut improved: Option<&ChainOutcome> = None;
    for o in &outcomes {
        if let Some(p) = &o.best {
            if program_latency(p) < spec_perf
                && improved
                    .map(|i| program_latency(p) < program_latency(i.best.as_ref().unwrap()))
                    .unwrap_or(true)
            {
                improved = Some(o);
            }
        }
    }
    let mut status = "no_improvement";
    if let Some(o) = improved {
        let p = o.best.as_ref().unwrap();
        let verdict = match verify(p, &spec, &mut final_z3) {
            Ok(Verdict::Equivalent) => "equivalent (z3, final long-timeout re-verification; solver claim)",
            Ok(Verdict::Counterexample(_)) => {
                eprintln!("FAIL straggler {}: final re-verification produced a counterexample — NOT claiming", bench);
                "counterexample on final re-verification — NOT claimed"
            }
            Err(_) => "unknown on final re-verification — NOT claimed",
        };
        let claimed = verdict.starts_with("equivalent");
        log.event(&[
            ("kind", JVal::S("search_result".into())),
            ("benchmark", JVal::S(bench.clone())),
            ("phase", JVal::S("seeded_optimization".into())),
            ("program", JVal::S(p.to_string())),
            ("length", JVal::U(p.0.iter().filter(|s| matches!(s, Slot::Instr(_))).count() as u64)),
            ("spec_length", JVal::U(spec.program.0.len() as u64)),
            ("perf", JVal::U(program_latency(p))),
            ("spec_perf", JVal::U(spec_perf)),
            ("verdict", JVal::S(verdict.into())),
        ]);
        if claimed {
            status = "improved";
            println!(
                "  PASS straggler {}: gcc form (perf {}) -> {} instr (perf {}) via seeded synthesis, chain {}:\n    {}",
                bench,
                spec_perf,
                p.0.iter().filter(|s| matches!(s, Slot::Instr(_))).count(),
                program_latency(p),
                o.chain,
                p.to_string().replace('\n', "\n    ")
            );
        } else {
            // A refuted candidate is a LOGGED OUTCOME, not a pipeline
            // failure: the second attempt ran, found a latency-improving
            // candidate, and the final verification refused it. The summary
            // must still be written (found live when the first straggler
            // run errored out here and left the O2 attempts summaryless).
            status = "candidate_refuted_on_final_verification";
            println!(
                "  NOTE straggler {}: latency-improving candidate REFUTED by final verification — not claimed",
                bench
            );
        }
    } else {
        println!("  NOTE straggler {}: no verified improvement from seeded starts either (chains ran to budget)", bench);
    }

    log.event(&[
        ("kind", JVal::S("straggler_summary".into())),
        ("benchmark", JVal::S(bench.clone())),
        ("status", JVal::S(status.into())),
        ("spec_perf", JVal::U(spec_perf)),
        ("best_perf", JVal::U(improved.map(|o| o.best_perf).unwrap_or(spec_perf))),
        ("chains", JVal::U(outcomes.len() as u64)),
        ("seconds_per_chain", JVal::U(seconds)),
    ]);
    Ok(())
}

/// join_all for the seeded pass (the shared join_all logs against an
/// AnyBenchmark; the bench name suffices for error reporting here).
fn join_all_plain(
    bench: &str,
    log: &mut ReportLog,
    handles: Vec<std::thread::JoinHandle<Result<isuperoptimizer::mcmc::ChainOutcome, String>>>,
) -> Result<Vec<isuperoptimizer::mcmc::ChainOutcome>, String> {
    let mut outcomes = Vec::new();
    for h in handles {
        match h.join() {
            Ok(Ok(o)) => outcomes.push(o),
            Ok(Err(e)) => {
                log.event(&[
                    ("kind", JVal::S("mcmc_summary".into())),
                    ("benchmark", JVal::S(bench.into())),
                    ("status", JVal::S(format!("chain error: {}", e))),
                ]);
                eprintln!("FAIL {}: chain error: {}", bench, e);
                return Err(e);
            }
            Err(_) => {
                eprintln!("FAIL {}: chain panicked", bench);
                return Err("chain panicked".into());
            }
        }
    }
    Ok(outcomes)
}

fn flag(args: &[String], name: &str) -> Option<String> {
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == name {
            return it.next().cloned();
        }
    }
    None
}

fn usage() {
    eprintln!("usage: isuperopt <command> [options]");
    eprintln!("  enumerate [--bench <name|enum-all>] [--out <file>] [--seed N]");
    eprintln!();
    eprintln!("benchmarks:");
    for b in benchmarks::BENCHMARKS {
        eprintln!("  {:<12} {:<44} {}", b.name, b.desc, b.spec.replace('\n', " ; "));
    }
}

// keep eval_program/parse_program linked into this binary (used by future
// subcommands and by the sanity check below)
#[allow(dead_code)]
fn _unused(_p: &isuperoptimizer::instr::Program, _s: &Spec) -> [u64; 16] {
    eval_program(&parse_program("movq %rax, %rcx").unwrap(), [0u64; 16]).regs
}
