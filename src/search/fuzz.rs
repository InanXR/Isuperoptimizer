//! Random program generation for differential testing, plus the complete
//! single-instruction operand grid for the `verify(p,p)` sweep.

use crate::instr::*;
use crate::rng::Rng;

fn reg(r: &mut Rng) -> Gpr {
    Gpr::ALL[r.below(16) as usize]
}

fn imm(r: &mut Rng) -> u64 {
    match r.below(4) {
        0 => r.interesting_u64(),
        1 => (r.below(33) as i64).wrapping_neg() as u64, // [-32, 0]
        2 => r.below(64),
        _ => r.next(),
    }
}

fn shift_count(r: &mut Rng) -> ShiftCount {
    match r.below(3) {
        0 => ShiftCount::Imm([0u8, 1, 2, 3, 7, 31, 63, 64, 127, 128, 200, 255][r.below(12) as usize]),
        1 => ShiftCount::Imm((r.below(256)) as u8),
        _ => ShiftCount::Cl,
    }
}

fn memaddr(r: &mut Rng) -> MemAddr {
    let base = if r.below(4) > 0 { Some(reg(r)) } else { None };
    let index = if r.below(2) == 0 { Some(reg(r)) } else { None };
    let scale = [1u8, 1, 2, 4, 8][r.below(5) as usize];
    let disp = match r.below(3) {
        0 => 0,
        1 => r.below(64) as i64,
        _ => (r.below(256) as i64) - 128,
    };
    MemAddr { base, index, scale, disp }
}

/// One random instruction, opcode-weighted to cover the whole subset.
/// Register-only: the MCMC chains never propose memory operations (their
/// counterexamples must be concretely re-runnable; see ADR-008), so this
/// generator excludes Load/Store. The differential harness uses
/// `random_instr_mem`, which includes them.
pub fn random_instr(r: &mut Rng) -> Instr {
    match r.below(105) {
        0..=11 => Instr::Mov { src: if r.below(2) == 0 { Operand::Reg(reg(r)) } else { Operand::Imm(imm(r)) }, dst: reg(r) },
        12..=23 => Instr::Add { src: if r.below(2) == 0 { Operand::Reg(reg(r)) } else { Operand::Imm(imm(r)) }, dst: reg(r) },
        24..=33 => Instr::Sub { src: if r.below(2) == 0 { Operand::Reg(reg(r)) } else { Operand::Imm(imm(r)) }, dst: reg(r) },
        34..=39 => Instr::Imul { src: reg(r), dst: reg(r) },
        40..=47 => Instr::And { src: if r.below(2) == 0 { Operand::Reg(reg(r)) } else { Operand::Imm(imm(r)) }, dst: reg(r) },
        48..=55 => Instr::Or { src: if r.below(2) == 0 { Operand::Reg(reg(r)) } else { Operand::Imm(imm(r)) }, dst: reg(r) },
        56..=63 => Instr::Xor { src: if r.below(2) == 0 { Operand::Reg(reg(r)) } else { Operand::Imm(imm(r)) }, dst: reg(r) },
        64..=71 => Instr::Cmp { src: if r.below(2) == 0 { Operand::Reg(reg(r)) } else { Operand::Imm(imm(r)) }, dst: reg(r) },
        72..=77 => Instr::Shl { count: shift_count(r), dst: reg(r) },
        78..=83 => Instr::Shr { count: shift_count(r), dst: reg(r) },
        84..=88 => Instr::Sar { count: shift_count(r), dst: reg(r) },
        89..=93 => Instr::Lea { mem: memaddr(r), dst: reg(r) },
        94..=96 => Instr::Set { cc: Cc::ALL[r.below(10) as usize], dst: reg(r) },
        97..=99 => Instr::Neg { dst: reg(r) },
        100..=102 => Instr::Not { dst: reg(r) },
        _ => Instr::Cmov { cc: Cc::ALL[r.below(10) as usize], src: reg(r), dst: reg(r) },
    }
}

