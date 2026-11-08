//! The brute-force enumerator: exhaustive, cost-ordered search over straight-
//! line programs with an optimality certificate (ADR-006).
//!
//! Universe (per benchmark, logged as `enumeration_universe`):
//!   - exact-length dense programs over a declared instruction alphabet
//!   - backward-liveness pruning: an instruction is only enumerated if it
//!     writes something (register or flags) that is live afterwards
//!
//! Soundness of the pruning for the *optimality* claim: if an equivalent
//! program of length L contains an instruction whose writes are all dead,
//! deleting that instruction yields an equivalent program of length L-1,
//! which is enumerated (recursively). So if every enumerated program of
//! length < K is rejected with a concrete witness, no equivalent program of
//! length < K exists in the universe — pruned or not. Witnesses come from
//! two sources, both concretely checkable:
//!   - testcase-screen rejections: the candidate diverges from the spec on
//!     some logged testcase (cost > 0 ⟹ divergence on a live-out);
//!   - z3 rejections: the solver's model, re-checked concretely.
//! z3 counterexamples are added to the test set (CEGIS refinement), which
//! only strengthens later screening.

use crate::cost::TestCase;
use crate::instr::{parse_program, Cc, Gpr, Instr, MemAddr, Operand, Program, ShiftCount, Slot};
use crate::reportlog::{JVal, ReportLog};
use crate::rng::Rng;
use crate::state::{eval_program, CState};
use crate::verify::{verify, Spec, Verdict};
use crate::z3proc::Z3Session;
use std::cell::Cell;

/// Instruction side-effects, for liveness analysis.
pub struct Effects {
    /// Registers written (0–2: div/idiv write RAX and RDX).
    pub writes_regs: Vec<Gpr>,
    pub writes_flags: bool,
    /// Writes to memory (never liveness-prunable in v0's model: memory
    /// writes are observable side effects).
    pub writes_mem: bool,
    pub reads_regs: Vec<Gpr>,
    pub reads_flags: bool,
}

pub fn instr_effects(i: &Instr) -> Effects {
    let mut reads = Vec::new();
    let mut reads_flags = false;
    let mut writes_flags = false;
    let mut writes_regs: Vec<Gpr> = Vec::new();
    let mut writes_mem = false;
    match i {
        Instr::Mov { src, dst } => {
            writes_regs.push(*dst);
            if let Operand::Reg(r) = src {
                reads.push(*r);
            }
        }
        Instr::Add { src, dst }
        | Instr::Sub { src, dst }
        | Instr::And { src, dst }
        | Instr::Or { src, dst }
        | Instr::Xor { src, dst } => {
            writes_regs.push(*dst);
            writes_flags = true;
            reads.push(*dst);
            if let Operand::Reg(r) = src {
                reads.push(*r);
            }
        }
        Instr::Imul { src, dst } => {
            writes_regs.push(*dst);
            writes_flags = true; // kills prior flags (ZF/SF undefined, CF/OF written)
            reads.push(*dst);
            reads.push(*src);
        }
        Instr::Cmp { src, dst } => {
            writes_flags = true;
            reads.push(*dst);
            if let Operand::Reg(r) = src {
                reads.push(*r);
            }
        }
        Instr::Shl { count, dst } | Instr::Shr { count, dst } | Instr::Sar { count, dst } => {
            writes_regs.push(*dst);
            writes_flags = true;
            reads.push(*dst);
            if *count == ShiftCount::Cl {
                reads.push(Gpr::Rcx);
            }
        }
        Instr::Lea { mem, dst } => {
            writes_regs.push(*dst);
            if let Some(b) = mem.base {
                reads.push(b);
            }
            if let Some(ix) = mem.index {
                reads.push(ix);
            }
        }
        Instr::Set { dst, .. } => {
            writes_regs.push(*dst);
            reads_flags = true;
        }
        Instr::Neg { dst } => {
            writes_regs.push(*dst);
            writes_flags = true;
            reads.push(*dst);
        }
        Instr::Not { dst } => {
            // writes a register, touches NO flags (SDM) — liveness must
            // not credit it as a flag definer.
            writes_regs.push(*dst);
            reads.push(*dst);
        }
        Instr::Cmov { src, dst, .. } => {
            writes_regs.push(*dst);
            reads.push(*src);
            reads.push(*dst); // keeps its own value when the condition fails
            reads_flags = true;
        }
        Instr::Div { src } | Instr::IDiv { src } => {
            writes_regs.push(Gpr::Rax);
            writes_regs.push(Gpr::Rdx);
            writes_flags = true; // kills prior flags (all four undefined after div)
            reads.push(Gpr::Rax);
            reads.push(Gpr::Rdx);
            reads.push(*src);
        }
        Instr::Load { mem, dst } => {
            writes_regs.push(*dst);
            if let Some(b) = mem.base {
                reads.push(b);
            }
            if let Some(ix) = mem.index {
                reads.push(ix);
            }
        }
        Instr::Store { src, mem } => {
            writes_mem = true;
            reads.push(*src);
            if let Some(b) = mem.base {
                reads.push(b);
            }
            if let Some(ix) = mem.index {
                reads.push(ix);
            }
        }
    }
    Effects { writes_regs, writes_flags, writes_mem, reads_regs: reads, reads_flags }
}

