//! Division and memory acceptance tests, at CI scale.
//! The full-scale evidence lives in the division/memory stress artifact
//! (bulk/z3/hw/hw-mem modes with the extended instruction set), machine-
//! verified by `recheck --from-log`.

use isuperoptimizer::ast::{BV, Env};
use isuperoptimizer::instr::{parse_program, Instr, Slot};
use isuperoptimizer::state::{eval_program, eval_program_in, MEM_BASE, MEM_BYTES};
use isuperoptimizer::sym::{sym_eval_program, SState};

fn regs(rax: u64, rbx: u64, rdx: u64) -> [u64; 16] {
    let mut r = [0u64; 16];
    r[0] = rax;
    r[2] = rdx;
    r[3] = rbx;
    r
}

#[test]
fn div_basics_and_de() {
    // 100 / 7: quotient 14, remainder 2.
    let s = eval_program(&parse_program("divq %rbx").unwrap(), regs(100, 7, 0));
    assert_eq!(s.regs[0], 14);
    assert_eq!(s.regs[2], 2);
    assert!(s.def);
    // Division by zero: undefined (#DE), canonical 0 values.
    let s = eval_program(&parse_program("divq %rbx").unwrap(), regs(100, 0, 0));
    assert!(!s.def);
    assert_eq!(s.regs[0], 0);
    // Quotient overflow: (2^64 + 5) / 1 does not fit in 64 bits.
    let s = eval_program(&parse_program("divq %rbx").unwrap(), regs(5, 1, 1));
    assert!(!s.def);
    // All four flags are undefined after div.
    let s = eval_program(&parse_program("divq %rbx").unwrap(), regs(100, 7, 0));
    assert!(!s.flags.zf_def && !s.flags.sf_def && !s.flags.cf_def && !s.flags.of_def);
    // Reads of those flags afterwards make the program undefined.
    let s = eval_program(&parse_program("divq %rbx\nsete %rcx").unwrap(), regs(100, 7, 0));
    assert!(!s.def);
}

#[test]
fn idiv_corners() {
    // INT64_MIN / -1: rdx = -1 sign-extends rax = INT64_MIN, so the 128-bit
    // dividend is exactly INT64_MIN; / -1 overflows i64 => #DE.
    let s = eval_program(
        &parse_program("idivq %rbx").unwrap(),
        regs(i64::MIN as u64, (-1i64) as u64, (-1i64) as u64),
    );
    assert!(!s.def);
    // Simple defined case: dividend 7 (rdx = 0), divisor 2 => q=3 r=1.
    let s = eval_program(&parse_program("idivq %rbx").unwrap(), regs(7, 2, 0));
    assert_eq!(s.regs[0], 3);
    assert_eq!(s.regs[2], 1);
    assert!(s.def);
    // rdx = 0 makes the 128-bit dividend 2^64-7 (positive), NOT -7:
    // dividing by -1 gives -(2^64-7), which underflows i64 => #DE.
    let s = eval_program(
        &parse_program("idivq %rbx").unwrap(),
        regs((-7i64) as u64, (-1i64) as u64, 0),
    );
    assert!(!s.def);
    // Correct -7 / 2: rdx = -1 sign-extends, dividend = -7 as 128-bit
    // (pattern 0xFFFF...FFF9 in the low word). q = -3, r = -1 (dividend sign).
    let s = eval_program(
        &parse_program("idivq %rbx").unwrap(),
        regs((-7i64) as u64, 2, (-1i64) as u64),
    );
    let dividend = ((((-1i64) as u64) as u128) << 64) | (((-7i64) as u64) as u128);
    assert_eq!(dividend as i128, -7);
    assert_eq!(s.regs[0], (-3i64) as u64);
    assert_eq!(s.regs[2], (-1i64) as u64);
    assert!(s.def);
}

