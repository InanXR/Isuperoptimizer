//! Search acceptance tests: the STOKE-style MCMC sampler, at CI scale. The
//! full-scale evidence lives in `artifacts/mcmc-search.jsonl` (regenerate
//! with `isuperopt search`), machine-verified by `recheck --from-log`.

use isuperoptimizer::enumerator::strip_dead;
use isuperoptimizer::instr::{parse_program, Slot};
use isuperoptimizer::mcmc::{
    apply_move, eq_prime_early, full_cost, latency, paper_imm, pick_kind, program_latency,
    seed_testcases, McmcConfig, MoveKind,
};
use isuperoptimizer::rng::Rng;

#[test]
fn move_probabilities_match_the_paper() {
    let cfg = McmcConfig::paper(1, 1, 1);
    let mut rng = Rng::new(99);
    let n = 200_000;
    let mut counts = [0u64; 4];
    for _ in 0..n {
        match pick_kind(&cfg, &mut rng) {
            MoveKind::Opcode => counts[0] += 1,
            MoveKind::Operand => counts[1] += 1,
            MoveKind::Swap => counts[2] += 1,
            MoveKind::Instruction => counts[3] += 1,
        }
    }
    let f = |c: u64| c as f64 / n as f64;
    // Paper: p_c=0.16, p_o=0.5, p_s=0.16, p_i=0.16 (Fig. 10).
    assert!((f(counts[0]) - 0.16).abs() < 0.02, "opcode {}", f(counts[0]));
    assert!((f(counts[1]) - 0.50).abs() < 0.02, "operand {}", f(counts[1]));
    assert!((f(counts[2]) - 0.16).abs() < 0.02, "swap {}", f(counts[2]));
    assert!((f(counts[3]) - 0.16).abs() < 0.02, "instruction {}", f(counts[3]));
}

#[test]
fn paper_immediates_match_the_published_set() {
    let mut rng = Rng::new(7);
    let mut saw_paper_small = false;
    let mut saw_power_of_two = false;
    let mut saw_negative_power = false;
    let mut saw_swar_extension = false;
    for _ in 0..5000 {
        let v = paper_imm(&mut rng);
        let sv = v as i64;
        if (-16..=16).contains(&sv) {
            saw_paper_small = true;
        }
        if v.count_ones() == 1 {
            saw_power_of_two = true;
        }
        if (!(v)).count_ones() == 1 {
            saw_negative_power = true;
        }
        if matches!(v, 0x55 | 0x33 | 0x0f) {
            saw_swar_extension = true;
        }
    }
    assert!(saw_paper_small && saw_power_of_two && saw_negative_power && saw_swar_extension);
}

#[test]
fn moves_preserve_shape_and_length() {
    let cfg = McmcConfig::paper(1, 1, 1);
    let mut rng = Rng::new(3);
    let start = parse_program("movq %rax, %rcx\naddq $1, %rdx\nsete %rbx").unwrap();
    let padded = {
        let mut v = start.0.clone();
        v.resize(cfg.ell, Slot::Unused);
        isuperoptimizer::instr::Program(v)
    };
    for _ in 0..2000 {
        let kind = pick_kind(&cfg, &mut rng);
        let m = apply_move(&padded, kind, &cfg, &mut rng);
        assert_eq!(m.0.len(), cfg.ell);
        // every non-unused slot still parses/validates structurally: the
        // Display round-trip must re-parse
        let _ = isuperoptimizer::instr::parse_program(&m.to_string()).expect("roundtrip");
    }
}

#[test]
fn latency_table() {
    assert_eq!(latency(&Slot::Unused), 0);
    assert_eq!(
        latency(&Slot::Instr(isuperoptimizer::instr::Instr::Imul {
            src: isuperoptimizer::instr::Gpr::Rbx,
            dst: isuperoptimizer::instr::Gpr::Rax
        })),
        3
    );
    let p = parse_program("movq %rax, %rcx\naddq %rbx, %rcx").unwrap();
    assert_eq!(program_latency(&p), 2);
}

#[test]
fn eq_prime_early_matches_full_cost() {
    let spec = isuperoptimizer::verify::Spec {
        program: parse_program("cmpq $0, %rax\nsete %rcx").unwrap(),
        live_outs: vec![isuperoptimizer::instr::Gpr::Rcx],
    };
    let cases = seed_testcases(&spec, 8, 42);
    let mut rng = Rng::new(11);
    for _ in 0..200 {
        let prog = isuperoptimizer::fuzz::random_program(&mut rng, 6);
        let full_eq: u64 = cases
            .iter()
            .map(|t| isuperoptimizer::cost::case_cost(
                &isuperoptimizer::state::eval_program(&prog, t.inputs).regs, t,
            ))
            .sum();
        for bound in [1u64, 3, 7, 1 << 40] {
            let early = eq_prime_early(&prog, &cases, bound);
            match early {
                Some(e) => {
                    assert_eq!(e, full_eq, "partial accumulation must equal the full eq'");
                }
                None => {
                    assert!(full_eq >= bound, "early exit only when the bound is truly exceeded");
                }
            }
        }
        // and the full MCMC cost is eq' + perf
        assert_eq!(full_cost(&prog, &cases), program_latency(&prog) + full_eq);
    }
}

/// strip_dead removes provably-dead instructions and preserves behavior on
/// every logged testcase (the full behavioral proof is the dead-code lemma;
/// this is the cheap end-to-end check).
#[test]
fn strip_dead_preserves_testcase_behavior() {
    let spec = isuperoptimizer::verify::Spec {
        program: parse_program(
            "movq %rax, %rcx\nmovq $0, %rdx\nsubq %rax, %rdx\ncmpq $0, %rax\ncmovlq %rdx, %rcx",
        )
        .unwrap(),
        live_outs: vec![isuperoptimizer::instr::Gpr::Rcx],
    };
    let cases = seed_testcases(&spec, 16, 5);
    let mut rng = Rng::new(13);
    for _ in 0..500 {
        let prog = isuperoptimizer::fuzz::random_program(&mut rng, 10);
        let stripped = strip_dead(&prog, &spec.live_outs);
        for t in &cases {
            let a = isuperoptimizer::state::eval_program(&prog, t.inputs);
            let b = isuperoptimizer::state::eval_program(&stripped, t.inputs);
            for g in &spec.live_outs {
                assert_eq!(
                    a.regs[g.idx()],
                    b.regs[g.idx()],
                    "strip_dead changed a live-out: prog:\n{}",
                    prog
                );
            }
        }
        assert!(program_latency(&stripped) <= program_latency(&prog));
    }
}