/// Backward-liveness state: which registers and whether flags are live at a
/// program point (before the suffix already generated).
#[derive(Clone, Copy)]
struct Live {
    regs: u32,
    flags: bool,
}

impl Live {
    fn initial(live_outs: &[Gpr]) -> Live {
        let mut regs = 0u32;
        for g in live_outs {
            regs |= 1u32 << g.idx();
        }
        Live { regs, flags: false } // flags are never live-outs in v0
    }

    /// Prepend `e`; None if the instruction is dead (all its writes dead).
    fn prepend(live: Live, e: &Effects) -> Option<Live> {
        let writes_live_reg = e.writes_regs.iter().any(|g| live.regs & (1 << g.idx()) != 0);
        if !writes_live_reg && !(e.writes_flags && live.flags) && !e.writes_mem {
            return None;
        }
        let mut regs = live.regs;
        for g in &e.writes_regs {
            regs &= !(1 << g.idx());
        }
        for r in &e.reads_regs {
            regs |= 1 << r.idx();
        }
        let flags = if e.writes_flags { false } else { live.flags || e.reads_flags };
        Some(Live { regs, flags })
    }
}

/// The alphabet of enumerable instructions (the declared universe).
pub fn alphabet(cfg: &EnumConfig) -> Vec<Instr> {
    let mut a = Vec::new();
    for dst in cfg.dst_regs.iter().copied() {
        for src in cfg.src_regs.iter().copied() {
            a.push(Instr::Mov { src: Operand::Reg(src), dst });
            a.push(Instr::Add { src: Operand::Reg(src), dst });
            a.push(Instr::Sub { src: Operand::Reg(src), dst });
            a.push(Instr::And { src: Operand::Reg(src), dst });
            a.push(Instr::Or { src: Operand::Reg(src), dst });
            a.push(Instr::Xor { src: Operand::Reg(src), dst });
            a.push(Instr::Cmp { src: Operand::Reg(src), dst });
            a.push(Instr::Imul { src, dst });
            for &cc in Cc::ALL.iter() {
                a.push(Instr::Set { cc, dst });
                a.push(Instr::Cmov { cc, src, dst });
            }
        }
        // Encodability (ADR-011): ALU-with-immediate forms exist in the
        // alphabet only for imm32-sign-encodable values — real x86 ALU ops
        // take no other immediate. Wide constants remain available on Mov
        // (the movabsq form), so the identity floor holds for specs that
        // materialize constants, exactly as gcc does.
        for v in cfg.imms.iter().copied() {
            a.push(Instr::Mov { src: Operand::Imm(v), dst });
            if crate::instr::imm32_encodable(v) {
                a.push(Instr::Add { src: Operand::Imm(v), dst });
                a.push(Instr::Sub { src: Operand::Imm(v), dst });
                a.push(Instr::And { src: Operand::Imm(v), dst });
                a.push(Instr::Or { src: Operand::Imm(v), dst });
                a.push(Instr::Xor { src: Operand::Imm(v), dst });
                a.push(Instr::Cmp { src: Operand::Imm(v), dst });
            }
        }
        for c in cfg.shift_counts.iter().copied() {
            a.push(Instr::Shl { count: ShiftCount::Imm(c), dst });
            a.push(Instr::Shr { count: ShiftCount::Imm(c), dst });
            a.push(Instr::Sar { count: ShiftCount::Imm(c), dst });
        }
        a.push(Instr::Neg { dst });
        a.push(Instr::Not { dst });
        if cfg.include_cl_shifts {
            a.push(Instr::Shl { count: ShiftCount::Cl, dst });
            a.push(Instr::Shr { count: ShiftCount::Cl, dst });
            a.push(Instr::Sar { count: ShiftCount::Cl, dst });
        }
        for &mem in cfg.lea_forms.iter() {
            a.push(Instr::Lea { mem, dst });
        }
    }
    a
}

