//! Stress harness: generates the acceptance evidence with real numbers.
//!
//! Modes (all on by default; sizes via flags):
//!   --bulk N     differential: concrete vs folded-symbolic, N programs x M states
//!   --states M   states per program in bulk mode (default 2)
//!   --z3 K       sampled z3 round-trip differential (printer + protocol + solver)
//!   --grid       verify(p,p) over the complete single-instruction grid
//!   --self E     verify(p,p) over E random multi-instruction programs
//!   --pairs      hand-verified equivalences + deliberately broken pairs
//!   --seed S     RNG seed (default 0xC0FFEE)
//!   --quick      small sizes for smoke runs
//!
//! Exit code 0 iff every enabled mode passes with zero discrepancies.

use std::time::Instant;

use isuperoptimizer::fuzz::{
    all_single_instr_programs, random_mem_state, random_program_mem, written_regs,
};
use isuperoptimizer::instr::{
    parse_program, Cc, Gpr, Instr, MemAddr, Operand, Program, ShiftCount, Slot,
};
use isuperoptimizer::reportlog::{JVal, ReportLog};
use isuperoptimizer::rng::Rng;
use isuperoptimizer::state::{eval_instr, eval_program, eval_program_in, CState};
use isuperoptimizer::sym::{sym_eval_program, SState};
use isuperoptimizer::verify::{verify, Spec, Verdict};
use isuperoptimizer::z3proc::{Answer, Z3Session};

use isuperoptimizer::ast::BV;

fn hex16(v: u64) -> String {
    format!("0x{:016x}", v)
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

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let quick = args.iter().any(|a| a == "--quick");
    let seed: u64 = flag(&args, "--seed").and_then(|s| s.parse().ok()).unwrap_or(0xC0FFEE);

    let (bulk, states, z3n, selfcheck) = if quick {
        (20_000, 2, 200, 200)
    } else {
        (
            flag(&args, "--bulk").and_then(|s| s.parse().ok()).unwrap_or(1_000_000),
            flag(&args, "--states").and_then(|s| s.parse().ok()).unwrap_or(2),
            flag(&args, "--z3").and_then(|s| s.parse().ok()).unwrap_or(10_000),
            flag(&args, "--self").and_then(|s| s.parse().ok()).unwrap_or(2_000),
        )
    };
    let want_grid = !args.iter().any(|a| a == "--no-grid");
    let want_pairs = !args.iter().any(|a| a == "--no-pairs");
    let hw: u64 = flag(&args, "--hw")
        .and_then(|s| s.parse().ok())
        .unwrap_or(if quick { 50 } else { 2000 });
    let log_path = flag(&args, "--log")
        .unwrap_or_else(|| "artifacts/stress-log.jsonl".to_string());

    // The artifact of record: every report-quotable fact lands in this file.
    // Reports paste from it; `recheck --from-log` re-verifies it.
    let mut log = ReportLog::open(&log_path);

    let mut failures = 0u32;
    failures += run_bulk(bulk, states, seed, &mut log);
    failures += run_z3_sample(z3n, seed ^ 0x5EED, &mut log);
    if want_grid {
        failures += run_grid_sweep(&mut log);
    }
    failures += run_selfcheck(selfcheck, seed ^ 0xABCD, &mut log);
    if want_pairs {
        failures += run_pairs(&mut log);
    }
    failures += run_hw(hw, seed ^ 0x11A0, &mut log);
    failures += run_hw_mem(hw, seed ^ 0x0DEF, &mut log);
    let oracle: u64 = flag(&args, "--compiler-oracle")
        .and_then(|s| s.parse().ok())
        .unwrap_or(2000);
    let oracle_batch = flag(&args, "--compiler-oracle-batch")
        .unwrap_or_else(|| "core".to_string());
    failures += run_compiler_oracle(oracle, seed ^ 0xAB1, &oracle_batch, &mut log);

    log.event(&[
        ("kind", JVal::S("run_summary".into())),
        ("status", JVal::S(if failures == 0 { "all_passed".into() } else { "failures".into() })),
        ("modes_failed", JVal::U(failures as u64)),
    ]);
    log.flush();
    if failures == 0 {
        println!("\nALL STRESS MODES PASSED");
        println!("artifact of record: {}", log.path);
        std::process::exit(0);
    } else {
        println!("\n{} STRESS MODE(S) FAILED", failures);
        println!("artifact of record: {}", log.path);
        std::process::exit(1);
    }
}

// ---------------------------------------------------------------- bulk

fn run_bulk(n: u64, m: u64, seed: u64, log: &mut ReportLog) -> u32 {
    if n == 0 {
        return 0;
    }
    let t0 = Instant::now();
    let mut rng = Rng::new(seed);
    let mut discrepancies = 0u64;
    let mut shown = 0u64;
    let mut program_states = 0u64;

    for _ in 0..n {
        // Memory-inclusive generator: the whole v0 subset.
        let p = random_program_mem(&mut rng, 8);
        let sym = sym_eval_program(&p, SState::from_vars());
        let has_stores = p.0.iter().any(|s| matches!(s, Slot::Instr(Instr::Store { .. })));
        for _ in 0..m {
            program_states += 1;
            let (st, mem, mem_base) = random_mem_state(&mut rng);
            let c = eval_program_in(&p, st, mem, mem_base);
            let env = isuperoptimizer::ast::Env { regs: st, mem: Some(&mem), mem_base };

            let mut bad: Vec<String> = vec![];
            for r in 0..16 {
                let symv = sym.regs[r].eval_in(&env);
                if symv != c.regs[r] {
                    bad.push(format!("r{}: concrete={:#018x} symbolic={:#018x}", r, c.regs[r], symv));
                }
            }
            if has_stores {
                // Memory: fold 8-byte windows of the final symbolic memory
                // and compare with the concrete sandbox.
                for k in 0..(isuperoptimizer::state::MEM_BYTES / 8) {
                    let addr = BV::Const(mem_base + (k as u64) * 8);
                    let symw = BV::Load64 { mem: Box::new(sym.mem.clone()), addr: Box::new(addr) }
                        .eval_in(&env);
                    let offw = k * 8;
                    let cw = u64::from_le_bytes(c.mem[offw..offw + 8].try_into().unwrap());
                    if symw != cw {
                        bad.push(format!(
                            "mem[{:#x}]: concrete={:#018x} symbolic={:#018x}",
                            mem_base + (k as u64) * 8, cw, symw
                        ));
                        break; // one report per program-state is enough
                    }
                }
            }
            macro_rules! fb {
                ($field:ident, $label:expr) => {
                    let symv = sym.flags.$field.eval_in(&env);
                    let cv = c.flags.$field;
                    if symv != cv {
                        bad.push(format!("{}: concrete={} symbolic={}", $label, cv, symv));
                    }
                };
            }
            fb!(zf, "zf");
            fb!(sf, "sf");
            fb!(cf, "cf");
            fb!(of, "of");
            fb!(zf_def, "zf_def");
            fb!(sf_def, "sf_def");
            fb!(cf_def, "cf_def");
            fb!(of_def, "of_def");
            let sym_def = sym.def.eval_in(&env);
            if sym_def != c.def {
                bad.push(format!("def: concrete={} symbolic={}", c.def, sym_def));
            }

            if !bad.is_empty() {
                discrepancies += 1;
                if shown < 3 {
                    shown += 1;
                    eprintln!("DISCREPANCY #{}:\nprogram:\n{}\ninputs: {:?}\n{}", discrepancies, p, st, bad.join("\n"));
                }
            }
        }
    }

    let dt = t0.elapsed().as_secs_f64();
    let rate = (program_states as f64 / dt).max(0.0) as u64;
    let millis = (dt * 1000.0) as u64;
    log.event(&[
        ("kind", JVal::S("mode_summary".into())),
        ("mode", JVal::S("bulk".into())),
        ("status", JVal::S(if discrepancies == 0 { "pass".into() } else { "fail".into() })),
        ("programs", JVal::U(n)),
        ("states_per_program", JVal::U(m)),
        ("program_evals", JVal::U(program_states)),
        ("discrepancies", JVal::U(discrepancies)),
        ("millis", JVal::U(millis)),
        ("rate_per_s", JVal::U(rate)),
    ]);
    if discrepancies == 0 {
        println!(
            "PASS bulk   : {} programs x {} states = {} program-evals, 0 discrepancies ({:.1}s, {} evals/s)",
            n, m, program_states, dt, rate
        );
        0
    } else {
        println!(
            "FAIL bulk   : {} programs x {} states = {} program-evals, {} DISCREPANCIES ({:.1}s)",
            n, m, program_states, discrepancies, dt
        );
        1
    }
}

