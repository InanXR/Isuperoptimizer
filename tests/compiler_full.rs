//! Full-batch acceptance tests: the full-provenance reconciliation (23 survivors,
//! 8 empirically-confirmed exclusions), the `notq` extension's concrete
//! semantics (flags UNAFFECTED), and the salq alias.

use isuperoptimizer::instr::{parse_gcc_asm, parse_program};
use isuperoptimizer::state::eval_program;

/// Every full-batch survivor parses from the committed assembly at both
/// levels, and every exclusion's logged reason is confirmed by the actual
/// parse error (no silent exclusions, ADR-010 discipline).
#[test]
fn full_batch_survivors_and_exclusions() {
    for level in ["O2", "O3"] {
        let asm = std::fs::read_to_string(format!("fixtures/compiler-full/candidates-{}.s", level))
            .expect("run from repo root");
        let survivors = isuperoptimizer::compiler_full::parse_survivors(&asm).unwrap();
        assert_eq!(survivors.len(), 23, "23 survivors at {}", level);
        for (name, prog) in &survivors {
            assert!(!prog.0.is_empty(), "{} has an empty program", name);
        }
        let exclusions = isuperoptimizer::compiler_full::validate_exclusions(&asm);
        assert_eq!(exclusions.len(), 8, "8 exclusions at {}", level);
        for (name, observed) in &exclusions {
            assert!(
                observed != "MISSING FROM ASSEMBLY"
                    && !observed.starts_with("UNEXPECTEDLY PARSED"),
                "{}: exclusion reason not confirmed ({})",
                name,
                observed
            );
            // the observed parse error must name a real subset violation
            assert!(
                observed.contains("is not in the v0 subset")
                    || observed.contains("partial-register")
                    || observed.contains("control flow"),
                "{}: unexpected parse error: {}",
                name,
                observed
            );
        }
    }
    assert_eq!(isuperoptimizer::compiler_full::FULL_EXCLUDED.len(), 8);
}

/// The seeded-phase `best` must NOT be pre-seeded with the start program:
/// seeded starts are deliberate structural mutations of the gcc form and
/// are NOT equivalent to the spec. Regression pin for VERIFICATION.md
/// row 31 — the first straggler run reported mutated variants as chain
/// bests, which the final long-timeout verification then refuted.
#[test]
fn seeded_phase_does_not_preseed_best_with_the_start() {
    use isuperoptimizer::instr::parse_program;
    use isuperoptimizer::mcmc::{run_chain, McmcConfig};
    use isuperoptimizer::verify::Spec;
    use isuperoptimizer::instr::Gpr;

    let spec = Spec {
        program: parse_program("leaq -1(%rdi), %rax\nandq %rdi, %rax").unwrap(),
        live_outs: vec![Gpr::Rax],
    };
    // a structural mutation of the spec: drop the and (NOT equivalent)
    let mutated = parse_program("leaq -1(%rdi), %rax").unwrap();
    let cfg = McmcConfig::paper(1, 1, 1);
    // zero budget: the loop never runs, so the outcome is exactly the
    // pre-seed decision.
    let out = run_chain(&spec, &cfg, "seeded_optimization", 0, 1, 0, mutated.clone()).unwrap();
    assert!(out.best.is_none(), "seeded chains must not report the (non-equivalent) start as best");
    // the paper's optimization phase DOES pre-seed with its known-correct
    // start (the spec itself) — that behavior is unchanged.
    let spec_prog = spec.program.clone();
    let out = run_chain(&spec, &cfg, "optimization", 0, 1, 0, spec_prog).unwrap();
    assert!(out.best.is_some(), "optimization chains keep the spec as best baseline");
    assert_eq!(out.best_perf, 2);
}

/// ABI register contract (ADR-011, row 33): every register the MCMC can
/// pick is caller-clobbered — a claimed winner must be a drop-in
/// replacement for the gcc function (clobbering RSP/RBP/RBX/R12-R15 would
/// crash on ret or break the caller).
#[test]
fn mcmc_registers_are_abi_scratch_only() {
    use isuperoptimizer::instr::Gpr;
    use isuperoptimizer::mcmc::random_reg;
    use isuperoptimizer::rng::Rng;
    let callee_saved = [
        Gpr::Rbx, Gpr::Rsp, Gpr::Rbp, Gpr::R12, Gpr::R13, Gpr::R14, Gpr::R15,
    ];
    let mut rng = Rng::new(1234);
    for _ in 0..2000 {
        let r = random_reg(&mut rng);
        assert!(
            !callee_saved.contains(&r),
            "MCMC picked callee-saved register {}",
            r.name()
        );
    }
}

