//! Standalone counterexample re-checker — concrete evaluator ONLY, no z3, no
//! summarization. Created after the early report audit: the prose report
//! mis-transcribed a counterexample input (rbx), which hand-verification
//! correctly caught. This binary is now the canonical way to audit any
//! counterexample by hand: it takes register inputs, runs the literal
//! programs through the concrete evaluator, and prints a per-instruction
//! trace. Reports must quote machine output verbatim; this tool exists so
//! nobody has to trust prose.
//!
//! Usage: recheck                — runs the recorded cases (see below)
//!        recheck <rax> <rbx>    — hex inputs, runs both pairs on them
//!        recheck --from-log [path] — re-verifies every counterexample in the
//!                                   stress JSONL log against the concrete
//!                                   evaluator (default path:
//!                                   artifacts/stress-log.jsonl)
//!
//! The program strings below are byte-identical to the pair definitions in
//! src/bin/stress.rs (copy-pasted, not re-typed from memory).

use isuperoptimizer::instr::{parse_program, Gpr, Slot};
use isuperoptimizer::reportlog::{get, parse_jsonl_line, JVal};
use isuperoptimizer::state::{eval_instr, eval_program, CState};
use std::collections::HashMap;

const PAIR1_TARGET: &str = "movq %rax, %rcx\naddq %rax, %rcx\naddq %rbx, %rcx\nshrq $1, %rcx";
const PAIR1_CANDIDATE: &str = "movq %rax, %rcx\nshrq $2, %rbx\naddq %rbx, %rcx";
const PAIR2_TARGET: &str = "movq %rax, %rcx\naddq %rax, %rcx\naddq %rbx, %rcx\nshrq $1, %rcx";
const PAIR2_CANDIDATE: &str = "movq %rax, %rcx\nshrq $1, %rbx\naddq %rbx, %rcx";

fn trace(label: &str, src: &str, rax: u64, rbx: u64) -> u64 {
    let p = parse_program(src).expect("literal programs must parse");
    let mut regs = [0u64; 16];
    regs[0] = rax; // rax
    regs[3] = rbx; // rbx
    let mut st = CState::from_regs(regs);
    println!("  [{}] rax={:#018x} rbx={:#018x}", label, rax, rbx);
    for line in src.split('\n') {
        println!("      {}", line);
    }
    for slot in &p.0 {
        if let Slot::Instr(i) = slot {
            st = eval_instr(i, &st);
            println!("      after {:<34} rcx={:#018x} rbx={:#018x}", i, st.regs[1], st.regs[3]);
        }
    }
    println!("      => final rcx = {:#018x}", st.regs[1]);
    st.regs[1]
}

fn parse_hex(s: &str) -> u64 {
    u64::from_str_radix(s.trim_start_matches("0x"), 16)
        .unwrap_or_else(|e| panic!("bad hex value `{}`: {}", s, e))
}

fn get_s<'a>(fields: &'a [(String, JVal)], key: &str) -> String {
    match get(fields, key) {
        Some(JVal::S(s)) => s.clone(),
        other => panic!("field `{}` missing or not a string: {:?}", key, other),
    }
}

fn get_arr(fields: &[(String, JVal)], key: &str) -> Vec<String> {
    match get(fields, key) {
        Some(JVal::ArrS(a)) => a.clone(),
        other => panic!("field `{}` missing or not a string array: {:?}", key, other),
    }
}