// ---------------------------------------------------------------- z3 sample

fn run_z3_sample(k: u64, seed: u64, log: &mut ReportLog) -> u32 {
    if k == 0 {
        return 0;
    }
    let t0 = Instant::now();
    let mut rng = Rng::new(seed);
    let mut z = match Z3Session::new() {
        Ok(z) => z,
        Err(e) => {
            eprintln!("FAIL z3sample: {}", e);
            return 1;
        }
    };
    let mut discrepancies = 0u64;
    let mut shown = 0u64;

    for _ in 0..k {
        let p = random_program_mem(&mut rng, 8);
        let (st, mem, mem_base) = random_mem_state(&mut rng);
        let c = eval_program_in(&p, st, mem, mem_base);
        let sym = sym_eval_program(&p, SState::from_vars());
        let uses_mem = p.0.iter().any(|s| {
            matches!(s, Slot::Instr(Instr::Load { .. }) | Slot::Instr(Instr::Store { .. }))
        });
        let has_stores = p.0.iter().any(|s| matches!(s, Slot::Instr(Instr::Store { .. })));

        let mut cmds = Vec::new();
        // All input declarations and pinning first — o-exprs may reference any rN.
        for r in 0..16 {
            cmds.push(format!("(declare-const r{} (_ BitVec 64))", r));
            cmds.push(format!("(assert (= r{} #x{:016x}))", r, st[r]));
        }
        if uses_mem {
            cmds.push("(declare-const mem (Array (_ BitVec 64) (_ BitVec 8)))".to_string());
            for (kb, b) in mem.iter().enumerate() {
                cmds.push(format!(
                    "(assert (= (select mem #x{:016x}) #x{:02x}))",
                    mem_base + kb as u64,
                    b
                ));
            }
        }
        for r in 0..16 {
            cmds.push(format!("(declare-const o{} (_ BitVec 64))", r));
            cmds.push(format!("(assert (= o{} {}))", r, sym.regs[r].to_smt2()));
        }
        let mut want: Vec<String> = (0..16).map(|r| format!("o{}", r)).collect();
        if has_stores {
            // Compare the final memory at every 8-byte sandbox window.
            for w in 0..(isuperoptimizer::state::MEM_BYTES / 8) {
                let name = format!("om{}", w);
                cmds.push(format!("(declare-const {} (_ BitVec 64))", name));
                cmds.push(format!(
                    "(assert (= {} {}))",
                    name,
                    BV::Load64 {
                        mem: Box::new(sym.mem.clone()),
                        addr: Box::new(BV::Const(mem_base + (w as u64) * 8)),
                    }
                    .to_smt2()
                ));
                want.push(name);
            }
        }
        for (name, expr) in [
            ("fz", &sym.flags.zf),
            ("ff", &sym.flags.sf),
            ("fc", &sym.flags.cf),
            ("fo", &sym.flags.of),
            ("fzd", &sym.flags.zf_def),
            ("fsd", &sym.flags.sf_def),
            ("fcd", &sym.flags.cf_def),
            ("fod", &sym.flags.of_def),
            ("fd", &sym.def),
        ] {
            cmds.push(format!("(declare-const {} Bool)", name));
            cmds.push(format!("(assert (= {} {}))", name, expr.to_smt2()));
        }

        for name in ["fz", "ff", "fc", "fo", "fzd", "fsd", "fcd", "fod", "fd"] {
            want.push(name.to_string());
        }

        let mut bad: Vec<String> = vec![];
        match z.solve(&cmds, Some(&want)) {
            Ok(Answer::Sat(vals)) => {
                let mut by_name = std::collections::HashMap::new();
                for (n, v) in &vals {
                    by_name.insert(n.clone(), *v);
                }
                for r in 0..16 {
                    let got = match by_name.get(&format!("o{}", r)) {
                        Some(v) => *v,
                        None => {
                            eprintln!("MISSING o{}; response pairs: {:?}", r, vals);
                            panic!("missing key o{}", r);
                        }
                    };
                    if got != c.regs[r] {
                        bad.push(format!("o{}: z3={:#018x} concrete={:#018x}", r, got, c.regs[r]));
                    }
                }
                if has_stores {
                    for w in 0..(isuperoptimizer::state::MEM_BYTES / 8) {
                        let got = match by_name.get(&format!("om{}", w)) {
                            Some(v) => *v,
                            None => {
                                eprintln!("MISSING om{}; response pairs: {:?}", w, vals);
                                panic!("missing key om{}", w);
                            }
                        };
                        let offw = w * 8;
                        let cw = u64::from_le_bytes(c.mem[offw..offw + 8].try_into().unwrap());
                        if got != cw {
                            bad.push(format!("om{}: z3={:#018x} concrete={:#018x}", w, got, cw));
                            break;
                        }
                    }
                }
                macro_rules! chk {
                    ($name:expr, $cval:expr) => {
                        let got = by_name[$name];
                        if got != ($cval as u64) {
                            bad.push(format!("{}: z3={} concrete={}", $name, got, $cval));
                        }
                    };
                }
                chk!("fz", c.flags.zf);
                chk!("ff", c.flags.sf);
                chk!("fc", c.flags.cf);
                chk!("fo", c.flags.of);
                chk!("fzd", c.flags.zf_def);
                chk!("fsd", c.flags.sf_def);
                chk!("fcd", c.flags.cf_def);
                chk!("fod", c.flags.of_def);
                chk!("fd", c.def);
            }
            Ok(other) => bad.push(format!("expected sat, got {}", if matches!(other, Answer::Unsat) { "unsat" } else { "unknown" })),
            Err(e) => bad.push(format!("z3 error: {}", e)),
        }

        if !bad.is_empty() {
            discrepancies += 1;
            if shown < 3 {
                shown += 1;
                eprintln!("Z3-SAMPLE DISCREPANCY #{}:\nprogram:\n{}\ninputs: {:?}\n{}", discrepancies, p, st, bad.join("\n"));
            }
        }
    }

    let dt = t0.elapsed().as_secs_f64();
    let rate = (k as f64 / dt).max(0.0) as u64;
    let millis = (dt * 1000.0) as u64;
    log.event(&[
        ("kind", JVal::S("mode_summary".into())),
        ("mode", JVal::S("z3".into())),
        ("status", JVal::S(if discrepancies == 0 { "pass".into() } else { "fail".into() })),
        ("queries", JVal::U(k)),
        ("discrepancies", JVal::U(discrepancies)),
        ("millis", JVal::U(millis)),
        ("rate_per_s", JVal::U(rate)),
    ]);
    if discrepancies == 0 {
        println!(
            "PASS z3     : {} full SMT-LIB2 round-trips through z3 (inputs asserted, outputs evaluated by solver), 0 discrepancies ({:.1}s, {} queries/s)",
            k, dt, rate
        );
        0
    } else {
        println!("FAIL z3     : {} queries, {} DISCREPANCIES ({:.1}s)", k, discrepancies, dt);
        1
    }
}

// ---------------------------------------------------------------- self-checks

fn spec_all_writes(p: &Program) -> Spec {
    Spec { program: p.clone(), live_outs: written_regs(p) }
}

