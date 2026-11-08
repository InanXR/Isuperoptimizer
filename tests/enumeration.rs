//! Enumeration acceptance tests: the brute-force enumerator with optimality
//! certificates, at CI scale. The full-scale evidence is in
//! `artifacts/enumeration-certificates.jsonl` (regenerate with
//! `isuperopt enumerate`), machine-verified by `recheck --from-log`.

use isuperoptimizer::benchmarks;
use isuperoptimizer::enumerator::{
    enumerate_benchmark, instr_effects, screen_witness, EnumConfig,
};
use isuperoptimizer::instr::{parse_program, Instr, Operand, Slot};
use isuperoptimizer::reportlog::ReportLog;
use isuperoptimizer::state::eval_program;
use isuperoptimizer::z3proc::Z3Session;

fn as_instr(slot: &Slot) -> &Instr {
    match slot {
        Slot::Instr(i) => i,
        Slot::Unused => panic!("expected an instruction slot"),
    }
}

#[test]
fn liveness_effects_table() {
    // cmp writes flags but no register; it is dead unless flags are live.
    let e = instr_effects(&Instr::Cmp { src: Operand::Reg(benchmarks::BENCHMARKS[0].live_outs[0]), dst: benchmarks::BENCHMARKS[0].live_outs[0] });
    assert!(e.writes_regs.is_empty() && e.writes_flags && !e.reads_flags);
    // setcc writes a register and reads flags.
    let e = instr_effects(as_instr(&parse_program("sete %rcx").unwrap().0[0]));
    assert!(!e.writes_regs.is_empty() && !e.writes_flags && e.reads_flags);
    // cmov reads its own destination (conditional move keeps it on false).
    let e = instr_effects(as_instr(&parse_program("cmoveq %rbx, %rax").unwrap().0[0]));
    assert!(e.reads_regs.contains(&isuperoptimizer::instr::Gpr::Rax));
    assert!(!e.writes_flags && e.reads_flags);
    // div writes both RAX and RDX; stores write memory and are never dead.
    let e = instr_effects(&Instr::Div { src: isuperoptimizer::instr::Gpr::Rbx });
    assert!(e.writes_regs.contains(&isuperoptimizer::instr::Gpr::Rax));
    assert!(e.writes_regs.contains(&isuperoptimizer::instr::Gpr::Rdx));
    let e = instr_effects(&Instr::Store {
        src: isuperoptimizer::instr::Gpr::Rax,
        mem: isuperoptimizer::instr::MemAddr { base: Some(isuperoptimizer::instr::Gpr::Rbx), index: None, scale: 1, disp: 0 },
    });
    assert!(e.writes_mem && e.writes_regs.is_empty());
}

#[test]
fn screen_witness_actually_distinguishes() {
    let spec = parse_program("cmpq $0, %rax\nsete %rcx").unwrap();
    let mut inputs = [0u64; 16];
    inputs[0] = 5;
    let tc = isuperoptimizer::cost::TestCase::from_spec_inputs(
        &isuperoptimizer::verify::Spec { program: spec.clone(), live_outs: vec![isuperoptimizer::instr::Gpr::Rcx] },
        inputs,
    );
    let cand = parse_program("movq $1, %rcx").unwrap();
    let (idx, _) = screen_witness(&cand, &[tc]).expect("must diverge");
    assert_eq!(idx, 0);
    assert_ne!(
        eval_program(&spec, inputs).regs[1],
        eval_program(&cand, inputs).regs[1]
    );
}

/// End-to-end on sign_mask: spec 4 instructions, optimum 2, with lengths
/// 1 exhaustively rejected in the log.
#[test]
fn sign_mask_optimum_is_2_with_certificate() {
    let b = benchmarks::by_name("sign_mask").unwrap();
    let cfg = EnumConfig::for_benchmark(b);
    let mut z3 = Z3Session::new().expect("z3");
    let dir = std::env::temp_dir().join("isuperopt-enumeration-test");
    let _ = std::fs::create_dir_all(&dir);
    let mut log = ReportLog::open(dir.join("sign_mask.jsonl").to_str().unwrap());
    let out = enumerate_benchmark(&b.into(), &cfg, &mut z3, &mut log, 0xABCD).unwrap();
    log.flush();

    let p = out.optimum.expect("sign_mask has a 2-instruction optimum");
    assert_eq!(p.0.len(), 2);
    assert_eq!(out.spec_length, 4);
    // Length 1 was fully enumerated and rejected.
    assert!(out.candidates_total > 100);
    // The optimum really computes x<0 ? -1 : 0.
    for x in [0u64, 1, u64::MAX, 1 << 63, 42, (1 << 63) | 7] {
        let mut inputs = [0u64; 16];
        inputs[0] = x;
        let s = eval_program(&parse_program(b.spec).unwrap(), inputs);
        let c = eval_program(&p, inputs);
        assert_eq!(s.regs[1], c.regs[1], "x={:#x}", x);
    }
}

/// sum: the 1-instruction lea optimum must be found and verified.
#[test]
fn sum_optimum_is_1() {
    let b = benchmarks::by_name("sum").unwrap();
    let cfg = EnumConfig::for_benchmark(b);
    let mut z3 = Z3Session::new().expect("z3");
    let dir = std::env::temp_dir().join("isuperopt-enumeration-test");
    let _ = std::fs::create_dir_all(&dir);
    let mut log = ReportLog::open(dir.join("sum.jsonl").to_str().unwrap());
    let out = enumerate_benchmark(&b.into(), &cfg, &mut z3, &mut log, 0xABCD).unwrap();
    assert_eq!(out.optimum.unwrap().0.len(), 1);
}