/// Differential-testing generator: the full subset including div/idiv and
/// memory load/store.
pub fn random_instr_mem(r: &mut Rng) -> Instr {
    match r.below(115) {
        0..=104 => random_instr(r),
        105..=108 => Instr::Div { src: reg(r) },
        109..=112 => Instr::IDiv { src: reg(r) },
        113 => Instr::Load { mem: memaddr(r), dst: reg(r) },
        _ => Instr::Store { src: reg(r), mem: memaddr(r) },
    }
}

/// Random program of length 1..=max_len (with occasional UNUSED padding).
pub fn random_program(r: &mut Rng, max_len: usize) -> Program {
    let len = 1 + r.below(max_len as u64) as usize;
    Program((0..len)
        .map(|_| {
            if r.below(20) == 0 {
                Slot::Unused
            } else {
                Slot::Instr(random_instr(r))
            }
        })
        .collect())
}

/// Memory-inclusive random program for the differential harness.
pub fn random_program_mem(r: &mut Rng, max_len: usize) -> Program {
    let len = 1 + r.below(max_len as u64) as usize;
    Program((0..len)
        .map(|_| {
            if r.below(20) == 0 {
                Slot::Unused
            } else {
                Slot::Instr(random_instr_mem(r))
            }
        })
        .collect())
}

/// Random memory contents with a fixed byte formula (shared with the
/// hardware oracle) and, with high probability, some in-sandbox pointer
/// values among the registers so loads/stores exercise defined paths.
pub fn random_mem_state(r: &mut Rng) -> ([u64; 16], [u8; crate::state::MEM_BYTES], u64) {
    let mut regs = [0u64; 16];
    for (i, v) in regs.iter_mut().enumerate() {
        // keep a few registers as in-sandbox pointers
        if i >= 4 && r.below(3) == 0 {
            *v = crate::state::MEM_BASE + r.below(crate::state::MEM_BYTES as u64 - 8);
        } else {
            *v = match r.below(3) {
                0 => r.interesting_u64(),
                _ => r.next(),
            };
        }
    }
    let mut mem = [0u8; crate::state::MEM_BYTES];
    for (i, b) in mem.iter_mut().enumerate() {
        *b = (i as u64 * 7 + 3) as u8;
    }
    (regs, mem, crate::state::MEM_BASE)
}

/// Random input state with interesting-heavy values.
pub fn random_state(r: &mut Rng) -> [u64; 16] {
    let mut s = [0u64; 16];
    for v in s.iter_mut() {
        *v = match r.below(3) {
            0 => r.interesting_u64(),
            _ => r.next(),
        };
    }
    s
}

/// Registers written by a program (stable order, deduped) — used to make
/// `verify(p,p)` self-checks observe every effect the program has. Division
/// writes both RAX and RDX; stores write memory, not registers.
pub fn written_regs(p: &Program) -> Vec<Gpr> {
    let mut seen = [false; 16];
    let mut out = Vec::new();
    let mut note = |g: Option<Gpr>| {
        if let Some(g) = g {
            if !seen[g.idx()] {
                seen[g.idx()] = true;
                out.push(g);
            }
        }
    };
    for slot in &p.0 {
        if let Slot::Instr(i) = slot {
            match i {
                Instr::Mov { dst, .. }
                | Instr::Add { dst, .. }
                | Instr::Sub { dst, .. }
                | Instr::Imul { dst, .. }
                | Instr::And { dst, .. }
                | Instr::Or { dst, .. }
                | Instr::Xor { dst, .. }
                | Instr::Shl { dst, .. }
                | Instr::Shr { dst, .. }
                | Instr::Sar { dst, .. }
                | Instr::Lea { dst, .. }
                | Instr::Set { dst, .. }
                | Instr::Cmov { dst, .. }
                | Instr::Load { dst, .. } => note(Some(*dst)),
                Instr::Div { .. } | Instr::IDiv { .. } => {
                    note(Some(Gpr::Rax));
                    note(Some(Gpr::Rdx));
                }
                Instr::Neg { dst } => note(Some(*dst)),
                Instr::Not { dst } => note(Some(*dst)),
                Instr::Cmp { .. } | Instr::Store { .. } => {}
            }
        }
    }
    out
}