fn run_grid_sweep(log: &mut ReportLog) -> u32 {
    let t0 = Instant::now();
    let progs = all_single_instr_programs();
    let mut z = match Z3Session::new() {
        Ok(z) => z,
        Err(e) => {
            eprintln!("FAIL grid: {}", e);
            return 1;
        }
    };
    let mut failures = 0u64;
    let mut first_err = String::new();
    for p in &progs {
        let spec = spec_all_writes(p);
        match verify(p, &spec, &mut z) {
            Ok(Verdict::Equivalent) => {}
            Ok(Verdict::Counterexample(_)) => {
                failures += 1;
                if first_err.is_empty() {
                    first_err = format!("self-verification produced a counterexample for {}", p);
                }
            }
            Err(e) => {
                failures += 1;
                if first_err.is_empty() {
                    first_err = format!("self-verification errored for {}: {}", p, e);
                }
            }
        }
    }
    let dt = t0.elapsed().as_secs_f64();
    let rate = (progs.len() as f64 / dt).max(0.0) as u64;
    let millis = (dt * 1000.0) as u64;
    log.event(&[
        ("kind", JVal::S("mode_summary".into())),
        ("mode", JVal::S("grid".into())),
        ("status", JVal::S(if failures == 0 { "pass".into() } else { "fail".into() })),
        ("programs", JVal::U(progs.len() as u64)),
        ("failures", JVal::U(failures)),
        ("millis", JVal::U(millis)),
        ("rate_per_s", JVal::U(rate)),
    ]);
    if failures == 0 {
        println!(
            "PASS grid   : verify(p,p) UNSAT for all {} single-instruction programs in the operand grid ({:.1}s, {} queries/s)",
            progs.len(), dt, rate
        );
        0
    } else {
        println!("FAIL grid   : {}/{} self-verifications failed; first: {}", failures, progs.len(), first_err);
        1
    }
}

fn run_selfcheck(e: u64, seed: u64, log: &mut ReportLog) -> u32 {
    if e == 0 {
        return 0;
    }
    let t0 = Instant::now();
    let mut rng = Rng::new(seed);
    let mut z = match Z3Session::new() {
        Ok(z) => z,
        Err(x) => {
            eprintln!("FAIL selfcheck: {}", x);
            return 1;
        }
    };
    let mut failures = 0u64;
    let mut first_err = String::new();
    for _ in 0..e {
        let p = random_program_mem(&mut rng, 6);
        let spec = spec_all_writes(&p);
        match verify(&p, &spec, &mut z) {
            Ok(Verdict::Equivalent) => {}
            Ok(Verdict::Counterexample(_)) => {
                failures += 1;
                if first_err.is_empty() {
                    first_err = format!("self-verification counterexample for {}", p);
                }
            }
            Err(x) => {
                failures += 1;
                if first_err.is_empty() {
                    first_err = format!("self-verification error for {}: {}", p, x);
                }
            }
        }
    }
    let dt = t0.elapsed().as_secs_f64();
    let millis = (dt * 1000.0) as u64;
    log.event(&[
        ("kind", JVal::S("mode_summary".into())),
        ("mode", JVal::S("self".into())),
        ("status", JVal::S(if failures == 0 { "pass".into() } else { "fail".into() })),
        ("programs", JVal::U(e)),
        ("failures", JVal::U(failures)),
        ("millis", JVal::U(millis)),
    ]);
    if failures == 0 {
        println!(
            "PASS self   : verify(p,p) UNSAT for {} random multi-instruction programs ({:.1}s)",
            e, dt
        );
        0
    } else {
        println!("FAIL self   : {}/{} failed; first: {}", failures, e, first_err);
        1
    }
}

// ---------------------------------------------------------------- hardware oracle

/// Differential-test the concrete evaluator against real x86-64 silicon via
/// `tools/hw_oracle`. Undefined flags are never compared: IMUL's ZF/SF, and
/// shift OF for effective counts other than 0/1 (count 0 compares only the
/// result, since "flags unaffected" is relative to unknowable prior flags).
fn run_hw(n: u64, seed: u64, log: &mut ReportLog) -> u32 {
    if n == 0 {
        return 0;
    }
    let t0 = std::time::Instant::now();
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Command, Stdio};

    let child = Command::new("./tools/hw_oracle")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            eprintln!("FAIL hw: could not spawn ./tools/hw_oracle (build it: cc -O2 -o tools/hw_oracle tools/hw_oracle.c): {}", e);
            return 1;
        }
    };
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());

    // Consume the MEMBASE handshake line (the oracle prints it first; this
    // loop does not use memory, but the protocol must stay in sync).
    let mut first = String::new();
    if stdout.read_line(&mut first).unwrap_or(0) == 0 || !first.starts_with("MEMBASE") {
        eprintln!("FAIL hw: oracle did not send MEMBASE handshake");
        return 1;
    }

    let mut rng = Rng::new(seed);
    let mut discrepancies = 0u64;
    let mut compared_flags = 0u64;
    let mut cases = 0u64;

    let binops = [
        ("add", Instr::Add { src: Operand::Reg(Gpr::Rbx), dst: Gpr::Rax }),
        ("sub", Instr::Sub { src: Operand::Reg(Gpr::Rbx), dst: Gpr::Rax }),
        ("cmp", Instr::Cmp { src: Operand::Reg(Gpr::Rbx), dst: Gpr::Rax }),
        ("and", Instr::And { src: Operand::Reg(Gpr::Rbx), dst: Gpr::Rax }),
        ("or", Instr::Or { src: Operand::Reg(Gpr::Rbx), dst: Gpr::Rax }),
        ("xor", Instr::Xor { src: Operand::Reg(Gpr::Rbx), dst: Gpr::Rax }),
        ("imul", Instr::Imul { src: Gpr::Rbx, dst: Gpr::Rax }),
    ];
    let shifts = [
        ("shl", 0u8, Instr::Shl { count: ShiftCount::Cl, dst: Gpr::Rax }),
        ("shr", 0, Instr::Shr { count: ShiftCount::Cl, dst: Gpr::Rax }),
        ("sar", 0, Instr::Sar { count: ShiftCount::Cl, dst: Gpr::Rax }),
        ("neg", 0, Instr::Neg { dst: Gpr::Rax }),
    ];    let ccs: Vec<(&str, Cc)> = [
        Cc::E, Cc::Ne, Cc::A, Cc::Ae, Cc::B, Cc::Be, Cc::G, Cc::Ge, Cc::L, Cc::Le,
    ]
    .iter()
    .map(|&c| (format!("set{}", c.suffix()).leak() as &str, c))
    .collect();
    let cmovs: Vec<(&str, Cc)> = ccs
        .iter()
        .map(|&(_, c)| (format!("cmov{}", c.suffix()).leak() as &str, c))
        .collect();

    // build the full op case list: (hw op name, instr list, kind)
    #[derive(Clone, Copy)]
    enum Kind { Bin, Shift, Set, Cmov, Neg, NotPreset }
    let mut ops: Vec<(&str, Vec<Instr>, Kind)> = vec![];
    for (name, i) in binops {
        ops.push((name, vec![i], Kind::Bin));
    }
    for (name, _, i) in shifts {
        let kind = if name == "neg" { Kind::Neg } else { Kind::Shift };
        ops.push((name, vec![i], kind));
    }
    for (name, cc) in &ccs {
        ops.push((
            name,
            vec![
                Instr::Cmp { src: Operand::Reg(Gpr::Rbx), dst: Gpr::Rax },
                Instr::Set { cc: *cc, dst: Gpr::Rax },
            ],
            Kind::Set,
        ));
    }
    for (name, cc) in &cmovs {
        ops.push((
            name,
            vec![
                Instr::Cmp { src: Operand::Reg(Gpr::Rbx), dst: Gpr::Rax },
                Instr::Cmov { cc: *cc, src: Gpr::Rbx, dst: Gpr::Rax },
            ],
            Kind::Cmov,
        ));
    }
    // `not`: flags must carry through UNAFFECTED (SDM), so the
    // hardware handler pre-sets flags with a subq on the same operand and
    // the model evaluates that sub first; all four flags are compared after
    // the notq. This validates the flags-untouched claim on silicon, not
    // just the register result.
    ops.push((
        "not",
        vec![
            Instr::Sub { src: Operand::Reg(Gpr::Rbx), dst: Gpr::Rax },
            Instr::Not { dst: Gpr::Rax },
        ],
        Kind::NotPreset,
    ));

    let interesting_counts = [0u64, 1, 2, 3, 5, 31, 32, 63, 64, 65, 100, 127, 128, 200, 255, 256];

    'outer: for _ in 0..n {
        for (name, instrs, kind) in &ops {
            let a = rng.interesting_u64();
            let b = rng.interesting_u64();
            let count = interesting_counts[rng.below(interesting_counts.len() as u64) as usize];

            let mut st = [0u64; 16];
            st[0] = a; // rax
            st[3] = b; // rbx
            st[1] = count; // rcx low byte = shift count
            let mut cstate = CState::from_regs(st);
            for i in instrs {
                cstate = eval_instr(i, &cstate);
            }
            cases += 1;

            let line = format!("{} {:x} {:x} {:x}\n", name, a, b, count);
            if stdin.write_all(line.as_bytes()).and_then(|_| stdin.flush()).is_err() {
                break 'outer;
            }
            let mut resp = String::new();
            if stdout.read_line(&mut resp).unwrap_or(0) == 0 {
                break 'outer;
            }
            let fields: Vec<&str> = resp.split_whitespace().collect();
            if fields.len() != 5 {
                discrepancies += 1;
                eprintln!("HW DISCREPANCY: bad oracle response `{}`", resp.trim());
                continue;
            }
            let hw_r = u64::from_str_radix(fields[0], 16).unwrap_or(u64::MAX);
            let (hw_zf, hw_sf, hw_cf, hw_of) = (
                fields[1] == "1",
                fields[2] == "1",
                fields[3] == "1",
                fields[4] == "1",
            );

            let mut bad: Vec<String> = vec![];
            let cf = &cstate.flags;
            if hw_r != cstate.regs[0] {
                bad.push(format!("r: hw={:#018x} concrete={:#018x}", hw_r, cstate.regs[0]));
            }
            // Which flags are defined in our model for this case?
            let (cmp_zf, cmp_sf, cmp_cf, cmp_of) = match kind {
                Kind::Bin if *name == "imul" => (false, false, true, true),
                Kind::Bin => (true, true, true, true),
                Kind::Shift => {
                    let c = count & 63;
                    if c == 0 {
                        (false, false, false, false)
                    } else if c == 1 {
                        (true, true, true, true)
                    } else {
                        (true, true, true, false)
                    }
                }
                Kind::Set | Kind::Cmov | Kind::Neg | Kind::NotPreset => (true, true, true, true),
            };
            if cmp_zf {
                compared_flags += 1;
                if hw_zf != cf.zf { bad.push(format!("zf: hw={} concrete={}", hw_zf, cf.zf)); }
            }
            if cmp_sf {
                compared_flags += 1;
                if hw_sf != cf.sf { bad.push(format!("sf: hw={} concrete={}", hw_sf, cf.sf)); }
            }
            if cmp_cf {
                compared_flags += 1;
                if hw_cf != cf.cf { bad.push(format!("cf: hw={} concrete={}", hw_cf, cf.cf)); }
            }
            if cmp_of {
                compared_flags += 1;
                if hw_of != cf.of { bad.push(format!("of: hw={} concrete={}", hw_of, cf.of)); }
            }

            if !bad.is_empty() {
                discrepancies += 1;
                if discrepancies <= 5 {
                    eprintln!(
                        "HW DISCREPANCY: op={} a={:#018x} b={:#018x} count={}\n{}",
                        name, a, b, count, bad.join("\n")
                    );
                }
            }
        }
    }
    drop(stdin);
    let _ = child.wait();

    let dt = t0.elapsed().as_secs_f64();
    let millis = (dt * 1000.0) as u64;
    log.event(&[
        ("kind", JVal::S("mode_summary".into())),
        ("mode", JVal::S("hw".into())),
        ("status", JVal::S(if discrepancies == 0 { "pass".into() } else { "fail".into() })),
        ("cases", JVal::U(cases)),
        ("ops", JVal::U(ops.len() as u64)),
        ("flag_comparisons", JVal::U(compared_flags)),
        ("discrepancies", JVal::U(discrepancies)),
        ("millis", JVal::U(millis)),
    ]);
    if discrepancies == 0 {
        println!(
            "PASS hw     : {} silicon cases across {} ops ({} flag comparisons), 0 discrepancies ({:.1}s)",
            cases, ops.len(), compared_flags, dt
        );
        0
    } else {
        println!("FAIL hw     : {} cases, {} DISCREPANCIES ({:.1}s)", cases, discrepancies, dt);
        1
    }
}

