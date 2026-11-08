//! Core-batch acceptance tests: entry-flag definedness (ADR-010) and the
//! gcc-assembly parsing layer.

use isuperoptimizer::instr::{parse_gcc_asm, parse_instr, parse_program};
use isuperoptimizer::state::eval_program;
use isuperoptimizer::verify::{verify, Spec, Verdict};
use isuperoptimizer::z3proc::Z3Session;

/// Per the SysV AMD64 ABI, incoming RFLAGS are caller-clobbered: reading a
/// flag at function ENTRY is a definedness violation. (Earlier phases modeled
/// entry flags as defined-false, which let `sete`-at-entry produce fake
/// constants — found by running real gcc output through the pipeline.)
#[test]
fn entry_flags_are_undefined() {
    // Reading ZF before any flag-writing instruction: undefined.
    let s = eval_program(&parse_program("sete %rcx").unwrap(), [7u64; 16]);
    assert!(!s.def, "sete at entry reads undefined ZF");
    assert_eq!(s.regs[1], 0); // canonical value

    // Same for cmov at entry.
    let s = eval_program(&parse_program("cmovgq %rbx, %rcx").unwrap(), [7u64; 16]);
    assert!(!s.def, "cmovg at entry reads undefined SF/OF/ZF");

    // Writing flags first makes the read defined.
    let s = eval_program(&parse_program("cmpq $0, %rax\nsete %rcx").unwrap(), [7u64; 16]);
    assert!(s.def);
    assert_eq!(s.regs[1], 0); // 7 != 0
}

/// The earlier `setb`-at-entry |x| form must now be REJECTED by the
/// verifier: it reads entry CF, which is undefined. The legitimate 3-instr
/// form (`leaq -1; xorq $-1; cmovlq`) sets flags before reading them and
/// stays equivalent.
#[test]
fn setb_at_entry_abs_is_rejected_but_xor_form_stands() {
    let mut z = Z3Session::new().unwrap();
    let spec = Spec {
        program: parse_program(
            "movq %rax, %rcx\nmovq $0, %rdx\nsubq %rax, %rdx\ncmpq $0, %rax\ncmovlq %rdx, %rcx",
        )
        .unwrap(),
        live_outs: vec![isuperoptimizer::instr::Gpr::Rcx],
    };
    // entry-flag-reading form: REJECTED via def_R.
    let bad = parse_program("setb %rcx\nsubq %rax, %rcx\ncmovlq %rax, %rcx").unwrap();
    match verify(&bad, &spec, &mut z).unwrap() {
        Verdict::Counterexample(_) => {} // correctly rejected
        Verdict::Equivalent => panic!("setb-at-entry must be rejected: it reads undefined entry CF"),
    }
    // xorq sets flags before cmovl reads them: still equivalent.
    let good = parse_program(
        "leaq -1(%rax), %rcx\nxorq $0xffffffffffffffff, %rcx\ncmovlq %rax, %rcx",
    )
    .unwrap();
    assert!(matches!(
        verify(&good, &spec, &mut z).unwrap(),
        Verdict::Equivalent
    ));
}

/// The gcc-assembly parser handles the real committed output: every
/// survivor parses, and every excluded function fails with the logged
/// reason.
#[test]
fn gcc_asm_survivors_and_exclusions() {
    let asm = std::fs::read_to_string("fixtures/compiler-core/candidates-O2.s").expect("run from repo root");
    let survivors = isuperoptimizer::compiler_core::parse_survivors(&asm).unwrap();
    assert_eq!(survivors.len(), 11, "11 survivors");
    // every survivor program is non-empty and total
    for (name, prog) in &survivors {
        assert!(!prog.0.is_empty(), "{} has an empty program", name);
    }
    // exclusions empirically confirmed
    for (name, observed) in isuperoptimizer::compiler_core::validate_exclusions(&asm) {
        assert!(
            observed != "MISSING FROM ASSEMBLY" && !observed.starts_with("UNEXPECTEDLY PARSED"),
            "{}: exclusion reason not confirmed ({})",
            name,
            observed
        );
    }
    assert_eq!(isuperoptimizer::compiler_core::CORE_EXCLUDED.len(), 8);
}

/// The parser round-trips the exact gcc encodings found in the committed
/// assembly: one-operand shifts (shift-by-1), unsuffixed cmov with
/// aliases, movabs-style immediates.
#[test]
fn gcc_syntax_forms_parse() {
    assert_eq!(
        parse_program("shrq %rax").unwrap(),
        parse_program("shrq $1, %rax").unwrap(),
        "one-operand shift == shift by 1"
    );
    // unsuffixed cmov, alias condition codes
    let p = parse_program("cmovnb %rbx, %rax").unwrap();
    assert_eq!(p.0.len(), 1);
    let p = parse_program("cmovae %rbx, %rax").unwrap();
    assert_eq!(p, parse_program("cmovnb %rbx, %rax").unwrap());
    // movabsq (gcc emits it for full-width immediates): = movq with a
    // full-width immediate. This is PINNED — the old test
    // used an `unwrap_or_else` fallback that passed even when movabsq
    // did not parse, hiding that the opcode was never in the parser.
    assert_eq!(
        parse_program("movq $-1, %rax").unwrap(),
        parse_program("movabsq $0xffffffffffffffff, %rax").unwrap(),
        "movabsq must parse as movq with a full-width immediate"
    );
    // and only as an immediate form
    assert!(parse_instr("movabsq %rbx, %rax").is_err());
    // parse_gcc_asm accepts `rep ret` as a terminator
    let fs = parse_gcc_asm("f:\n\trep ret\n").unwrap();
    assert_eq!(fs.len(), 1);
    assert!(fs[0].1 .0.is_empty());
}