pub struct EnumConfig {
    pub dst_regs: Vec<Gpr>,
    pub src_regs: Vec<Gpr>,
    pub imms: Vec<u64>,
    pub shift_counts: Vec<u8>,
    pub lea_forms: Vec<MemAddr>,
    pub include_cl_shifts: bool,
    pub max_length: usize,
    /// Above this many candidates in a length class, screen-rejection
    /// witnesses are summarized by count instead of logged one-by-one.
    pub witness_log_threshold: u64,
    pub initial_testcases: usize,
}

impl EnumConfig {
    pub fn for_spec(name: &str, spec_src: &str, live_outs: &[Gpr], max_length: usize) -> EnumConfig {
        // Source registers are restricted to the spec's inputs: the miter
        // shares all 16 input registers, so reading a register the spec never
        // reads creates input-dependence the equivalence check rejects (the
        // only exceptions are values that cancel — never length-minimal, and
        // the universe is the certificate's declared scope in any case).
        //
        // The spec's OWN instruction forms (immediates, shift counts, lea
        // addressing modes) are always included: the identity program must
        // be enumerable — "no equivalent program of length <= L" must never
        // be an artifact of the spec itself falling outside its own universe
        // (found by running gcc output through the enumerator).
        let _ = name;
        let spec = parse_program(spec_src).expect("spec must parse");
        let mut spec_reads = spec_read_regs(spec_src);
        for &g in live_outs {
            if !spec_reads.contains(&g) {
                spec_reads.push(g);
            }
        }
        // the argument registers must be readable (to copy args around)
        for &a in [Gpr::Rdi, Gpr::Rsi, Gpr::Rdx, Gpr::Rcx].iter() {
            if !spec_reads.contains(&a) {
                spec_reads.push(a);
            }
        }
        // destinations: the ABI-scratch set (SysV caller-clobbered) plus
        // the argument registers. ABI register contract (ADR-011,
        // VERIFICATION.md row 33): a candidate writing RBX/RBP/R12-15
        // would break the caller's callee-saved state and an RSP write
        // breaks `ret` — drop-in replacements may write only
        // caller-clobbered registers. (`for_benchmark` still pins the
        // historical 4-register scratch set including RBX: those
        // artifacts certify in-model benchmark optima, not drop-ins.)
        let mut dst = vec![Gpr::Rax, Gpr::Rcx, Gpr::Rdx];
        for &a in [Gpr::Rdi, Gpr::Rsi].iter() {
            if !dst.contains(&a) {
                dst.push(a);
            }
        }
        // base universe + the spec's own immediates / shift counts / lea forms
        let mut imms = vec![
            0, 1, 2, 3, 4, 5, 7, 8, 63, 64, 255, 0xFFFF_FFFF,
            u64::MAX, (i64::MIN as u64), (i64::MAX as u64),
        ];
        let mut shift_counts = vec![1, 2, 3, 4, 31, 63, 64];
        let mut lea_forms = vec![
            MemAddr { base: Some(Gpr::Rax), index: Some(Gpr::Rbx), scale: 1, disp: 0 },
            MemAddr { base: Some(Gpr::Rax), index: Some(Gpr::Rax), scale: 2, disp: 0 },
            MemAddr { base: Some(Gpr::Rax), index: Some(Gpr::Rax), scale: 4, disp: 0 },
            MemAddr { base: Some(Gpr::Rax), index: Some(Gpr::Rax), scale: 8, disp: 0 },
            MemAddr { base: Some(Gpr::Rax), index: Some(Gpr::Rbx), scale: 2, disp: 0 },
            MemAddr { base: Some(Gpr::Rax), index: Some(Gpr::Rbx), scale: 4, disp: 0 },
            MemAddr { base: Some(Gpr::Rax), index: None, scale: 1, disp: 0 },
            MemAddr { base: Some(Gpr::Rbx), index: Some(Gpr::Rax), scale: 1, disp: 0 },
        ];
        for slot in &spec.0 {
            if let Slot::Instr(i) = slot {
                match i {
                    Instr::Mov { src: Operand::Imm(v), .. }
                    | Instr::Add { src: Operand::Imm(v), .. }
                    | Instr::Sub { src: Operand::Imm(v), .. }
                    | Instr::And { src: Operand::Imm(v), .. }
                    | Instr::Or { src: Operand::Imm(v), .. }
                    | Instr::Xor { src: Operand::Imm(v), .. }
                    | Instr::Cmp { src: Operand::Imm(v), .. } => {
                        if !imms.contains(v) {
                            imms.push(*v);
                        }
                    }
                    Instr::Shl { count: ShiftCount::Imm(c), .. }
                    | Instr::Shr { count: ShiftCount::Imm(c), .. }
                    | Instr::Sar { count: ShiftCount::Imm(c), .. } => {
                        if !shift_counts.contains(c) {
                            shift_counts.push(*c);
                        }
                    }
                    Instr::Lea { mem, .. } => {
                        if !lea_forms.contains(mem) {
                            lea_forms.push(*mem);
                        }
                    }
                    _ => {}
                }
            }
        }
        // identity floor (registers): every register the spec WRITES must
        // also be a legal destination, so the spec itself is always in its
        // own universe. gcc specs write only ABI-scratch, but e.g. p19's
        // form uses %r8 — this keeps the floor total for registers too,
        // matching what it already does for immediates/shifts/lea forms.
        for slot in &spec.0 {
            if let Slot::Instr(i) = slot {
                for r in instr_effects(i).writes_regs {
                    if !dst.contains(&r) {
                        dst.push(r);
                    }
                }
            }
        }
        EnumConfig {
            dst_regs: dst,
            src_regs: spec_reads,
            imms,
            shift_counts,
            lea_forms,
            include_cl_shifts: true,
            max_length,
            witness_log_threshold: 2000,
            initial_testcases: 8,
        }
    }

