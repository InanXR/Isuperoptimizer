//! The four core acceptance criteria, in a form `cargo test` runs on every
//! commit. The millions-scale evidence comes from `cargo run --release --bin
//! stress`; this file pins the same criteria at CI scale so a regression can
//! never land silently.
//!
//! Criterion 1: symbolic_eval ≡ concrete_eval (differential, zero discrepancies)
//! Criterion 2: verify(p,p) is UNSAT for programs in the subset
//! Criterion 3: hand-verified equivalences hold
//! Criterion 4: a deliberately broken pair yields a *correct* counterexample

use isuperoptimizer::fuzz::{all_single_instr_programs, random_program, random_state, written_regs};
use isuperoptimizer::instr::{parse_program, Gpr};
use isuperoptimizer::rng::Rng;
use isuperoptimizer::state::eval_program;
use isuperoptimizer::sym::{sym_eval_program, SState};
use isuperoptimizer::verify::{verify, Spec, Verdict};
use isuperoptimizer::z3proc::Z3Session;

const BULK_PROGRAMS: u64 = 30_000;
const BULK_STATES: u64 = 3;

/// Criterion 1: concrete ≡ folded-symbolic on random programs/states,
/// comparing registers, all flag values, all flag definednesses, and `def`.
#[test]
fn criterion_1_differential_zero_discrepancies() {
    let mut rng = Rng::new(0xACCE_55ED);
    let mut checked = 0u64;
    for _ in 0..BULK_PROGRAMS {
        let p = random_program(&mut rng, 8);
        let sym = sym_eval_program(&p, SState::from_vars());
        for _ in 0..BULK_STATES {
            let st = random_state(&mut rng);
            let c = eval_program(&p, st);
            for r in 0..16 {
                assert_eq!(
                    sym.regs[r].eval(&st),
                    c.regs[r],
                    "register r{} mismatch for program:\n{}",
                    r, p
                );
            }
            assert_eq!(sym.flags.zf.eval(&st), c.flags.zf, "zf mismatch\n{}", p);
            assert_eq!(sym.flags.sf.eval(&st), c.flags.sf, "sf mismatch\n{}", p);
            assert_eq!(sym.flags.cf.eval(&st), c.flags.cf, "cf mismatch\n{}", p);
            assert_eq!(sym.flags.of.eval(&st), c.flags.of, "of mismatch\n{}", p);
            assert_eq!(sym.flags.zf_def.eval(&st), c.flags.zf_def, "zf_def mismatch\n{}", p);
            assert_eq!(sym.flags.sf_def.eval(&st), c.flags.sf_def, "sf_def mismatch\n{}", p);
            assert_eq!(sym.flags.cf_def.eval(&st), c.flags.cf_def, "cf_def mismatch\n{}", p);
            assert_eq!(sym.flags.of_def.eval(&st), c.flags.of_def, "of_def mismatch\n{}", p);
            assert_eq!(sym.def.eval(&st), c.def, "def mismatch\n{}", p);
            checked += 1;
        }
    }
    eprintln!("criterion 1: {} program-states, 0 discrepancies", checked);
}

/// Criterion 2: verify(p,p) must be UNSAT — sampled across the operand grid
/// and random multi-instruction programs.
#[test]
fn criterion_2_self_verification_unsat() {
    let mut z = Z3Session::new().expect("z3 binary must be available (see ADR-005)");
    let mut count = 0u64;

    // Sample every opcode class from the full grid (the stress binary runs the
    // complete grid; here we take a stride to keep CI fast).
    let grid = all_single_instr_programs();
    for p in grid.iter().step_by(37) {
        let spec = Spec { program: p.clone(), live_outs: written_regs(p) };
        match verify(p, &spec, &mut z) {
            Ok(Verdict::Equivalent) => count += 1,
            Ok(Verdict::Counterexample(_)) => panic!("self-verification found a counterexample for {}", p),
            Err(e) => panic!("self-verification error for {}: {}", p, e),
        }
    }

    let mut rng = Rng::new(0x5E1F);
    for _ in 0..60 {
        let p = random_program(&mut rng, 6);
        let spec = Spec { program: p.clone(), live_outs: written_regs(&p) };
        match verify(&p, &spec, &mut z) {
            Ok(Verdict::Equivalent) => count += 1,
            Ok(Verdict::Counterexample(_)) => panic!("self-verification found a counterexample for {}", p),
            Err(e) => panic!("self-verification error for {}: {}", p, e),
        }
    }
    eprintln!("criterion 2: verify(p,p) UNSAT for {} programs (grid sample + random)", count);
}