/// Base event fields for a pair verdict.
fn pair_base_fields(pair: &str, t: &str, c: &str, live: &[Gpr]) -> Vec<(&'static str, JVal)> {
    vec![
        ("kind", JVal::S("pair_verdict".into())),
        ("pair", JVal::S(pair.to_string())),
        ("target", JVal::S(t.to_string())),
        ("candidate", JVal::S(c.to_string())),
        (
            "live_outs",
            JVal::ArrS(live.iter().map(|g| g.name().to_string()).collect()),
        ),
    ]
}

/// Append full counterexample details (16 hex inputs, both programs' concrete
/// outputs on those inputs, and which live-outs actually differ) to an event.
/// The report pastes these fields verbatim; `recheck --from-log` re-verifies
/// every one of them with the concrete evaluator. When a pinned memory is
/// supplied (memory-involving pairs), outputs are computed under it and the
/// event carries the pin so any replay reproduces them exactly.
fn log_counterexample(
    fields: &mut Vec<(&'static str, JVal)>,
    spec: &Spec,
    cand: &Program,
    tc: &isuperoptimizer::cost::TestCase,
    mem: Option<(&[u8], u64)>,
) {
    let (target, rewrite) = match mem {
        Some((bytes, base)) => (
            eval_program_in(&spec.program, tc.inputs, to_mem(bytes), base),
            eval_program_in(cand, tc.inputs, to_mem(bytes), base),
        ),
        None => (eval_program(&spec.program, tc.inputs), eval_program(cand, tc.inputs)),
    };
    let mut differs_on = Vec::new();
    for g in &spec.live_outs {
        if target.regs[g.idx()] != rewrite.regs[g.idx()] {
            differs_on.push(g.name().to_string());
        }
    }
    if let Some((bytes, base)) = mem {
        fields.push(("mem_base", JVal::S(hex16(base))));
        fields.push((
            "mem_pin",
            JVal::ArrS(bytes.iter().map(|b| format!("0x{:02x}", b)).collect()),
        ));
    }
    fields.push(("inputs", JVal::ArrS(tc.inputs.iter().map(|v| hex16(*v)).collect())));
    fields.push(("target_out", JVal::ArrS(target.regs.iter().map(|v| hex16(*v)).collect())));
    fields.push(("cand_out", JVal::ArrS(rewrite.regs.iter().map(|v| hex16(*v)).collect())));
    fields.push(("differs_on", JVal::ArrS(differs_on)));
}

/// The fixed 256-byte pin used by the memory pairs (and only there): the
/// same formula the hardware oracle uses, at the sandbox base.
fn pairs_pin() -> ([u8; isuperoptimizer::state::MEM_BYTES], u64) {
    let mut mem = [0u8; isuperoptimizer::state::MEM_BYTES];
    for (i, b) in mem.iter_mut().enumerate() {
        *b = (i as u64 * 7 + 3) as u8;
    }
    (mem, isuperoptimizer::state::MEM_BASE)
}

fn to_mem(bytes: &[u8]) -> [u8; isuperoptimizer::state::MEM_BYTES] {
    let mut m = [0u8; isuperoptimizer::state::MEM_BYTES];
    let n = bytes.len().min(m.len());
    m[..n].copy_from_slice(&bytes[..n]);
    m
}

/// Hardware differential for the division/memory ops: divq/idivq (with the #DE
/// `def` protocol) and load/store (against the oracle's own static buffer
/// at its runtime address, discovered via the MEMBASE handshake).
fn run_hw_mem(n: u64, seed: u64, log: &mut ReportLog) -> u32 {
    if n == 0 {
        return 0;
    }
    let t0 = Instant::now();
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Command, Stdio};

    let mut child = match Command::new("./tools/hw_oracle")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("FAIL hw-mem: cannot spawn ./tools/hw_oracle: {}", e);
            return 1;
        }
    };
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());

    // MEMBASE handshake: the oracle's buffer address (ASLR-random per run).
    let mut first = String::new();
    if stdout.read_line(&mut first).unwrap_or(0) == 0 {
        eprintln!("FAIL hw-mem: oracle closed before MEMBASE");
        return 1;
    }
    let mem_base: u64 = first
        .trim()
        .strip_prefix("MEMBASE ")
        .and_then(|s| u64::from_str_radix(s, 16).ok())
        .unwrap_or_else(|| panic!("bad MEMBASE line: {}", first.trim()));
    // The concrete sandbox mirrors the oracle's byte formula.
    let mut mem = [0u8; isuperoptimizer::state::MEM_BYTES];
    for (i, b) in mem.iter_mut().enumerate() {
        *b = (i as u64 * 7 + 3) as u8;
    }

    let mut rng = Rng::new(seed);
    let mut discrepancies = 0u64;
    let mut cases = 0u64;
    let mut def_cases = 0u64;

    for _ in 0..n {
        for op in ["divq", "idivq", "load", "storeload"] {
            let a = rng.interesting_u64();
            let b = rng.interesting_u64();
            let rdx = rng.interesting_u64();
            let offset = rng.below((isuperoptimizer::state::MEM_BYTES - 8) as u64);
            let addr = mem_base + offset;

            let (prog, regs) = match op {
                "divq" => (
                    Program(vec![Slot::Instr(Instr::Div { src: Gpr::Rbx })]),
                    {
                        let mut r = [0u64; 16];
                        r[0] = a; r[3] = b; r[2] = rdx;
                        r
                    },
                ),
                "idivq" => (
                    Program(vec![Slot::Instr(Instr::IDiv { src: Gpr::Rbx })]),
                    {
                        let mut r = [0u64; 16];
                        r[0] = a; r[3] = b; r[2] = rdx;
                        r
                    },
                ),
                "load" => (
                    Program(vec![Slot::Instr(Instr::Load {
                        mem: MemAddr { base: Some(Gpr::Rax), index: None, scale: 1, disp: 0 },
                        dst: Gpr::Rcx,
                    })]),
                    {
                        let mut r = [0u64; 16];
                        r[0] = addr; r[3] = b;
                        r
                    },
                ),
                _ => (
                    Program(vec![
                        Slot::Instr(Instr::Store {
                            src: Gpr::Rbx,
                            mem: MemAddr { base: Some(Gpr::Rax), index: None, scale: 1, disp: 0 },
                        }),
                        Slot::Instr(Instr::Load {
                            mem: MemAddr { base: Some(Gpr::Rax), index: None, scale: 1, disp: 0 },
                            dst: Gpr::Rcx,
                        }),
                    ]),
                    {
                        let mut r = [0u64; 16];
                        r[0] = addr; r[3] = b;
                        r
                    },
                ),
            };
            let c = eval_program_in(&prog, regs, mem, mem_base);
            cases += 1;

            let line = if op == "divq" || op == "idivq" {
                format!("{} {:x} {:x} {:x}\n", op, a, b, rdx)
            } else {
                format!("{} {:x} {:x} {:x}\n", op, addr, b, 0)
            };
            if stdin.write_all(line.as_bytes()).and_then(|_| stdin.flush()).is_err() {
                break;
            }
            let mut resp = String::new();
            if stdout.read_line(&mut resp).unwrap_or(0) == 0 {
                break;
            }
            let resp = resp.trim();
            let fields: Vec<&str> = resp.split_whitespace().collect();

            let mut bad: Vec<String> = vec![];
            match op {
                "divq" | "idivq" => {
                    if resp == "def" {
                        def_cases += 1;
                        if c.def {
                            bad.push("hw says #DE but concrete says defined".into());
                        }
                    } else {
                        if !c.def {
                            bad.push(format!(
                                "hw divided ({} {}) but concrete says undefined",
                                fields[0], fields[1]
                            ));
                        } else if fields.len() >= 2 {
                            let hw_rax = u64::from_str_radix(fields[0], 16).unwrap_or(u64::MAX);
                            let hw_rdx = u64::from_str_radix(fields[1], 16).unwrap_or(u64::MAX);
                            if hw_rax != c.regs[0] {
                                bad.push(format!("rax: hw={:#018x} concrete={:#018x}", hw_rax, c.regs[0]));
                            }
                            if hw_rdx != c.regs[2] {
                                bad.push(format!("rdx: hw={:#018x} concrete={:#018x}", hw_rdx, c.regs[2]));
                            }
                        }
                    }
                }
                "load" => {
                    let hw = u64::from_str_radix(fields[0], 16).unwrap_or(u64::MAX);
                    if hw != c.regs[1] {
                        bad.push(format!("load: hw={:#018x} concrete={:#018x}", hw, c.regs[1]));
                    }
                    if !c.def {
                        bad.push("in-range load should be defined".into());
                    }
                }
                _ => {
                    let hw = u64::from_str_radix(fields[0], 16).unwrap_or(u64::MAX);
                    if hw != c.regs[1] || hw != b {
                        bad.push(format!(
                            "storeload: hw={:#018x} concrete={:#018x} expected(stored)={:#018x}",
                            hw, c.regs[1], b
                        ));
                    }
                    // The oracle's buffer is persistent: mirror the store so
                    // subsequent loads in this session compare against the
                    // same contents.
                    let off = offset as usize;
                    mem[off..off + 8].copy_from_slice(&b.to_le_bytes());
                }
            }
            if !bad.is_empty() {
                discrepancies += 1;
                if discrepancies <= 5 {
                    eprintln!(
                        "HW-MEM DISCREPANCY: op={} a={:#018x} b={:#018x} rdx={:#018x}\n{}",
                        op, a, b, rdx, bad.join("\n")
                    );
                }
            }
        }
    }
    drop(stdin);
    let _ = child.wait();

    let dt = t0.elapsed().as_secs_f64();
    log.event(&[
        ("kind", JVal::S("mode_summary".into())),
        ("mode", JVal::S("hw_mem".into())),
        ("status", JVal::S(if discrepancies == 0 { "pass".into() } else { "fail".into() })),
        ("cases", JVal::U(cases)),
        ("def_cases", JVal::U(def_cases)),
        ("discrepancies", JVal::U(discrepancies)),
        ("millis", JVal::U((dt * 1000.0) as u64)),
    ]);
    if discrepancies == 0 {
        println!(
            "PASS hw-mem : {} silicon cases ({} exercising #DE), 0 discrepancies ({:.1}s)",
            cases, def_cases, dt
        );
        0
    } else {
        println!("FAIL hw-mem : {} cases, {} DISCREPANCIES ({:.1}s)", cases, discrepancies, dt);
        1
    }
}