    pub fn for_benchmark(b: &crate::benchmarks::Benchmark) -> EnumConfig {
        let mut cfg = Self::for_spec(
            b.name,
            b.spec,
            b.live_outs,
            match b.strategy {
                Strategy::Enumerate { max_length } => max_length,
                _ => 0,
            },
        );
        // the enumeration suite's universes predate for_spec's ABI widening: restore the
        // original 4-register scratch set and spec-read-only sources so the
        // committed enumeration artifact keeps reproducing exactly (the spec's own
        // imms/shifts/lea forms were already in the base set for all six
        // benchmarks, so nothing else differs).
        cfg.dst_regs = vec![Gpr::Rax, Gpr::Rbx, Gpr::Rcx, Gpr::Rdx];
        cfg.src_regs = spec_read_regs(b.spec);
        cfg
    }
}

use crate::benchmarks::Strategy;

fn spec_read_regs(spec_src: &str) -> Vec<Gpr> {
    let p = parse_program(spec_src).expect("benchmark spec must parse");
    let mut regs = Vec::new();
    let mut seen = [false; 16];
    for slot in &p.0 {
        if let Slot::Instr(i) = slot {
            for r in instr_effects(i).reads_regs {
                if !seen[r.idx()] {
                    seen[r.idx()] = true;
                    regs.push(r);
                }
            }
        }
    }
    regs
}

pub struct EnumOutcome {
    pub optimum: Option<Program>,
    pub optimal_length: usize,
    pub spec_length: usize,
    pub candidates_total: u64,
    pub z3_queries: u64,
    pub witnesses_logged: u64,
    pub testcases: Vec<TestCase>,
}

/// Early-abort screen: does `prog` match the spec on every testcase (cost 0)?
/// Equivalent to `cost_on_testcases(..) == 0` but exits at the first
/// divergence (each testcase's min-distance term is 0 only on exact matches
/// of every live-out).
pub fn screen_zero_cost(prog: &Program, cases: &[TestCase]) -> bool {
    for t in cases {
        let s = eval_program(prog, t.inputs);
        for &g in &t.live_outs {
            if s.regs[g.idx()] != t.outputs[g.idx()] {
                return false;
            }
        }
    }
    true
}