/// The enumeration destination universe for gcc specs is ABI-scratch: the
/// identity floor still holds (spec write-registers are unioned in —
/// pinned by p19 whose gcc form writes %r8, an MCMC-scoped bench).
#[test]
fn for_spec_dst_is_abi_scratch_but_floor_covers_spec_writes() {
    use isuperoptimizer::enumerator::EnumConfig;
    use isuperoptimizer::instr::Gpr;
    let cfg = EnumConfig::for_spec(
        "probe",
        "movq %rdi, %r8\nshrq %cl, %rdi\nxorq %r8, %rdi\nandq %rsi, %rdi\nmovq %rdi, %rax\nxorq %rdi, %r8\nsalq %cl, %rax\nxorq %r8, %rax",
        &[Gpr::Rax],
        3,
    );
    assert!(!cfg.dst_regs.contains(&Gpr::Rbx), "RBX is callee-saved: not a drop-in destination");
    assert!(cfg.dst_regs.contains(&Gpr::Rax));
    // the identity floor: the spec's own %r8 write must be a legal dst
    assert!(cfg.dst_regs.contains(&Gpr::R8), "spec write-registers must be in the universe");
}

/// Encodability contract (ADR-011): the searched subset is restricted to
/// x86-encodable ALU immediates, while the *model* stays permissive so
/// committed artifacts' rejection witnesses keep replaying.
#[test]
fn encodability_of_the_searched_subset() {
    use isuperoptimizer::instr::{parse_instr, Gpr, Instr, Operand};
    use isuperoptimizer::mcmc::paper_imm_alu;
    use isuperoptimizer::rng::Rng;

    // 1. ALU-slot immediates drawn by MCMC are always imm32-encodable.
    let mut rng = Rng::new(99);
    for _ in 0..1000 {
        let v = paper_imm_alu(&mut rng);
        assert!(
            isuperoptimizer::instr::imm32_encodable(v),
            "paper_imm_alu produced non-encodable {v:#x}"
        );
    }

    // 2. The enumeration alphabet carries wide constants ONLY on Mov.
    use isuperoptimizer::enumerator::{alphabet, EnumConfig};
    use isuperoptimizer::instr::MemAddr;
    let wide = 0x5555_5555_5555_5555u64;
    let cfg = EnumConfig {
        dst_regs: vec![Gpr::Rax],
        src_regs: vec![Gpr::Rdi],
        imms: vec![wide, 1],
        shift_counts: vec![1],
        lea_forms: vec![MemAddr { base: Some(Gpr::Rax), index: None, scale: 1, disp: 0 }],
        include_cl_shifts: false,
        max_length: 2,
        witness_log_threshold: 2000,
        initial_testcases: 8,
    };
    let a = alphabet(&cfg);
    assert!(a.contains(&Instr::Mov { src: Operand::Imm(wide), dst: Gpr::Rax }));
    for probe in [
        Instr::And { src: Operand::Imm(wide), dst: Gpr::Rax },
        Instr::Add { src: Operand::Imm(wide), dst: Gpr::Rax },
        Instr::Xor { src: Operand::Imm(wide), dst: Gpr::Rax },
    ] {
        assert!(!a.contains(&probe), "alphabet must not carry wide ALU immediates: {probe:?}");
    }
    // encodable immediates stay on the ALU forms
    assert!(a.contains(&Instr::And { src: Operand::Imm(1), dst: Gpr::Rax }));

    // 3. The PARSER stays permissive (the model accepts wide ALU immediates
    // as an abstraction; committed rejection witnesses replay through it).
    // If this ever tightens, every committed artifact containing wide-imm
    // witnesses stops re-checking — a deliberate, documented decision.
    assert!(parse_instr("andq $0x5555555555555555, %rax").is_ok());
    assert!(parse_instr("movq $0x5555555555555555, %rax").is_ok());
}

/// The batch covers the full Gulwani P1-P25 provenance: the 23 survivors
/// plus the 8 exclusions account for all 25 P-programs (5 of them via the
/// core-batch survivors that are themselves P-programs) plus the 6 core-batch
/// extras — 31 distinct compiler-emitted functions in the committed
/// assembly.
#[test]
fn full_batch_covers_the_provenance() {
    let asm = std::fs::read_to_string("fixtures/compiler-full/candidates-O2.s").unwrap();
    let names: Vec<String> = isuperoptimizer::compiler_core::split_asm_functions(&asm)
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    assert_eq!(names.len(), 31, "31 functions in the batch");
    // every registry entry exists in the assembly, and vice versa
    let mut expect: Vec<&str> = isuperoptimizer::compiler_full::FULL_FUNCS
        .iter()
        .map(|f| f.name)
        .collect();
    expect.extend(isuperoptimizer::compiler_full::FULL_EXCLUDED.iter().map(|(n, _)| *n));
    for n in &expect {
        assert!(names.iter().any(|x| x == n), "{} missing from assembly", n);
    }
}