/// Re-verify the artifact of record. Handles:
///   - pair_verdict / rejection_witness: full concrete re-check of logged
///     counterexample/witness values
///   - testcase: the logged spec_out must reproduce from the logged spec
///   - optimum: the logged optimum program must match the spec on every
///     testcase logged so far for that benchmark (cost 0)
///   - bench_start / enumeration_* / *_summary: echoed raw (the numbers of
///     record); enumeration coverage itself is re-derivable by re-running
///     the committed enumerator (deterministic, seeded).
/// Equivalent-verdict events are skipped (checking those is the solver's
/// job, not the concrete auditor's). Returns a process exit code.
fn recheck_from_log(path: &str) -> i32 {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("cannot read log `{}`: {}", path, e);
            return 2;
        }
    };
    println!("re-checking artifact of record: {}", path);
    let mut verified = 0;
    // Per-kind tally: the reported count must trace to source without
    // reverse-engineering (an external-audit finding — the count
    // included bench_start validations that ADR-009's table described as
    // merely "echoed", producing a 22-event gap an auditor had to
    // reverse-engineer). Every `verified += 1` below increments its kind.
    let mut verified_by_kind: std::collections::BTreeMap<String, u64> = std::collections::BTreeMap::new();
    let mut failed = 0;
    let mut skipped = 0;

    // benchmark name -> (spec program, live_outs, logged testcases)
    let mut benches: HashMap<String, (isuperoptimizer::instr::Program, Vec<Gpr>, Vec<([u64; 16], [u64; 16])>)> = HashMap::new();

    for (idx, line) in content.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let lineno = idx + 1;
        let fields = match parse_jsonl_line(line) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("{}:{}: parse error: {}", path, lineno, e);
                return 2;
            }
        };
        let kind = match get(&fields, "kind") {
            Some(JVal::S(s)) => s.clone(),
            _ => String::new(),
        };
        let bench = match get(&fields, "benchmark") {
            Some(JVal::S(s)) => s.clone(),
            _ => String::new(),
        };

        if kind == "bench_start" {
            let spec_src = get_s(&fields, "spec");
            let live: Vec<Gpr> = get_arr(&fields, "live_outs")
                .iter()
                .filter_map(|n| Gpr::from_name(n))
                .collect();
            let prog = parse_program(&spec_src).expect("logged spec must parse");
            println!("event line {}: bench {} spec:", lineno, bench);
            for l in spec_src.split('\n') {
                println!("    {}", l);
            }
            benches.insert(bench.clone(), (prog, live, Vec::new()));
            verified += 1;
            verified_by_kind
                .entry("bench_start".to_string())
                .and_modify(|c| *c += 1)
                .or_insert(1);
            continue;
        }

        if kind == "testcase" {
            let inputs: Vec<String> = get_arr(&fields, "inputs");
            let spec_out: Vec<String> = get_arr(&fields, "spec_out");
            let (spec, _live, cases) = match benches.get_mut(&bench) {
                Some(b) => b,
                None => {
                    println!("FAIL  line {}: testcase for unknown benchmark {}", lineno, bench);
                    failed += 1;
                    continue;
                }
            };
            let mut regs = [0u64; 16];
            for i in 0..16 {
                regs[i] = parse_hex(&inputs[i]);
            }
            let got = eval_program(spec, regs);
            let mut bad = vec![];
            for i in 0..16 {
                let want = parse_hex(&spec_out[i]);
                if got.regs[i] != want {
                    bad.push(format!("spec_out[{}]: logged={:#018x} recomputed={:#018x}", i, want, got.regs[i]));
                }
            }
            if bad.is_empty() {
                println!("PASS  testcase {} #{}: spec_out reproduces", bench, get(&fields, "index").map(|v| match v { JVal::U(n) => n.to_string(), _ => "?".into() }).unwrap_or_default());
                verified += 1;
                verified_by_kind.entry("testcase".to_string()).and_modify(|c| *c += 1).or_insert(1);
            } else {
                println!("FAIL  testcase {} (line {}): {}", bench, lineno, bad.join("; "));
                failed += 1;
            }
            cases.push((regs, {
                let mut o = [0u64; 16];
                for i in 0..16 {
                    o[i] = parse_hex(&spec_out[i]);
                }
                o
            }));
            continue;
        }

        if kind == "rejection_witness" {
            let cand_src = get_s(&fields, "candidate");
            let inputs: Vec<String> = get_arr(&fields, "inputs");
            let spec_out: Vec<String> = get_arr(&fields, "spec_out");
            let cand_out: Vec<String> = get_arr(&fields, "cand_out");
            let origin = get_s(&fields, "origin");
            let (spec, live, _) = match benches.get(&bench) {
                Some(b) => b,
                None => {
                    println!("FAIL  line {}: rejection_witness for unknown benchmark {}", lineno, bench);
                    failed += 1;
                    continue;
                }
            };
            let mut regs = [0u64; 16];
            for i in 0..16 {
                regs[i] = parse_hex(&inputs[i]);
            }
            let cand = match parse_program(&cand_src) {
                Ok(p) => p,
                Err(e) => {
                    println!("FAIL  line {}: witness candidate does not parse: {}", lineno, e);
                    failed += 1;
                    continue;
                }
            };
            let t = eval_program(spec, regs);
            let c = eval_program(&cand, regs);
            let mut bad: Vec<String> = vec![];
            for i in 0..16 {
                let want_s = parse_hex(&spec_out[i]);
                if t.regs[i] != want_s {
                    bad.push(format!("spec_out[{}]: logged={:#018x} recomputed={:#018x}", i, want_s, t.regs[i]));
                }
                let want_c = parse_hex(&cand_out[i]);
                if c.regs[i] != want_c {
                    bad.push(format!("cand_out[{}]: logged={:#018x} recomputed={:#018x}", i, want_c, c.regs[i]));
                }
            }
            let mut differs: Vec<String> = vec![];
            for g in live {
                if t.regs[g.idx()] != c.regs[g.idx()] {
                    differs.push(g.name().to_string());
                }
            }
            // Screen rejections must actually distinguish on a live-out
            // (cost > 0 ⟹ divergence); z3 rejections may be def-based with
            // coinciding concrete outputs.
            if origin == "screen" && differs.is_empty() {
                bad.push("screen witness does not diverge on any live-out".into());
            }
            if bad.is_empty() {
                println!(
                    "PASS  rejection_witness {} (line {}, origin {}): candidate {} rejected, differs_on={:?}",
                    bench, lineno, origin, cand_src.replace('\n', " ; "), differs
                );
                verified += 1;
                verified_by_kind.entry("rejection_witness".to_string()).and_modify(|c| *c += 1).or_insert(1);
            } else {
                println!("FAIL  rejection_witness {} (line {}): {}", bench, lineno, bad.join("; "));
                failed += 1;
            }
            continue;
        }

        if kind == "boundary_case" {
            // A historical non-witness: the logged input must reproduce both
            // programs' outputs, both must be defined, and their live-outs
            // must be EQUAL — the input does not distinguish the pair, which
            // is exactly what the pre-fix predicate got wrong.
            let t_src = get_s(&fields, "target");
            let c_src = get_s(&fields, "candidate");
            let name = get_s(&fields, "name");
            let live = get_arr(&fields, "live_outs");
            let inputs = get_arr(&fields, "inputs");
            let target_out = get_arr(&fields, "target_out");
            let cand_out = get_arr(&fields, "cand_out");
            if inputs.len() != 16 || target_out.len() != 16 || cand_out.len() != 16 {
                println!("FAIL  boundary_case {} (line {}): malformed arrays", name, lineno);
                failed += 1;
                continue;
            }
            let mut regs = [0u64; 16];
            for i in 0..16 {
                regs[i] = parse_hex(&inputs[i]);
            }
            let t = eval_program(&parse_program(&t_src).expect("logged target must parse"), regs);
            let c = eval_program(&parse_program(&c_src).expect("logged candidate must parse"), regs);
            let mut bad: Vec<String> = vec![];
            for i in 0..16 {
                if t.regs[i] != parse_hex(&target_out[i]) {
                    bad.push(format!("target_out[{}]: logged={:#018x} recomputed={:#018x}", i, parse_hex(&target_out[i]), t.regs[i]));
                }
                if c.regs[i] != parse_hex(&cand_out[i]) {
                    bad.push(format!("cand_out[{}]: logged={:#018x} recomputed={:#018x}", i, parse_hex(&cand_out[i]), c.regs[i]));
                }
            }
            if let (Some(JVal::B(td)), Some(JVal::B(cd))) =
                (get(&fields, "target_defined"), get(&fields, "cand_defined"))
            {
                let (td, cd) = (*td, *cd);
                if t.def != td {
                    bad.push(format!("target_defined: logged={} recomputed={}", td, t.def));
                }
                if c.def != cd {
                    bad.push(format!("cand_defined: logged={} recomputed={}", cd, c.def));
                }
                if !td || !cd {
                    bad.push("boundary case must have both programs defined".into());
                }
            } else {
                bad.push("missing target_defined/cand_defined fields".into());
            }
            for lname in &live {
                match Gpr::from_name(lname) {
                    Some(g) => {
                        if t.regs[g.idx()] != c.regs[g.idx()] {
                            bad.push(format!(
                                "boundary input must NOT distinguish the pair, but {} differs ({:#018x} vs {:#018x})",
                                lname, t.regs[g.idx()], c.regs[g.idx()]
                            ));
                        }
                    }
                    None => bad.push(format!("live_out `{}` is not a register", lname)),
                }
            }
            if bad.is_empty() {
                println!(
                    "PASS  boundary_case {} (line {}): input reproduces as a non-witness (outputs equal, both defined)",
                    name, lineno
                );
                verified += 1;
                verified_by_kind.entry("boundary_case".to_string()).and_modify(|c| *c += 1).or_insert(1);
            } else {
                println!("FAIL  boundary_case {} (line {}): {}", name, lineno, bad.join("; "));
                failed += 1;
            }
            continue;
        }

        if kind == "optimum" || kind == "search_result" {
            // Only claimed-equivalent results are validated; "NOT claimed"
            // verdicts are recorded as skips.
            if let Some(JVal::S(v)) = get(&fields, "verdict") {
                if !v.starts_with("equivalent") {
                    println!(
                        "skip  {} {} (line {}): verdict `{}` — not an equivalence claim",
                        kind, bench, lineno, v
                    );
                    skipped += 1;
                    continue;
                }
            }
            let prog_src = get_s(&fields, "program");
            let (spec, live, cases) = match benches.get(&bench) {
                Some(b) => b,
                None => {
                    println!("FAIL  line {}: optimum for unknown benchmark {}", lineno, bench);
                    failed += 1;
                    continue;
                }
            };
            let prog = match parse_program(&prog_src) {
                Ok(p) => p,
                Err(e) => {
                    println!("FAIL  line {}: optimum program does not parse: {}", lineno, e);
                    failed += 1;
                    continue;
                }
            };
            let mut bad: Vec<String> = vec![];
            for (i, (inputs, _)) in cases.iter().enumerate() {
                let t = eval_program(spec, *inputs);
                let c = eval_program(&prog, *inputs);
                for g in live {
                    if t.regs[g.idx()] != c.regs[g.idx()] {
                        bad.push(format!("testcase {}: {} mismatch: spec={:#018x} cand={:#018x}", i, g.name(), t.regs[g.idx()], c.regs[g.idx()]));
                    }
                }
            }
            if bad.is_empty() {
                println!(
                    "PASS  {} {} (line {}): program matches spec on all {} logged testcases (equivalence itself is the logged z3 claim)",
                    kind, bench, lineno, cases.len()
                );
                verified += 1;
                verified_by_kind.entry(kind.clone()).and_modify(|c| *c += 1).or_insert(1);
            } else {
                println!("FAIL  {} {} (line {}): {}", kind, bench, lineno, bad.join("; "));
                failed += 1;
            }
            continue;
        }

        // Everything else: pair_verdict handling (the initial pair suite) and raw echoes.
        if kind != "pair_verdict" {
            if kind == "mode_summary" || kind == "run_summary" || kind == "enumeration_summary"
                || kind == "enumeration_length" || kind == "enumeration_universe"
                || kind == "mcmc_summary" || kind == "mcmc_config" || kind == "chain_result" {
                println!("event line {}: {} — echoed raw below:", lineno, kind);
                println!("  {}", line);
            }
            skipped += 1;
            continue;
        }
        let pair = get_s(&fields, "pair");
        let verdict = match get(&fields, "verdict") {
            Some(JVal::S(s)) => s.clone(),
            other => {
                println!("skip  {} (line {}): verdict not a string: {:?}", pair, lineno, other);
                skipped += 1;
                continue;
            }
        };
        if verdict != "counterexample" {
            println!(
                "skip  {} (line {}): verdict={} — equivalence claims need the solver, out of scope for the concrete auditor",
                pair, lineno, verdict
            );
            skipped += 1;
            continue;
        }

        // Full re-verification of a logged counterexample.
        let t_src = get_s(&fields, "target");
        let c_src = get_s(&fields, "candidate");
        let live = get_arr(&fields, "live_outs");
        let inputs = get_arr(&fields, "inputs");
        let target_out = get_arr(&fields, "target_out");
        let cand_out = get_arr(&fields, "cand_out");
        let differs_on = get_arr(&fields, "differs_on");
        if inputs.len() != 16 || target_out.len() != 16 || cand_out.len() != 16 {
            println!("FAIL  {} (line {}): malformed counterexample arrays", pair, lineno);
            failed += 1;
            continue;
        }
        let mut regs = [0u64; 16];
        for i in 0..16 {
            regs[i] = parse_hex(&inputs[i]);
        }
        // Pinned-memory pair counterexamples (division/memory pairs) carry
        // their pin; the replay must run under it to reproduce the values.
        let pin: Option<([u8; 256], u64)> = match (
            get(&fields, "mem_base"),
            get(&fields, "mem_pin"),
        ) {
            (Some(JVal::S(base_s)), Some(JVal::ArrS(bytes))) => {
                let base = parse_hex(base_s);
                let mut mem = [0u8; 256];
                if bytes.len() != 256 {
                    println!("FAIL  {} (line {}): mem_pin has {} bytes, expected 256", pair, lineno, bytes.len());
                    failed += 1;
                    continue;
                }
                for (i, b) in bytes.iter().enumerate() {
                    mem[i] = u8::from_str_radix(b.trim_start_matches("0x"), 16)
                        .unwrap_or_else(|e| panic!("bad mem_pin byte `{}`: {}", b, e));
                }
                Some((mem, base))
            }
            _ => None,
        };
        let run = |src: &str| -> isuperoptimizer::state::CState {
            let p = parse_program(src).expect("logged program must parse");
            match &pin {
                Some((mem, base)) => isuperoptimizer::state::eval_program_in(&p, regs, *mem, *base),
                None => eval_program(&p, regs),
            }
        };
        let t = run(&t_src);
        let c = run(&c_src);

        let mut bad: Vec<String> = vec![];
        for i in 0..16 {
            let want_t = parse_hex(&target_out[i]);
            if t.regs[i] != want_t {
                bad.push(format!("target_out[{}]: logged={:#018x} recomputed={:#018x}", i, want_t, t.regs[i]));
            }
            let want_c = parse_hex(&cand_out[i]);
            if c.regs[i] != want_c {
                bad.push(format!("cand_out[{}]: logged={:#018x} recomputed={:#018x}", i, want_c, c.regs[i]));
            }
        }
        let mut actual_differs: Vec<String> = vec![];
        for name in &live {
            match Gpr::from_name(name) {
                Some(g) => {
                    if t.regs[g.idx()] != c.regs[g.idx()] {
                        actual_differs.push(name.clone());
                    }
                }
                None => bad.push(format!("live_out `{}` is not a register", name)),
            }
        }
        if actual_differs != differs_on {
            bad.push(format!(
                "differs_on: logged={:?} recomputed={:?}",
                differs_on, actual_differs
            ));
        }

        if bad.is_empty() {
            println!("PASS  {} (line {}): logged counterexample reproduces exactly", pair, lineno);
            if let Some(first) = live.first() {
                let g = Gpr::from_name(first).unwrap();
                println!(
        "      inputs rax={} rbx={} -> target {}={} candidate {}={} (differs_on={:?})",
        inputs[0], inputs[3], first, hex(t.regs[g.idx()]), first, hex(c.regs[g.idx()]), differs_on
                );
            }
            verified += 1;
            verified_by_kind.entry("pair_verdict".to_string()).and_modify(|c| *c += 1).or_insert(1);
        } else {
            println!("FAIL  {} (line {}): {}", pair, lineno, bad.join("; "));
            failed += 1;
        }
    }
    println!(
        "\nlog re-check: {} events re-verified exactly ({}), {} FAILED, {} events skipped",
        verified,
        verified_by_kind
            .iter()
            .map(|(k, c)| format!("{}={}", k, c))
            .collect::<Vec<_>>()
            .join(" "),
        failed,
        skipped
    );
    if failed == 0 { 0 } else { 1 }
}