/// Criterion 3: hand-verified equivalences must verify as Equivalent.
#[test]
fn criterion_3_hand_verified_equivalences() {
    let mut z = Z3Session::new().expect("z3 binary must be available");

    let pairs: &[(&str, &str, Gpr)] = &[
        // x*10: imul vs strength-reduced shifts (mod-2^64 ring identity)
        (
            "movq $10, %rcx\nimulq %rcx, %rax",
            "movq %rax, %rdx\nshlq $3, %rax\nshlq $1, %rdx\naddq %rdx, %rax",
            Gpr::Rax,
        ),
        // (a+b)-b == a
        (
            "movq %rax, %rcx\naddq %rbx, %rcx\nsubq %rbx, %rcx",
            "movq %rax, %rcx",
            Gpr::Rcx,
        ),
        // x^x == 0
        ("xorq %rax, %rax", "movq $0, %rax", Gpr::Rax),
        // signed min, two flag orderings
        (
            "cmpq %rbx, %rax\ncmovgq %rbx, %rax",
            "cmpq %rax, %rbx\ncmovlq %rbx, %rax",
            Gpr::Rax,
        ),
        // lea == add
        ("leaq (%rax,%rbx), %rcx", "movq %rax, %rcx\naddq %rbx, %rcx", Gpr::Rcx),
        // cmp rax,rax; sete is always 1
        ("cmpq %rax, %rax\nsete %rcx", "movq $1, %rcx", Gpr::Rcx),
    ];

    for (i, (t, c, live)) in pairs.iter().enumerate() {
        let spec = Spec { program: parse_program(t).unwrap(), live_outs: vec![*live] };
        let cand = parse_program(c).unwrap();
        match verify(&cand, &spec, &mut z) {
            Ok(Verdict::Equivalent) => {}
            Ok(Verdict::Counterexample(tc)) => panic!(
                "pair {} claimed equivalent but got counterexample inputs={:?}",
                i + 1,
                tc.inputs
            ),
            Err(e) => panic!("pair {} errored: {}", i + 1, e),
        }
    }
    eprintln!("criterion 3: {} hand-verified equivalences all verified Equivalent", pairs.len());
}

/// Criterion 4: a deliberately broken pair must produce a counterexample that
/// is *correct* when independently checked with the concrete evaluator.
#[test]
fn criterion_4_broken_pair_yields_correct_counterexample() {
    let mut z = Z3Session::new().expect("z3 binary must be available");

    // Broken shift-by-2 instead of 1.
    let spec = Spec {
        program: parse_program(
            "movq %rax, %rcx\naddq %rax, %rcx\naddq %rbx, %rcx\nshrq $1, %rcx",
        )
        .unwrap(),
        live_outs: vec![Gpr::Rcx],
    };
    let cand = parse_program("movq %rax, %rcx\nshrq $2, %rbx\naddq %rbx, %rcx").unwrap();

    match verify(&cand, &spec, &mut z) {
        Ok(Verdict::Counterexample(tc)) => {
            // Independent check: run both programs by hand on the model inputs.
            let target = eval_program(&spec.program, tc.inputs);
            let rewrite = eval_program(&cand, tc.inputs);
            assert_ne!(
                target.regs[Gpr::Rcx.idx()],
                rewrite.regs[Gpr::Rcx.idx()],
                "counterexample must actually distinguish the programs: inputs={:?}",
                tc.inputs
            );
            assert_eq!(target.regs[Gpr::Rcx.idx()], tc.outputs[Gpr::Rcx.idx()]);
            eprintln!(
                "criterion 4: counterexample rax={:#018x} rbx={:#018x}: target rcx={:#018x}, broken candidate rcx={:#018x}",
                tc.inputs[0], tc.inputs[3], target.regs[Gpr::Rcx.idx()], rewrite.regs[Gpr::Rcx.idx()]
            );
        }
        Ok(Verdict::Equivalent) => panic!("broken pair verified equivalent — the verifier would be unsound"),
        Err(e) => panic!("broken pair errored: {}", e),
    }

    // Bonus, discovered during implementation: the plan's example pair
    // ((2a+b)>>1 vs a+(b>>1)) is NOT an identity under two's-complement
    // wraparound, and the verifier must correctly reject it.
    let spec = Spec {
        program: parse_program(
            "movq %rax, %rcx\naddq %rax, %rcx\naddq %rbx, %rcx\nshrq $1, %rcx",
        )
        .unwrap(),
        live_outs: vec![Gpr::Rcx],
    };
    let cand = parse_program("movq %rax, %rcx\nshrq $1, %rbx\naddq %rbx, %rcx").unwrap();
    match verify(&cand, &spec, &mut z) {
        Ok(Verdict::Counterexample(tc)) => {
            let target = eval_program(&spec.program, tc.inputs);
            let rewrite = eval_program(&cand, tc.inputs);
            assert_ne!(target.regs[Gpr::Rcx.idx()], rewrite.regs[Gpr::Rcx.idx()]);
            eprintln!(
                "criterion 4 (bonus): plan's (2a+b)>>1 'identity' correctly rejected: rax={:#018x} rbx={:#018x} -> target={:#018x} candidate={:#018x}",
                tc.inputs[0], tc.inputs[3], target.regs[Gpr::Rcx.idx()], rewrite.regs[Gpr::Rcx.idx()]
            );
        }
        Ok(Verdict::Equivalent) => panic!("wraparound counterexample missed: the identity is false for 2a+b >= 2^64"),
        Err(e) => panic!("{}", e),
    }

    // UB-introducing rewrite must be rejected via def_R (ADR-003).
    let spec = Spec {
        program: parse_program("movq %rax, %rcx").unwrap(),
        live_outs: vec![Gpr::Rcx],
    };
    let cand = parse_program("shlq $2, %rax\nsetl %rcx").unwrap();
    match verify(&cand, &spec, &mut z) {
        Ok(Verdict::Counterexample(_)) => eprintln!("criterion 4 (bonus): UB-introducing rewrite rejected via def"),
        Ok(Verdict::Equivalent) => panic!("UB-introducing rewrite accepted — def contract broken"),
        Err(e) => panic!("{}", e),
    }
}