// ---------------------------------------------------------------- compiler oracle

/// The compiled-binary oracle (ADR-010): proves that the parser's
/// transcription of real `gcc -O2/-O3` output is behavior-preserving, by
/// calling the actual compiled functions through the real SysV ABI and
/// comparing against this project's concrete evaluator running the parsed
/// assembly. Runs per function per optimization level over corner + random
/// cases.
fn run_compiler_oracle(cases_per_fn: u64, seed: u64, batch: &str, log: &mut ReportLog) -> u32 {
    if cases_per_fn == 0 {
        return 0;
    }
    let t0 = Instant::now();
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Command, Stdio};
    use isuperoptimizer::compiler_core::{self, GCC_FLAGS_BASE};

    let repo_root = std::env::current_dir().expect("run from the repo root");
    let (batch_dir, parse_fn, registry, validate_fn): (
        &str,
        fn(&str) -> Result<Vec<(&'static str, isuperoptimizer::instr::Program)>, String>,
        &[compiler_core::CompilerFunc],
        fn(&str) -> Vec<(&'static str, String)>,
    ) = match batch {
        "core" => (
            "fixtures/compiler-core",
            compiler_core::parse_survivors,
            compiler_core::CORE_FUNCS,
            compiler_core::validate_exclusions,
        ),
        "full" => (
            "fixtures/compiler-full",
            isuperoptimizer::compiler_full::parse_survivors,
            isuperoptimizer::compiler_full::FULL_FUNCS,
            isuperoptimizer::compiler_full::validate_exclusions,
        ),
        other => {
            eprintln!("FAIL compiler-oracle: unknown batch `{}` (core|full)", other);
            return 1;
        }
    };
    let candidates = repo_root.join(batch_dir).join("candidates.c");
    if !candidates.exists() {
        eprintln!("FAIL compiler-oracle: {}/candidates.c not found (run from repo root)", batch_dir);
        return 1;
    }

    let mut discrepancies = 0u64;
    let mut total_cases = 0u64;

    for (level, opt_flag) in [("O2", "-O2"), ("O3", "-O3")] {
        let asm_path = repo_root.join(batch_dir).join(format!("candidates-{}.s", level));
        let asm_text = match std::fs::read_to_string(&asm_path) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("FAIL compiler-oracle: cannot read {}: {}", asm_path.display(), e);
                return 1;
            }
        };
        let survivors = match parse_fn(&asm_text) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("FAIL compiler-oracle ({}): parse: {}", level, e);
                return 1;
            }
        };

        // Build the harness from the committed candidates.c.
        let harness_src = compiler_core::harness_source_for(registry, &candidates.to_string_lossy());
        let harness_c = std::env::temp_dir().join(format!("isuperopt-harness-{}.c", level));
        let harness_bin = std::env::temp_dir().join(format!("isuperopt-harness-{}", level));
        if let Err(e) = std::fs::write(&harness_c, &harness_src) {
            eprintln!("FAIL compiler-oracle: cannot write harness: {}", e);
            return 1;
        }
        let cc = format!("cc {} {} -I{}", opt_flag, GCC_FLAGS_BASE, repo_root.to_string_lossy());
        let status = Command::new("sh")
            .arg("-c")
            .arg(format!("{} \"{}\" -o \"{}\"", cc, harness_c.display(), harness_bin.display()))
            .status();
        match status {
            Ok(s) if s.success() => {}
            other => {
                eprintln!("FAIL compiler-oracle: harness compilation failed: {:?}", other);
                return 1;
            }
        }

        let mut child = match Command::new(&harness_bin)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                eprintln!("FAIL compiler-oracle: cannot spawn harness: {}", e);
                return 1;
            }
        };
        let mut stdin = child.stdin.take().unwrap();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());

        let mut rng = Rng::new(seed ^ (level.len() as u64) << 32 ^ 0xC0DE);
        for (f, prog) in &survivors {
            let meta = registry.iter().find(|m| m.name == *f).unwrap();
            let mut fn_cases = 0u64;
            let mut fn_discrepancies = 0u64;

            // Corner cases first: interesting values in the ABI argument
            // registers (arg 0), zeros elsewhere.
            let corners = [
                0u64, 1, 2, 3, 4, 7, 8, 63, 64, 255, 0x7fff_ffff_ffff_ffff,
                0x8000_0000_0000_0000, 0xffff_ffff_ffff_ffff, 0x5555_5555_5555_5555,
                0xaaaa_aaaa_aaaa_aaaa, 0x0f0f_0f0f_0f0f_0f0f, 42,
            ];
            let mut inputs_list: Vec<[u64; 16]> = Vec::new();
            for &c in corners.iter() {
                let mut inp = [0u64; 16];
                inp[0] = c;
                if meta.nargs > 1 {
                    inp[1] = c.rotate_left(17) | 1; // a correlated-but-different second arg
                }
                if meta.nargs > 2 {
                    inp[2] = c.rotate_right(11) ^ 0xff;
                }
                inputs_list.push(inp);
            }
            for _ in corners.len() as u64..cases_per_fn {
                let mut inp = [0u64; 16];
                inp[0] = rng.interesting_u64();
                if meta.nargs > 1 {
                    inp[1] = rng.interesting_u64();
                }
                if meta.nargs > 2 {
                    inp[2] = rng.interesting_u64();
                }
                inputs_list.push(inp);
            }

            for inp in &inputs_list {
                // ABI: arg i -> RDI/RSI/RDX; our Gpr indices: Rdi=7, Rsi=6, Rdx=2.
                let mut regs = [0u64; 16];
                for i in 0..meta.nargs {
                    regs[compiler_core::arg_reg(i).idx()] = inp[i];
                }
                let c = eval_program(prog, regs);
                total_cases += 1;
                fn_cases += 1;

                let line = format!(
                    "{} {:x} {:x} {:x}\n",
                    f,
                    regs[7], regs[6], regs[2]
                );
                if stdin.write_all(line.as_bytes()).and_then(|_| stdin.flush()).is_err() {
                    break;
                }
                let mut resp = String::new();
                if stdout.read_line(&mut resp).unwrap_or(0) == 0 {
                    eprintln!("FAIL compiler-oracle: harness closed early on {}", f);
                    discrepancies += 1;
                    break;
                }
                let got = u64::from_str_radix(resp.trim(), 16).unwrap_or(u64::MAX);
                let want = c.regs[compiler_core::RET_REG.idx()];
                if got != want || !c.def {
                    fn_discrepancies += 1;
                    discrepancies += 1;
                    if fn_discrepancies <= 3 {
                        eprintln!(
                            "COMPILER-ORACLE DISCREPANCY {} ({}): input rdi={:#018x} rsi={:#018x} rdx={:#018x}: binary={:#018x} parsed={:#018x} def={}",
                            f, level, regs[7], regs[6], regs[2], got, want, c.def
                        );
                    }
                }
            }
            log.event(&[
                ("kind", JVal::S("compiler_oracle_func".into())),
                ("function", JVal::S(f.to_string())),
                ("level", JVal::S(level.into())),
                ("cases", JVal::U(fn_cases)),
                ("discrepancies", JVal::U(fn_discrepancies)),
                ("status", JVal::S(if fn_discrepancies == 0 { "pass".into() } else { "fail".into() })),
            ]);
            // per-fn status is logged above; the summary counts discrepancies
        }
        drop(stdin);
        let _ = child.wait();

        // Empirically confirm every logged exclusion reason for this level.
        for (name, observed) in validate_fn(&asm_text) {
            log.event(&[
                ("kind", JVal::S("exclusion_confirmed".into())),
                ("function", JVal::S(name.into())),
                ("level", JVal::S(level.into())),
                ("parse_error", JVal::S(observed)),
            ]);
        }
    }

    let dt = t0.elapsed().as_secs_f64();
    log.event(&[
        ("kind", JVal::S("mode_summary".into())),
        ("mode", JVal::S("compiler_oracle".into())),
        ("batch", JVal::S(batch.to_string())),
        ("status", JVal::S(if discrepancies == 0 { "pass".into() } else { "fail".into() })),
        ("cases", JVal::U(total_cases)),
        ("discrepancies", JVal::U(discrepancies)),
        ("millis", JVal::U((dt * 1000.0) as u64)),
    ]);
    if discrepancies == 0 {
        println!(
            "PASS compiler-oracle : {} ABI calls vs parsed-assembly evaluations across both levels, 0 discrepancies ({:.1}s)",
            total_cases, dt
        );
        0
    } else {
        println!(
            "FAIL compiler-oracle : {} cases, {} DISCREPANCIES ({:.1}s)",
            total_cases, discrepancies, dt
        );
        1
    }
}