/// Regression (VERIFICATION.md row 27): i128::MIN / -1 must not panic the
/// evaluator (real Rust division panics even in release; the corner is
/// reachable from random registers — rdx = 0x8000000000000000, rax = 0,
/// divisor = -1) and must classify as #DE in both the concrete evaluator,
/// the fold, and the DivDefined predicate. Also the unsigned twin:
/// 0x8000000000000000_0000000000000000 / 0xFFFFFFFFFFFFFFFF > u64::MAX => #DE.
#[test]
fn idiv_i128_min_corner_is_de_not_panic() {
    // concrete
    let s = eval_program(
        &parse_program("idivq %rbx").unwrap(),
        regs(0, (-1i64) as u64, 0x8000_0000_0000_0000),
    );
    assert!(!s.def, "i128::MIN / -1 does not fit i64: #DE");
    // Unsigned twin is DEFINED: 2^127 = (2^64-1)·2^63 + 2^63, quotient
    // exactly 2^63 fits u64 (an earlier version of this test asserted #DE here; the
    // arithmetic was wrong; the fold is the ground truth).
    let s = eval_program(
        &parse_program("divq %rbx").unwrap(),
        regs(0, (-1i64) as u64, 0x8000_0000_0000_0000),
    );
    assert!(s.def, "2^127 / (2^64-1) = 2^63 fits u64: defined");
    assert_eq!(s.regs[0], 0x8000_0000_0000_0000);
    assert_eq!(s.regs[2], 0x8000_0000_0000_0000, "remainder 2^63");
    // fold (eval via AST with concrete leaves)
    use isuperoptimizer::ast::{Bool, BV, Env};
    let env = Env { regs: [0u64; 16], mem: None, mem_base: 0 };
    let hi = BV::Const(0x8000_0000_0000_0000);
    let lo = BV::Const(0);
    let d = BV::Const(0xFFFF_FFFF_FFFF_FFFF);
    let def = Bool::DivDefined { hi: hi.clone(), lo: lo.clone(), d: d.clone(), signed: true };
    assert!(!def.eval_in(&env), "DivDefined fold: #DE at the i128::MIN corner");
    let q = isuperoptimizer::ast::BV::IDiv64 { hi: Box::new(hi), lo: Box::new(lo), d: Box::new(d), out: isuperoptimizer::ast::DivOut::Quot };
    assert_eq!(q.eval_in(&env), 0, "canonical 0 on the undefined path");
}

#[test]
fn memory_load_store_and_oob() {
    let mut mem = [0u8; MEM_BYTES];
    for (i, b) in mem.iter_mut().enumerate() {
        *b = (i as u64 * 7 + 3) as u8;
    }
    // Load from an in-sandbox address.
    let p = parse_program("movq (%rax), %rcx").unwrap();
    let s = eval_program_in(&p, regs(MEM_BASE + 16, 0, 0), mem, MEM_BASE);
    let expect = u64::from_le_bytes([
        (16 * 7 + 3) as u8, (17 * 7 + 3) as u8, (18 * 7 + 3) as u8, (19 * 7 + 3) as u8,
        (20 * 7 + 3) as u8, (21 * 7 + 3) as u8, (22 * 7 + 3) as u8, (23 * 7 + 3) as u8,
    ]);
    assert_eq!(s.regs[1], expect);
    assert!(s.def);
    // OOB load: undefined, canonical 0.
    let s = eval_program_in(&p, regs(MEM_BASE - 8, 0, 0), mem, MEM_BASE);
    assert!(!s.def);
    assert_eq!(s.regs[1], 0);
    // Store then load back.
    let p = parse_program("movq %rbx, (%rax)\nmovq (%rax), %rcx").unwrap();
    let s = eval_program_in(&p, regs(MEM_BASE + 32, 0xdeadbeefcafebabe, 0), mem, MEM_BASE);
    assert_eq!(s.regs[1], 0xdeadbeefcafebabe);
    assert!(s.def);
    let off = 32;
    assert_eq!(&s.mem[off..off + 8], &0xdeadbeefcafebabeu64.to_le_bytes());
    // OOB store: undefined, dropped.
    let p = parse_program("movq %rbx, (%rax)").unwrap();
    let s = eval_program_in(&p, regs(MEM_BASE + MEM_BYTES as u64 - 4, 1, 0), mem, MEM_BASE);
    assert!(!s.def);
    assert_eq!(s.mem[MEM_BYTES - 8], mem[MEM_BYTES - 8]); // unchanged
}