/// The witness for a screen rejection: the first testcase on which the
/// candidate diverges from the spec on a live-out.
pub fn screen_witness<'t>(
    prog: &Program,
    cases: &'t [TestCase],
) -> Option<(usize, CState)> {
    for (i, t) in cases.iter().enumerate() {
        let s = eval_program(prog, t.inputs);
        for &g in &t.live_outs {
            if s.regs[g.idx()] != t.outputs[g.idx()] {
                return Some((i, s));
            }
        }
    }
    None
}

/// Run the exhaustive enumeration for one benchmark. `log` receives every
/// certificate event. Returns the outcome; the caller logs the summary.
pub fn enumerate_benchmark(
    bench: &crate::benchmarks::AnyBenchmark<'_>,
    cfg: &EnumConfig,
    z3: &mut Z3Session,
    log: &mut ReportLog,
    seed: u64,
) -> Result<EnumOutcome, String> {
    let spec = Spec {
        program: parse_program(bench.spec).map_err(|e| e.to_string())?,
        live_outs: bench.live_outs.to_vec(),
    };
    let spec_len = spec.program.0.len();

    let alpha = alphabet(cfg);
    let effects: Vec<Effects> = alpha.iter().map(instr_effects).collect();
    let alpha_display_len = alpha.len();

    log.event(&[
        ("kind", JVal::S("bench_start".into())),
        ("benchmark", JVal::S(bench.name.into())),
        ("spec", JVal::S(bench.spec.into())),
        ("desc", JVal::S(bench.desc.into())),
        ("live_outs", JVal::ArrS(bench.live_outs.iter().map(|g| g.name().to_string()).collect())),
        ("strategy", JVal::S("enumerate".into())),
    ]);
    log.event(&[
        ("kind", JVal::S("enumeration_universe".into())),
        ("benchmark", JVal::S(bench.name.into())),
        ("max_length", JVal::U(cfg.max_length as u64)),
        ("alphabet_size", JVal::U(alpha_display_len as u64)),
        ("dst_regs", JVal::ArrS(cfg.dst_regs.iter().map(|g| g.name().to_string()).collect())),
        ("src_regs", JVal::ArrS(cfg.src_regs.iter().map(|g| g.name().to_string()).collect())),
        ("imms", JVal::ArrS(cfg.imms.iter().map(|v| format!("{:#x}", v)).collect())),
        ("shift_counts", JVal::ArrS(cfg.shift_counts.iter().map(|v| v.to_string()).collect())),
        ("include_cl_shifts", JVal::B(cfg.include_cl_shifts)),
        ("lea_forms", JVal::ArrS(cfg.lea_forms.iter().map(|m| m.to_string()).collect())),
        ("witness_log_threshold", JVal::U(cfg.witness_log_threshold)),
        (
            "pruning",
            JVal::S("backward liveness: an instruction must write a register or flags that are live afterwards; sound because deleting a dead-write instruction preserves observable behavior (ADR-006)".into()),
        ),
    ]);

    // Seed testcases.
    let mut rng = Rng::new(seed);
    let mut cases: Vec<TestCase> = Vec::new();
    for i in 0..cfg.initial_testcases {
        let mut inputs = crate::fuzz::random_state(&mut rng);
        // Guarantee the interesting corners appear in the test set.
        if i == 0 {
            inputs[0] = 0;
            inputs[1] = 0;
            inputs[3] = 0;
        } else if i == 1 {
            inputs[0] = u64::MAX;
            inputs[3] = 1;
        } else if i == 2 {
            inputs[0] = 1 << 63;
            inputs[3] = (1 << 63) | 1;
        } else if i == 3 {
            inputs[0] = 42;
            inputs[3] = 0x8000_0000_0000_0000;
        }
        let tc = TestCase::from_spec_inputs(&spec, inputs);
        log.event(&[
            ("kind", JVal::S("testcase".into())),
            ("benchmark", JVal::S(bench.name.into())),
            ("index", JVal::U(i as u64)),
            ("origin", JVal::S("seed".into())),
            ("inputs", JVal::ArrS(tc.inputs.iter().map(|v| format!("0x{:016x}", v)).collect())),
            ("spec_out", JVal::ArrS(tc.outputs.iter().map(|v| format!("0x{:016x}", v)).collect())),
        ]);
        cases.push(tc);
    }

    let mut z3_queries = 0u64;
    let mut witnesses_logged = 0u64;
    let mut candidates_total = 0u64;
    let mut optimum: Option<Program> = None;
    let stop = Cell::new(false);

    struct PendingWitness {
        candidate: String,
        inputs: [u64; 16],
        spec_out: [u64; 16],
        cand_out: [u64; 16],
        witness_index: usize,
    }

    for length in 1..=cfg.max_length {
        if optimum.is_some() {
            break;
        }
        let mut count = 0u64;
        let mut cost0 = 0u64;
        let mut z3_rejected = 0u64;
        let mut screen_rejected = 0u64;
        // Screen-rejection witnesses are buffered per length class: if the
        // whole class is small (<= threshold) every witness is logged;
        // otherwise the class is summarized by count and the witnesses remain
        // re-derivable from the logged testcases (any rejected candidate
        // diverges on some logged testcase; running it on the committed test
        // set reproduces the divergence).
        let mut pending: Vec<PendingWitness> = Vec::new();
        let mut pending_overflow = false;
        let mut suffix: Vec<Instr> = Vec::with_capacity(length);
        enumerate_dfs(
            &alpha,
            &effects,
            length,
            Live::initial(&spec.live_outs),
            &stop,
            &mut suffix,
            &mut |seq: &[Instr]| {
                count += 1;
                let prog = Program(seq.iter().map(|i| Slot::Instr(*i)).collect());
                if screen_zero_cost(&prog, &cases) {
                    cost0 += 1;
                    z3_queries += 1;
                    match verify(&prog, &spec, z3)? {
                        Verdict::Equivalent => {
                            optimum = Some(prog);
                            stop.set(true);
                        }
                        Verdict::Counterexample(tc) => {
                            z3_rejected += 1;
                            log_rejection_witness(
                                log,
                                bench.name,
                                &prog,
                                &tc.inputs,
                                &tc.outputs,
                                &eval_program(&prog, tc.inputs),
                                "z3",
                            );
                            witnesses_logged += 1;
                            let idx = cases.len() as u64;
                            log.event(&[
                                ("kind", JVal::S("testcase".into())),
                                ("benchmark", JVal::S(bench.name.into())),
                                ("index", JVal::U(idx)),
                                ("origin", JVal::S("z3_counterexample".into())),
                                ("inputs", JVal::ArrS(tc.inputs.iter().map(|v| format!("0x{:016x}", v)).collect())),
                                ("spec_out", JVal::ArrS(tc.outputs.iter().map(|v| format!("0x{:016x}", v)).collect())),
                            ]);
                            cases.push(tc);
                        }
                    }
                } else {
                    screen_rejected += 1;
                    if pending.len() > cfg.witness_log_threshold as usize {
                        pending_overflow = true;
                    } else if let Some((idx, cstate)) = screen_witness(&prog, &cases) {
                        pending.push(PendingWitness {
                            candidate: prog.to_string(),
                            inputs: cases[idx].inputs,
                            spec_out: cases[idx].outputs,
                            cand_out: cstate.regs,
                            witness_index: idx,
                        });
                    }
                }
                Ok::<(), String>(())
            },
        )?;

        if !pending_overflow {
            for w in pending {
                log.event(&[
                    ("kind", JVal::S("rejection_witness".into())),
                    ("benchmark", JVal::S(bench.name.into())),
                    ("origin", JVal::S("screen".into())),
                    ("candidate", JVal::S(w.candidate)),
                    ("witness_index", JVal::U(w.witness_index as u64)),
                    ("inputs", JVal::ArrS(w.inputs.iter().map(|v| format!("0x{:016x}", v)).collect())),
                    ("spec_out", JVal::ArrS(w.spec_out.iter().map(|v| format!("0x{:016x}", v)).collect())),
                    ("cand_out", JVal::ArrS(w.cand_out.iter().map(|v| format!("0x{:016x}", v)).collect())),
                ]);
                witnesses_logged += 1;
            }
        }

        let found = optimum.is_some();
        log.event(&[
            ("kind", JVal::S("enumeration_length".into())),
            ("benchmark", JVal::S(bench.name.into())),
            ("length", JVal::U(length as u64)),
            ("candidates", JVal::U(count)),
            ("cost0", JVal::U(cost0)),
            ("screen_rejected", JVal::U(screen_rejected)),
            ("z3_rejected", JVal::U(z3_rejected)),
            ("equivalent_found", JVal::B(found)),
        ]);
        candidates_total += count;
    }

    let optimal_length = optimum.as_ref().map(|p| p.0.len()).unwrap_or(0);
    if let Some(p) = &optimum {
        log.event(&[
            ("kind", JVal::S("optimum".into())),
            ("benchmark", JVal::S(bench.name.into())),
            ("program", JVal::S(p.to_string())),
            ("length", JVal::U(p.0.len() as u64)),
            ("spec_length", JVal::U(spec_len as u64)),
            ("verdict", JVal::S("equivalent (z3, solver claim)".into())),
        ]);
    }

    Ok(EnumOutcome {
        optimum,
        optimal_length,
        spec_length: spec_len,
        candidates_total,
        z3_queries,
        witnesses_logged,
        testcases: cases,
    })
}