fn hex(v: u64) -> String {
    format!("{:#018x}", v)
}

fn run_pair(name: &str, target_src: &str, cand_src: &str, rax: u64, rbx: u64) {
    println!("=== {} @ rax={:#018x} rbx={:#018x} ===", name, rax, rbx);
    let t = trace("target", target_src, rax, rbx);
    let c = trace("candidate", cand_src, rax, rbx);
    if t != c {
        println!("  DIFFER: target rcx={:#018x} candidate rcx={:#018x}  -> valid counterexample\n", t, c);
    } else {
        println!("  SAME: target rcx={:#018x} == candidate rcx={:#018x}\n", t, c);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    // `recheck --from-log [path]`: re-verify the artifact of record itself.
    if args.first().map(|s| s.as_str()) == Some("--from-log") {
        let path = args.get(1).cloned().unwrap_or_else(|| "artifacts/stress-log.jsonl".to_string());
        let code = recheck_from_log(&path);
        std::process::exit(code);
    }

    let extra: Vec<(u64, u64)> = if args.len() >= 2 {
        vec![(
            u64::from_str_radix(args[0].trim_start_matches("0x"), 16).expect("rax hex"),
            u64::from_str_radix(args[1].trim_start_matches("0x"), 16).expect("rbx hex"),
        )]
    } else {
        vec![]
    };

    let mut cases: Vec<(&str, u64, u64)> = vec![
        ("pair1 broken, stress model inputs", 0xc000_0000_0000_0000, 0x6),
        ("pair2 plan-identity, stress model inputs", 0x8000_0000_0000_0000, 0x2),
        ("pair2 plan-identity, hand-check assumed rbx", 0x8000_0000_0000_0000, 0x8000_0000_0000_0002),
    ];
    for (rax, rbx) in extra {
        cases.push(("command-line inputs", rax, rbx));
    }

    for (label, rax, rbx) in cases {
        let (t, c) = if label.starts_with("pair1") {
            (PAIR1_TARGET, PAIR1_CANDIDATE)
        } else {
            (PAIR2_TARGET, PAIR2_CANDIDATE)
        };
        run_pair(label, t, c, rax, rbx);
    }
}