/// Concrete vs folded-symbolic over random states with memory — the CI-scale
/// version of the bulk differential for the extended subset.
#[test]
fn differential_with_memory() {
    use isuperoptimizer::fuzz::{random_mem_state, random_program_mem};
    use isuperoptimizer::rng::Rng;
    let mut rng = Rng::new(0xFEED_CAFE);
    let mut checked = 0u64;
    for _ in 0..3000 {
        let p = random_program_mem(&mut rng, 6);
        let sym = sym_eval_program(&p, SState::from_vars());
        let has_stores = p
            .0
            .iter()
            .any(|s| matches!(s, Slot::Instr(Instr::Store { .. })));
        for _ in 0..3 {
            let (st, mem, base) = random_mem_state(&mut rng);
            let c = eval_program_in(&p, st, mem, base);
            let env = Env { regs: st, mem: Some(&mem), mem_base: base };
            for r in 0..16 {
                assert_eq!(sym.regs[r].eval_in(&env), c.regs[r], "r{} mismatch in:\n{}", r, p);
            }
            if has_stores {
                for k in 0..(MEM_BYTES / 8) {
                    let w = BV::Load64 {
                        mem: Box::new(sym.mem.clone()),
                        addr: Box::new(BV::Const(base + (k as u64) * 8)),
                    }
                    .eval_in(&env);
                    let off = k * 8;
                    let cw = u64::from_le_bytes(c.mem[off..off + 8].try_into().unwrap());
                    assert_eq!(w, cw, "mem window {} in:\n{}", k, p);
                }
            }
            assert_eq!(sym.def.eval_in(&env), c.def, "def mismatch in:\n{}", p);
            checked += 1;
        }
    }
    eprintln!("division/memory differential: {} program-states, 0 discrepancies", checked);
}

/// The miter handles memory programs: a candidate missing the target's store
/// is rejected; an exact store-then-load pair verifies equivalent.
#[test]
fn verify_with_memory() {
    use isuperoptimizer::instr::Gpr;
    use isuperoptimizer::verify::{verify, Spec, Verdict, PinnedMem};
    use isuperoptimizer::z3proc::Z3Session;

    let mut mem = [0u8; MEM_BYTES];
    for (i, b) in mem.iter_mut().enumerate() {
        *b = (i as u64 * 7 + 3) as u8;
    }
    let pin = PinnedMem { bytes: &mem, base: MEM_BASE };
    let spec = Spec {
        program: parse_program("movq %rbx, 8(%rax)\nmovq 8(%rax), %rcx").unwrap(),
        live_outs: vec![Gpr::Rcx],
    };
    // Same program verifies.
    let mut z = Z3Session::new().unwrap();
    let v = verify(&spec.program.clone(), &spec, &mut z).unwrap();
    assert!(matches!(v, Verdict::Equivalent));
    // A candidate that skips the store is rejected.
    let cand = parse_program("movq 8(%rax), %rcx").unwrap();
    let verdict = isuperoptimizer::verify::verify_in(&cand, &spec, &mut z, Some(&pin)).unwrap();
    match verdict {
        Verdict::Counterexample(tc) => {
            // The witness must be concretely checkable with the pinned memory.
            let t = eval_program_in(&spec.program, tc.inputs, mem, MEM_BASE);
            let c = eval_program_in(&cand, tc.inputs, mem, MEM_BASE);
            assert_ne!(t.regs[1], c.regs[1]);
            // Regression (audit finding): the returned TestCase's outputs must
            // be computed under the SAME pin as the query, so the TestCase is
            // self-consistent for any consumer (ADR-003 contract).
            assert_eq!(tc.outputs[1], t.regs[1], "TestCase outputs must match the pinned execution");
        }
        Verdict::Equivalent => panic!("missing-store candidate wrongly accepted"),
    }
}