fn log_rejection_witness(
    log: &mut ReportLog,
    bench: &str,
    prog: &Program,
    inputs: &[u64; 16],
    spec_out: &[u64; 16],
    cand_state: &CState,
    origin: &str,
) {
    log.event(&[
        ("kind", JVal::S("rejection_witness".into())),
        ("benchmark", JVal::S(bench.into())),
        ("candidate", JVal::S(prog.to_string())),
        ("origin", JVal::S(origin.into())),
        ("inputs", JVal::ArrS(inputs.iter().map(|v| format!("0x{:016x}", v)).collect())),
        ("spec_out", JVal::ArrS(spec_out.iter().map(|v| format!("0x{:016x}", v)).collect())),
        ("cand_out", JVal::ArrS(cand_state.regs.iter().map(|v| format!("0x{:016x}", v)).collect())),
    ]);
}

/// Depth-first enumeration of all liveness-valid instruction sequences of
/// exactly `len` instructions, built backward (last instruction first).
/// Stops early when `stop` is set (an optimum was found mid-class).
fn enumerate_dfs(
    alpha: &[Instr],
    effects: &[Effects],
    len: usize,
    live: Live,
    stop: &Cell<bool>,
    suffix: &mut Vec<Instr>,
    emit: &mut dyn FnMut(&[Instr]) -> Result<(), String>,
) -> Result<(), String> {
    if stop.get() {
        return Ok(());
    }
    if suffix.len() == len {
        // suffix was built backward; the program runs forward
        let seq: Vec<Instr> = suffix.iter().rev().cloned().collect();
        return emit(&seq);
    }
    for (i, e) in effects.iter().enumerate() {
        if stop.get() {
            return Ok(());
        }
        if let Some(live2) = Live::prepend(live, e) {
            suffix.push(alpha[i]);
            let r = enumerate_dfs(alpha, effects, len, live2, stop, suffix, emit);
            suffix.pop();
            r?;
        }
    }
    Ok(())
}

/// Return the program with every provably-dead instruction removed
/// (replaced by `Unused`). Sound by the same lemma that justifies the
/// enumeration pruning: deleting an instruction whose writes are all dead
/// (registers w.r.t. the live-outs, flags w.r.t. later flag readers)
/// preserves observable behavior. Used by the MCMC optimization phases as a
/// normalization step so that dead-code stripping never costs a solver call.
pub fn strip_dead(prog: &Program, live_outs: &[Gpr]) -> Program {
    let mut live = Live::initial(live_outs);
    let mut keep = vec![false; prog.0.len()];
    for i in (0..prog.0.len()).rev() {
        if let Slot::Instr(instr) = &prog.0[i] {
            let e = instr_effects(instr);
            match Live::prepend(live, &e) {
                None => { /* dead: strip */ }
                Some(l2) => {
                    keep[i] = true;
                    live = l2;
                }
            }
        }
    }
    Program(
        prog.0
            .iter()
            .zip(keep)
            .map(|(s, k)| if k { *s } else { Slot::Unused })
            .collect(),
    )
}