/// The complete single-instruction program grid over a finite operand set —
/// the basis for the "verify(p,p) is UNSAT for every program in the subset"
/// sweep. Covers every opcode × representative operands.
pub fn all_single_instr_programs() -> Vec<Program> {
    let mut out = Vec::new();
    let imms = [0u64, 1, 2, 3, 63, 0xFFFF_FFFF, 0x8000_0000_0000_0000, u64::MAX];

    macro_rules! push {
        ($i:expr) => {
            out.push(Program(vec![Slot::Instr($i)]))
        };
    }

    for &dst in Gpr::ALL.iter() {
        for &src in Gpr::ALL.iter() {
            push!(Instr::Mov { src: Operand::Reg(src), dst });
            push!(Instr::Add { src: Operand::Reg(src), dst });
            push!(Instr::Sub { src: Operand::Reg(src), dst });
            push!(Instr::And { src: Operand::Reg(src), dst });
            push!(Instr::Or { src: Operand::Reg(src), dst });
            push!(Instr::Xor { src: Operand::Reg(src), dst });
            push!(Instr::Cmp { src: Operand::Reg(src), dst });
            push!(Instr::Imul { src, dst });
        }
        for &v in imms.iter() {
            push!(Instr::Mov { src: Operand::Imm(v), dst });
            push!(Instr::Add { src: Operand::Imm(v), dst });
            push!(Instr::Sub { src: Operand::Imm(v), dst });
            push!(Instr::And { src: Operand::Imm(v), dst });
            push!(Instr::Or { src: Operand::Imm(v), dst });
            push!(Instr::Xor { src: Operand::Imm(v), dst });
            push!(Instr::Cmp { src: Operand::Imm(v), dst });
        }
        for count in [ShiftCount::Imm(0), ShiftCount::Imm(1), ShiftCount::Imm(2), ShiftCount::Imm(63), ShiftCount::Imm(64), ShiftCount::Imm(255), ShiftCount::Cl] {
            push!(Instr::Shl { count, dst });
            push!(Instr::Shr { count, dst });
            push!(Instr::Sar { count, dst });
        }
        for mem in [
            MemAddr { base: Some(Gpr::Rsi), index: None, scale: 1, disp: 0 },
            MemAddr { base: Some(Gpr::Rsi), index: Some(Gpr::Rcx), scale: 4, disp: 8 },
            MemAddr { base: Some(Gpr::Rsi), index: Some(Gpr::Rdx), scale: 8, disp: -16 },
            MemAddr { base: None, index: Some(Gpr::Rdx), scale: 8, disp: 3 },
            MemAddr { base: Some(Gpr::Rbp), index: None, scale: 1, disp: -128 },
            MemAddr { base: Some(Gpr::Rdi), index: Some(Gpr::Rax), scale: 2, disp: i64::MIN },
            MemAddr { base: Some(Gpr::R12), index: None, scale: 1, disp: i64::MAX },
        ] {
            push!(Instr::Lea { mem, dst });
        }
        for &cc in Cc::ALL.iter() {
            push!(Instr::Set { cc, dst });
            for &src in [Gpr::Rax, Gpr::Rcx, Gpr::Rdx, Gpr::Rbx].iter() {
                push!(Instr::Cmov { cc, src, dst });
            }
        }
        // division and memory additions
        for &src in Gpr::ALL.iter() {
            push!(Instr::Div { src });
            push!(Instr::IDiv { src });
            push!(Instr::Neg { dst });
            push!(Instr::Not { dst });
        }
        for mem in [
            MemAddr { base: Some(Gpr::Rax), index: None, scale: 1, disp: 0 },
            MemAddr { base: Some(Gpr::Rsi), index: Some(Gpr::Rcx), scale: 4, disp: 8 },
            MemAddr { base: None, index: Some(Gpr::Rdx), scale: 8, disp: 3 },
            MemAddr { base: Some(Gpr::Rbp), index: None, scale: 1, disp: -128 },
        ] {
            push!(Instr::Load { mem, dst });
            push!(Instr::Store { src: Gpr::Rbx, mem });
        }
    }
    out
}