/// Regression for the i64::MAX #DE boundary bug (VERIFICATION.md rows 17
/// and 23). The signed #DE predicate's upper bound was strict (`bvslt`), so
/// the DEFINED quotient exactly i64::MAX was classified as undefined, and
/// the miter produced a spurious counterexample (equal outputs) at
/// rax = 0xfffffffffffffffe. This test pins the exact boundary through the
/// fold, through the miter's own formula, and through z3 — and commits the
/// historical buggy predicate text so the narrative is checkable, not
/// narrated.
#[test]
fn idiv_quotient_i64_max_boundary_is_defined() {
    use isuperoptimizer::ast::{bool_and, bool_and_all, bool_not, bool_or, Bool, BV, Env};
    use isuperoptimizer::instr::Gpr;
    use isuperoptimizer::sym::{sym_eval_program, SState};
    use isuperoptimizer::z3proc::{Answer, Z3Session};

    // The exact historical witness: rax = 0xfffffffffffffffe (= -2), rdx = 0.
    let mut inputs = [0u64; 16];
    inputs[0] = 0xfffffffffffffffe;

    // (1) The predicate folds DEFINED at the boundary.
    let def = Bool::DivDefined {
        hi: BV::Const(0),
        lo: BV::Const(0xfffffffffffffffe),
        d: BV::Const(2),
        signed: true,
    };
    assert!(
        def.eval(&[0u64; 16]),
        "q == i64::MAX must be classified defined (inclusive upper bound)"
    );

    // (2) z3 proves the predicate true at these constants (asserting its
    // negation is UNSAT).
    let mut z = Z3Session::new().unwrap();
    let cmds = vec![format!("(assert (not {}))", def.to_smt2())];
    match z.solve(&cmds, None).unwrap() {
        Answer::Unsat => {}
        _ => panic!("z3 must refute ¬DivDefined at the i64::MAX quotient boundary"),
    }

    // (3) The committed buggy predicate text — the strict `bvslt` upper
    // bound as it existed before the fix — is FALSE at the boundary
    // (it asserted q < i64::MAX when q == i64::MAX), which is precisely
    // how the misclassification happened. Committing it makes the bug
    // narrative checkable: the old text refutes, the new text holds.
    let old_predicate_text = "(and (distinct #x0000000000000002 #x0000000000000000) \
         (bvsge (bvsdiv ((_ concat 64 64) #x0000000000000000 #xfffffffffffffffe) \
                        ((_ sign_extend 64) #x0000000000000002)) \
                ((_ sign_extend 64) #x8000000000000000)) \
         (bvslt (bvsdiv ((_ concat 64 64) #x0000000000000000 #xfffffffffffffffe) \
                        ((_ sign_extend 64) #x0000000000000002)) \
                ((_ sign_extend 64) #x7fffffffffffffff)))";
    let cmds = vec![format!("(assert {})", old_predicate_text)];
    match z.solve(&cmds, None).unwrap() {
        Answer::Unsat => {} // the old text is false here: it denied a defined division
        _ => panic!("the historical buggy predicate must be FALSE at the boundary (that was the bug)"),
    }

    // (4) The miter's fails-formula — the exact construction `verify` uses
    // — folds FALSE at the boundary input: the input is a non-witness.
    let target = parse_program(
        "movq %rax, %rdx\nsarq $63, %rdx\nmovq $2, %rcx\nidivq %rcx",
    )
    .unwrap();
    let cand = parse_program("xorq %rdx, %rdx\nmovq $2, %rcx\nidivq %rcx").unwrap();
    let init = SState::from_vars();
    let t = sym_eval_program(&target, init.clone());
    let c = sym_eval_program(&cand, init);
    let outs_eq = bool_and_all(vec![Bool::Eq(
        t.regs[Gpr::Rdx.idx()].clone(),
        c.regs[Gpr::Rdx.idx()].clone(),
    )]);
    let fails = bool_and(
        t.def.clone(),
        bool_or(bool_not(c.def.clone()), bool_not(outs_eq)),
    );
    let env = Env { regs: inputs, mem: None, mem_base: 0 };
    assert!(
        !fails.eval_in(&env),
        "the boundary input must not satisfy the miter's fails-formula (non-witness)"
    );

    // (5) And z3 confirms the constrained query is UNSAT: with the inputs
    // pinned to the boundary values, no violation exists.
    let mut cmds = Vec::new();
    for i in 0..16 {
        cmds.push(format!("(declare-const r{} (_ BitVec 64))", i));
        cmds.push(format!("(assert (= r{} #x{:016x}))", i, inputs[i]));
    }
    cmds.push(format!("(assert {})", fails.to_smt2()));
    match z.solve(&cmds, None).unwrap() {
        Answer::Unsat => {}
        _ => panic!("the pinned-boundary miter query must be UNSAT"),
    }

    // (6) The concrete evaluator agrees: both defined, outputs equal.
    let tc = eval_program(&target, inputs);
    let cc = eval_program(&cand, inputs);
    assert!(tc.def && cc.def);
    assert_eq!(tc.regs[Gpr::Rdx.idx()], 0); // -2 mod 2 = 0
    assert_eq!(cc.regs[Gpr::Rdx.idx()], 0); // (2^64-2) mod 2 = 0
}
/// The miter's memory comparison scope (ADR-008): the comparison window at
/// a store address is exactly that store's 8 written bytes. An extra store
/// is unobservable iff it never overlaps the target's written byte range —
/// with a symbolic target address almost any extra store can overlap some
/// window and is then rightly REJECTED (it really changes a byte the target
/// wrote). To isolate the scope, the target here stores at a *constant*
/// address, so its write set is fixed; the extra store at another constant
/// address is disjoint → unobservable → accepted.
#[test]
fn verify_memory_scope_extra_writes_unobservable() {
    use isuperoptimizer::instr::{Gpr, Instr, MemAddr, Program, Slot};
    use isuperoptimizer::verify::{verify, Spec, Verdict};
    use isuperoptimizer::z3proc::Z3Session;

    let const_addr = |off: i64| MemAddr { base: None, index: None, scale: 1, disp: MEM_BASE as i64 + off };
    let base_prog = |extra: Option<i64>| -> Program {
        let mut slots = vec![
            Slot::Instr(Instr::Store { src: Gpr::Rbx, mem: const_addr(16) }),
            Slot::Instr(Instr::Load { mem: const_addr(16), dst: Gpr::Rcx }),
        ];
        if let Some(off) = extra {
            slots.insert(1, Slot::Instr(Instr::Store { src: Gpr::Rbx, mem: const_addr(off) }));
        }
        Program(slots)
    };
    let spec = Spec { program: base_prog(None), live_outs: vec![Gpr::Rcx] };
    let mut z = Z3Session::new().unwrap();

    // Extra defined store at a disjoint constant address: unobservable.
    let v = verify(&base_prog(Some(64)), &spec, &mut z).unwrap();
    assert!(
        matches!(v, Verdict::Equivalent),
        "extra defined store disjoint from the target's write set must be unobservable"
    );

    // Extra store OVERLAPPING the target's window (same constant address,
    // applied after the target's own store — with the same value, so still
    // equivalent; this pins that overlap alone is not the issue, bytes are).
    let v = verify(&base_prog(Some(16)), &spec, &mut z).unwrap();
    assert!(matches!(v, Verdict::Equivalent));

    // An extra store at an out-of-range constant address is a definedness
    // violation and must be REJECTED — with the free-memory guard refusing
    // to hand out an unreproducible witness.
    let slots = vec![
        Slot::Instr(Instr::Store { src: Gpr::Rbx, mem: const_addr(16) }),
        Slot::Instr(Instr::Store { src: Gpr::Rbx, mem: MemAddr { base: None, index: None, scale: 1, disp: -8 } }),
        Slot::Instr(Instr::Load { mem: const_addr(16), dst: Gpr::Rcx }),
    ];
    let cand = Program(slots);
    match verify(&cand, &spec, &mut z) {
        Err(e) => assert!(
            e.contains("not concretely reproducible"),
            "unpinned memory SAT must be refused, got: {}",
            e
        ),
        Ok(Verdict::Equivalent) => panic!("OOB extra store wrongly accepted"),
        Ok(Verdict::Counterexample(_)) => panic!("unpinned memory counterexample must not be handed out"),
    }
}