/// `notq` concrete semantics: dst := !dst and ALL FOUR FLAGS UNTOUCHED —
/// both values and definedness carry through (the SDM's NOT, unlike NEG,
/// performs no flag updates). This is the property the hardware oracle
/// validates on silicon with the sub-preset protocol.
#[test]
fn notq_complements_and_leaves_flags_untouched() {
    // Set distinct flag values with a sub, then notq: flags must survive.
    let prog = parse_program("movq $7, %rbx\nmovq $3, %rax\nsubq %rbx, %rax\nnotq %rax").unwrap();
    let s = eval_program(&prog, [0u64; 16]);
    // rax = 3 - 7 = -4, then notq: ~(-4) = 3
    assert_eq!(s.regs[0], 3u64, "notq complements: ~(3-7) == 3");
    // sub 3 - 7: result -4 -> ZF=0 SF=1 CF=1 OF=0
    let before = eval_program(&parse_program("movq $7, %rbx\nmovq $3, %rax\nsubq %rbx, %rax").unwrap(), [0u64; 16]);
    assert_eq!(s.flags.zf, before.flags.zf);
    assert_eq!(s.flags.sf, before.flags.sf);
    assert_eq!(s.flags.cf, before.flags.cf);
    assert_eq!(s.flags.of, before.flags.of);
    assert_eq!(s.flags.zf_def, before.flags.zf_def);
    assert_eq!(s.flags.sf_def, before.flags.sf_def);
    assert_eq!(s.flags.cf_def, before.flags.cf_def);
    assert_eq!(s.flags.of_def, before.flags.of_def);
    assert!(s.def);

    // and NOT of an UNDEFINED-flag state stays undefined (untouched means
    // untouched: notq cannot manufacture definedness)
    let s = eval_program(&parse_program("notq %rax").unwrap(), [5u64; 16]);
    assert_eq!(s.regs[0], !5u64);
    assert!(!s.flags.zf_def && !s.flags.sf_def && !s.flags.cf_def && !s.flags.of_def);
    // ...but the register write itself is defined
    assert!(s.def);
}

/// notq ≡ xorq $-1 concretely, and ≢ negq (the classic confusion).
#[test]
fn not_eq_xor_minus_one_not_neg() {
    for x in [0u64, 1, 5, 0x8000000000000000, u64::MAX, 0xdeadbeefcafebabe] {
        let mut st = [0u64; 16];
        st[0] = x;
        let notp = eval_program(&parse_program("notq %rax").unwrap(), st);
        let xorp = eval_program(&parse_program("xorq $-1, %rax").unwrap(), st);
        let negp = eval_program(&parse_program("negq %rax").unwrap(), st);
        assert_eq!(notp.regs[0], xorp.regs[0]);
        assert_ne!(notp.regs[0], negp.regs[0]);
    }
}

/// The GNU-notes trailer fix: the last function in the file (sat_add, a
/// survivor) must parse cleanly even though `.note.gnu.property` labels
/// (`0:`, `1:`) follow it. parse_gcc_asm would correctly REJECT those
/// labels ("label after ret") if they landed in a chunk — the fix keeps
/// them out of every chunk (chunks close at `.LFE`).
#[test]
fn trailer_labels_do_not_poison_the_last_function() {
    let asm = std::fs::read_to_string("fixtures/compiler-full/candidates-O2.s").unwrap();
    let sat = asm
        .split('\n')
        .any(|l| l.trim() == "0:");
    assert!(sat, "fixture must contain the trailer label this test guards against");
    // the core batch's own artifact still reproduces: the avg2 exclusion is the
    // andl error, unchanged by the .LFE chunk-close fix.
    let core_asm = std::fs::read_to_string("fixtures/compiler-core/candidates-O2.s").unwrap();
    let excl = isuperoptimizer::compiler_core::validate_exclusions(&core_asm);
    let (_, avg2_err) = excl.iter().find(|(n, _)| *n == "avg2").unwrap();
    assert!(avg2_err.contains("andl"), "avg2 reason unchanged: {}", avg2_err);
}

/// gcc_syntax round-trips for the full-batch forms: salq (shlq synonym),
/// notq, variable %cl shifts in real compiler output (p19).
#[test]
fn full_batch_gcc_syntax_forms() {
    assert_eq!(
        parse_program("salq %cl, %rax").unwrap(),
        parse_program("shlq %cl, %rax").unwrap()
    );
    let fs = parse_gcc_asm("f:\n\tnotq %rdi\n\tret\n").unwrap();
    assert_eq!(fs[0].1 .0.len(), 1);
}