fn run_pairs(log: &mut ReportLog) -> u32 {
    let mut z = match Z3Session::new() {
        Ok(z) => z,
        Err(e) => {
            eprintln!("FAIL pairs: {}", e);
            return 1;
        }
    };
    let mut failures = 0u32;
    let mut pair_events: u64 = 0;

    let equiv_pairs: &[(&str, &str, &[Gpr])] = &[
        // x*10 via imul vs strength-reduced shifts (ring identity mod 2^64)
        (
            "movq $10, %rcx\nimulq %rcx, %rax",
            "movq %rax, %rdx\nshlq $3, %rax\nshlq $1, %rdx\naddq %rdx, %rax",
            &[Gpr::Rax],
        ),
        // (a+b)-b == a (two's complement group identity)
        (
            "movq %rax, %rcx\naddq %rbx, %rcx\nsubq %rbx, %rcx",
            "movq %rax, %rcx",
            &[Gpr::Rcx],
        ),
        // x^x == 0 (the canonical zeroing idiom)
        ("xorq %rax, %rax", "movq $0, %rax", &[Gpr::Rax]),
        // signed min, two flag orderings
        (
            "cmpq %rbx, %rax\ncmovgq %rbx, %rax",
            "cmpq %rax, %rbx\ncmovlq %rbx, %rax",
            &[Gpr::Rax],
        ),
        // lea == add (same effective-address arithmetic)
        ("leaq (%rax,%rbx), %rcx", "movq %rax, %rcx\naddq %rbx, %rcx", &[Gpr::Rcx]),
        // cmp rax,rax; sete is always 1
        ("cmpq %rax, %rax\nsete %rcx", "movq $1, %rcx", &[Gpr::Rcx]),
        // Division: x mod 2 (unsigned) computed by DIVQ equals the
        // compiler's strength reduction x & 1. Dividend is zeroed RDX:RAX =
        // RAX, divisor 2: always defined; remainder lands in RDX.
        (
            "movq $2, %rcx\nxorq %rdx, %rdx\ndivq %rcx",
            "movq %rax, %rdx\nandq $1, %rdx",
            &[Gpr::Rdx],
        ),
        // Negation: 0 - x via the dedicated opcode equals the
        // sub-from-zero construction (flags differ, but flags are not
        // live-outs in v0).
        (
            "negq %rcx",
            "movq $0, %rdx\nsubq %rcx, %rdx\nmovq %rdx, %rcx",
            &[Gpr::Rcx],
        ),
        // NOT: the real opcode equals the xor-with-all-ones
        // construction. (The earlier broken pair used `xor $-1` as the NOT stand-in in the
        // broken-pair list because notq was not yet in the subset; with
        // notq admitted this equivalence is now checkable directly.)
        (
            "notq %rcx",
            "xorq $0xffffffffffffffff, %rcx",
            &[Gpr::Rcx],
        ),
    ];

    println!("hand-verified equivalences:");
    for (i, (t, c, live)) in equiv_pairs.iter().enumerate() {
        let spec = Spec { program: parse_program(t).unwrap(), live_outs: live.to_vec() };
        let cand = parse_program(c).unwrap();
        let mut f = pair_base_fields(&format!("equiv_pair_{}", i + 1), t, c, live);
        match verify(&cand, &spec, &mut z) {
            Ok(Verdict::Equivalent) => {
                println!("  PASS pair {}: equivalent", i + 1);
                f.push(("verdict", JVal::S("equivalent".into())));
            }
            Ok(Verdict::Counterexample(tc)) => {
                failures += 1;
                println!("  FAIL pair {}: claimed-equivalent pair got counterexample inputs={:?}", i + 1, tc.inputs);
                f.push(("verdict", JVal::S("counterexample".into())));
                log_counterexample(&mut f, &spec, &cand, &tc, None);
            }
            Err(e) => {
                failures += 1;
                println!("  FAIL pair {}: {}", i + 1, e);
                f.push(("verdict", JVal::S(format!("error: {}", e))));
            }
        }
        pair_events += 1;
        log.event(&f);
    }

    let broken_pairs: &[(&str, &str, &[Gpr])] = &[
        // deliberately broken: the plan's `a + (b>>1)` rewrite with the shift
        // count corrupted to 2, i.e. `a + (b>>2)` vs the target `(2a+b)>>1`.
        // (NB: this is a *structurally different expression*, not the target
        // shifted twice — see recheck.rs for the literal programs.)
        (
            "movq %rax, %rcx\naddq %rax, %rcx\naddq %rbx, %rcx\nshrq $1, %rcx",
            "movq %rax, %rcx\nshrq $2, %rbx\naddq %rbx, %rcx",
            &[Gpr::Rcx],
        ),
        // the plan's example "identity" — NOT an identity under wraparound:
        // (2a+b)>>1 == a+(b>>1) fails whenever 2a+b wraps (e.g. a = 2^63)
        (
            "movq %rax, %rcx\naddq %rax, %rcx\naddq %rbx, %rcx\nshrq $1, %rcx",
            "movq %rax, %rcx\nshrq $1, %rbx\naddq %rbx, %rcx",
            &[Gpr::Rcx],
        ),
        // Division: signed x mod 2 requires RDX to SIGN-EXTEND RAX
        // before IDIVQ (remainder takes the dividend's sign: 0 or -1). The
        // broken candidate zeroes RDX instead — the dividend becomes the
        // *unsigned* x, and for negative odd x the remainder is +1, not -1.
        // Both are defined everywhere; they diverge exactly on negative
        // odd inputs (e.g. rax = 0x8000000000000001).
        (
            "movq %rax, %rdx\nsarq $63, %rdx\nmovq $2, %rcx\nidivq %rcx",
            "xorq %rdx, %rdx\nmovq $2, %rcx\nidivq %rcx",
            &[Gpr::Rdx],
        ),
        // Negation vs NOT: the classic one-off-by-one confusion.
        // -x = ~x + 1, so `xor $-1` (NOT) differs from neg on every input
        // except x = 0 (both give 0 there... at x=0: neg=0, not=~0-1? no:
        // ~0 = -1 ≠ 0). They differ on every input including 0.
        (
            "negq %rcx",
            "xorq $0xffffffffffffffff, %rcx",
            &[Gpr::Rcx],
        ),
    ];

    println!("deliberately broken pairs (must yield counterexamples):");
    for (i, (t, c, live)) in broken_pairs.iter().enumerate() {
        let spec = Spec { program: parse_program(t).unwrap(), live_outs: live.to_vec() };
        let cand = parse_program(c).unwrap();
        let mut f = pair_base_fields(&format!("broken_pair_{}", i + 1), t, c, live);
        match verify(&cand, &spec, &mut z) {
            Ok(Verdict::Counterexample(tc)) => {
                // Independently re-check the counterexample with the concrete
                // evaluator: outputs must actually differ on a live-out.
                let target = eval_program(&spec.program, tc.inputs);
                let rewrite = eval_program(&cand, tc.inputs);
                let mut differs = false;
                for &g in live.iter() {
                    if target.regs[g.idx()] != rewrite.regs[g.idx()] {
                        differs = true;
                    }
                }
                f.push(("verdict", JVal::S("counterexample".into())));
                log_counterexample(&mut f, &spec, &cand, &tc, None);
                if differs {
                    let g = live[0];
                    println!(
                        "  PASS broken pair {}: counterexample rax={:#018x} rbx={:#018x} -> target {}={:#018x} vs candidate {}={:#018x}",
                        i + 1,
                        tc.inputs[0],
                        tc.inputs[3],
                        g.name(),
                        target.regs[g.idx()],
                        g.name(),
                        rewrite.regs[g.idx()]
                    );
                } else {
                    failures += 1;
                    println!("  FAIL broken pair {}: counterexample does not actually differ", i + 1);
                }
            }
            Ok(Verdict::Equivalent) => {
                failures += 1;
                println!("  FAIL broken pair {}: verifier said equivalent (unsound!)", i + 1);
                f.push(("verdict", JVal::S("equivalent".into())));
            }
            Err(e) => {
                failures += 1;
                println!("  FAIL broken pair {}: {}", i + 1, e);
                f.push(("verdict", JVal::S(format!("error: {}", e))));
            }
        }
        pair_events += 1;
        log.event(&f);
    }

    // Documented vacuous case (ADR-003): a target that is UB on every input
    // (setl reads OF after a 2-bit shift) verifies equivalent to anything.
    let vac_spec = Spec {
        program: parse_program("shlq $2, %rax\nsetl %rcx").unwrap(),
        live_outs: vec![Gpr::Rcx],
    };
    let vac_cand_src = "movq $0, %rcx";
    let vac_cand = parse_program(vac_cand_src).unwrap();
    let mut f = pair_base_fields("vacuous_ub_case", "shlq $2, %rax\nsetl %rcx", vac_cand_src, &[Gpr::Rcx]);
    match verify(&vac_cand, &vac_spec, &mut z) {
        Ok(Verdict::Equivalent) => {
            println!("  PASS vacuous-UB case: UB-everywhere target is (documented) vacuously equivalent");
            f.push(("verdict", JVal::S("equivalent".into())));
        }
        other => {
            failures += 1;
            println!("  FAIL vacuous-UB case: unexpected verdict {:?}", matches!(other, Ok(Verdict::Counterexample(_))));
            f.push(("verdict", JVal::S(format!("unexpected: {}", matches!(other, Ok(Verdict::Counterexample(_)))))));
        }
    }
    pair_events += 1;
    log.event(&f);

    // UB-introducing rewrite must be rejected: def_T true, def_R false.
    let def_spec = Spec {
        program: parse_program("movq %rax, %rcx").unwrap(),
        live_outs: vec![Gpr::Rcx],
    };
    let def_cand_src = "shlq $2, %rax\nsetl %rcx";
    let def_cand = parse_program(def_cand_src).unwrap();
    let mut f = pair_base_fields("def_rejection_case", "movq %rax, %rcx", def_cand_src, &[Gpr::Rcx]);
    match verify(&def_cand, &def_spec, &mut z) {
        Ok(Verdict::Counterexample(tc)) => {
            println!("  PASS def-rejection: rewrite introducing UB is rejected via def_R");
            f.push(("verdict", JVal::S("counterexample".into())));
            log_counterexample(&mut f, &def_spec, &def_cand, &tc, None);
        }
        Ok(Verdict::Equivalent) => {
            failures += 1;
            println!("  FAIL def-rejection: UB-introducing rewrite wrongly accepted");
            f.push(("verdict", JVal::S("equivalent".into())));
        }
        Err(e) => {
            failures += 1;
            println!("  FAIL def-rejection: {}", e);
            f.push(("verdict", JVal::S(format!("error: {}", e))));
        }
    }
    pair_events += 1;
    log.event(&f);

    // ---- Division/memory pairs (ADR-008 review gap): hand-verified store/
    // load equivalence over ALL initial memories, and a deliberately-broken
    // store offset whose counterexample is reproducible under a pinned
    // memory (logged with the pin so any replay reproduces it exactly).
    let (pin_mem, pin_base) = pairs_pin();
    let pin = isuperoptimizer::verify::PinnedMem { bytes: &pin_mem, base: pin_base };

    // equiv: store+load via two addressing routes — same address, same
    // bytes, same loaded value: rax -> [rbx+16] -> rcx, whether the
    // address is formed by the displacement or by an explicit add.
    {
        let t = "movq %rax, 16(%rbx)\nmovq 16(%rbx), %rcx";
        let c = "addq $16, %rbx\nmovq %rax, (%rbx)\nmovq (%rbx), %rcx";
        let spec = Spec { program: parse_program(t).unwrap(), live_outs: vec![Gpr::Rcx] };
        let cand = parse_program(c).unwrap();
        let mut f = pair_base_fields("equiv_mem_pair", t, c, &[Gpr::Rcx]);
        match isuperoptimizer::verify::verify_in(&cand, &spec, &mut z, None) {
            Ok(Verdict::Equivalent) => {
                println!("  PASS memory equivalence pair: store+load via two addressing routes equivalent over ALL initial memories");
                f.push(("verdict", JVal::S("equivalent (z3, unpinned: over all initial memories; solver claim)".into())));
            }
            Ok(Verdict::Counterexample(tc)) => {
                failures += 1;
                println!("  FAIL memory equivalence pair: counterexample inputs={:?}", tc.inputs);
                f.push(("verdict", JVal::S("counterexample".into())));
                log_counterexample(&mut f, &spec, &cand, &tc, None);
            }
            Err(e) => {
                failures += 1;
                println!("  FAIL memory equivalence pair: {}", e);
                f.push(("verdict", JVal::S(format!("error: {}", e))));
            }
        }
        pair_events += 1;
        log.event(&f);
    }

    // broken: the store goes to the WRONG offset — the target's write set
    // and the loaded-back register both diverge; verified under the pinned
    // memory so the witness is concretely reproducible.
    {
        let t = "movq %rax, 16(%rbx)\nmovq 16(%rbx), %rcx";
        let c = "movq %rax, 8(%rbx)\nmovq 16(%rbx), %rcx";
        let spec = Spec { program: parse_program(t).unwrap(), live_outs: vec![Gpr::Rcx] };
        let cand = parse_program(c).unwrap();
        let mut f = pair_base_fields("broken_mem_pair", t, c, &[Gpr::Rcx]);
        match isuperoptimizer::verify::verify_in(&cand, &spec, &mut z, Some(&pin)) {
            Ok(Verdict::Counterexample(tc)) => {
                let target = eval_program_in(&spec.program, tc.inputs, pin_mem, pin_base);
                let rewrite = eval_program_in(&cand, tc.inputs, pin_mem, pin_base);
                let differs = target.regs[1] != rewrite.regs[1];
                f.push(("verdict", JVal::S("counterexample".into())));
                log_counterexample(&mut f, &spec, &cand, &tc, Some((&pin_mem, pin_base)));
                if differs {
                    println!(
                        "  PASS broken memory pair: wrong store offset — counterexample rax={:#018x} rbx={:#018x} -> target rcx={:#018x} vs candidate rcx={:#018x} (pinned memory; witness logged with the pin)",
                        tc.inputs[0], tc.inputs[3], target.regs[1], rewrite.regs[1]
                    );
                } else {
                    failures += 1;
                    println!("  FAIL broken memory pair: counterexample does not actually differ under the pin");
                }
            }
            Ok(Verdict::Equivalent) => {
                failures += 1;
                println!("  FAIL broken memory pair: verifier said equivalent (unsound!)");
                f.push(("verdict", JVal::S("equivalent".into())));
            }
            Err(e) => {
                failures += 1;
                println!("  FAIL broken memory pair: {}", e);
                f.push(("verdict", JVal::S(format!("error: {}", e))));
            }
        }
        pair_events += 1;
        log.event(&f);
    }

    // ---- HISTORICAL BOUNDARY NON-WITNESS (VERIFICATION.md rows 17 & 23).
    // The first broken_pair_3 run — before the `bvsle` fix — produced a
    // SPURIOUS counterexample at exactly this input: the broken candidate's
    // quotient is exactly i64::MAX (defined), which the strict upper bound
    // misclassified as undefined, so the miter "counterexampled" a pair
    // whose outputs are equal. The discovery witness lived in a debugging
    // artifact that was overwritten by the passing re-run and then deleted
    // — the evidence trail was destroyed (row 23). This event is the
    // permanent, replayable record: it logs the boundary input as a
    // NON-witness, which recheck re-derives concretely (outputs reproduce,
    // both programs defined, outputs equal).
    {
        let t = "movq %rax, %rdx\nsarq $63, %rdx\nmovq $2, %rcx\nidivq %rcx";
        let c = "xorq %rdx, %rdx\nmovq $2, %rcx\nidivq %rcx";
        let mut inputs = [0u64; 16];
        inputs[0] = 0xfffffffffffffffe; // rax = -2: candidate quotient == i64::MAX
        let tp = parse_program(t).unwrap();
        let cp = parse_program(c).unwrap();
        let ts = eval_program(&tp, inputs);
        let cs = eval_program(&cp, inputs);
        let equal = ts.regs[2] == cs.regs[2];
        let both_defined = ts.def && cs.def;
        log.event(&[
            ("kind", JVal::S("boundary_case".into())),
            ("name", JVal::S("idiv_quotient_i64_max_boundary".into())),
            ("target", JVal::S(t.into())),
            ("candidate", JVal::S(c.into())),
            ("live_outs", JVal::ArrS(vec!["rdx".into()])),
            ("inputs", JVal::ArrS(inputs.iter().map(|v| hex16(*v)).collect())),
            ("target_out", JVal::ArrS(ts.regs.iter().map(|v| hex16(*v)).collect())),
            ("cand_out", JVal::ArrS(cs.regs.iter().map(|v| hex16(*v)).collect())),
            ("target_defined", JVal::B(ts.def)),
            ("cand_defined", JVal::B(cs.def)),
            ("outputs_equal", JVal::B(equal)),
            (
                "note",
                JVal::S(
                    "pre-fix signed #DE predicate (strict bvslt upper bound) classified the \
                     candidate's quotient — exactly i64::MAX — as undefined here, producing a \
                     spurious counterexample with equal outputs. This input is a NON-witness: \
                     both programs defined, outputs equal. Committed regression tests pin the \
                     same boundary through the fold and z3, including the historical buggy \
                     predicate text (tests/division_memory.rs, idiv_quotient_i64_max_boundary_is_defined)."
                        .into(),
                ),
            ),
        ]);
        pair_events += 1;
        if equal && both_defined {
            println!("  PASS boundary case: i64::MAX quotient is defined; the historical spurious-counterexample input is a logged, replayable non-witness");
        } else {
            failures += 1;
            println!("  FAIL boundary case: expected both-defined + equal outputs at the i64::MAX quotient boundary");
        }
    }

    log.event(&[
        ("kind", JVal::S("mode_summary".into())),
        ("mode", JVal::S("pairs".into())),
        ("status", JVal::S(if failures == 0 { "pass".into() } else { "fail".into() })),
        ("verdicts", JVal::U(pair_events)),
        ("failures", JVal::U(failures as u64)),
    ]);
    if failures == 0 {
        println!("PASS pairs  : all hand pairs behave as specified");
        0
    } else {
        println!("FAIL pairs  : {} failures", failures);
        1
    }
}
