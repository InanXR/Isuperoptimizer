//! STOKE-style MCMC/simulated-annealing search (ASPLOS'13 §3–4, Fig. 10).
//!
//! Faithful to the published system:
//!   - state: fixed-length programs (ℓ slots) padded with `UNUSED`
//!   - moves and probabilities: opcode  p_c = 0.16, operand p_o = 0.5,
//!     swap p_s = 0.16, instruction p_i = 0.16, UNUSED proposal p_u = 0.16
//!   - immediates drawn from the paper's set: [-16, 16] ∪ {±2^k}
//!     (plus a *logged* SWAR extension — see McmcConfig::immediate_policy)
//!   - acceptance: Metropolis ratio with β = 0.1 (the paper's Eq. 6),
//!     including the §4.5 early-exit optimization: sample p first, compute
//!     the maximum acceptable cost c_cur − ln(p)/β, and stop evaluating
//!     testcases once the running sum exceeds it
//!   - cost: eq'(improved equality metric, w_m = 3, Eq. 15) + perf
//!     (Σ per-opcode latency, the paper's static H(f) model) in the
//!     optimization phase; eq' only in the synthesis phase
//!   - eq* discipline: the solver is consulted only when eq' == 0, with
//!     verdict memoization; failed verifications add counterexamples to the
//!     test set (CEGIS refinement)
//!   - two phases: synthesis (random start, correctness only) and
//!     optimization (correct start, correctness + performance)
//!
//! Documented deviations (ADR-007): the latency table is static rather than
//! learned by regression (the paper learned theirs from hardware counters);
//! per the paper's own Fig. 10 weights we use the Metropolis ratio, and
//! threads run independent chains (no mid-run testcase broadcast — the
//! paper broadcasts its *initial* test set, as do we; refinements are
//! per-chain and all are logged).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::cost::{case_cost, TestCase};
use crate::instr::{
    parse_program, Cc, Gpr, Instr, MemAddr, Operand, Program, ShiftCount, Slot,
};
use crate::rng::Rng;
use crate::state::eval_program;
use crate::verify::{verify, Spec, Verdict};
use crate::z3proc::Z3Session;

// ---------------------------------------------------------------- config

#[derive(Clone, Debug)]
pub struct McmcConfig {
    pub ell: usize,
    pub p_opcode: f64,
    pub p_operand: f64,
    pub p_swap: f64,
    pub p_instruction: f64,
    pub p_unused: f64,
    pub beta: f64,
    pub testcases: usize,
    pub opt_chains: usize,
    pub synth_chains: usize,
    pub opt_seconds: u64,
    pub synth_seconds: u64,
    pub seed: u64,
    /// The paper's immediate set plus a small documented extension the SWAR
    /// benchmarks need (0x55/0x33/0x0F patterns are not powers of two).
    pub immediate_policy: &'static str,
}

impl McmcConfig {
    /// The published hyperparameters (Fig. 10) at their published values;
    /// budgets scaled to this hardware (documented, logged).
    pub fn paper(seed: u64, opt_seconds: u64, synth_seconds: u64) -> McmcConfig {
        McmcConfig {
            ell: 50,
            p_opcode: 0.16,
            p_operand: 0.5,
            p_swap: 0.16,
            p_instruction: 0.16,
            p_unused: 0.16,
            beta: 0.1,
            testcases: 32,
            opt_chains: 4,
            synth_chains: 2,
            opt_seconds,
            synth_seconds,
            seed,
            immediate_policy: "paper set [-16,16] u {+2^k, -2^k} plus documented \
                 SWAR extension {0x55, 0x33, 0x0F, 0x5555555555555555, \
                 0x3333333333333333, 0x0f0f0f0f0f0f0f0f, 0xff}; ALU-slot \
                 immediates restricted to x86-encodable sign-extended imm32 \
                 (ADR-011 encodability pass — full-width constants must \
                 materialize via movq/movabsq, as real gcc output does; \
                 documented deviation from the paper's unrestricted generator)",
        }
    }
}

// ---------------------------------------------------------------- latency

/// Static per-opcode latency estimates (the paper's H(f) = Σ LATENCY(i)
/// model; ours is a static table rather than learned — ADR-007).
pub fn latency(slot: &Slot) -> u64 {
    match slot {
        Slot::Unused => 0,
        Slot::Instr(i) => match i {
            Instr::Mov { .. } => 1,
            Instr::Add { .. } | Instr::Sub { .. } | Instr::And { .. } | Instr::Or { .. }
            | Instr::Xor { .. } | Instr::Cmp { .. } => 1,
            Instr::Imul { .. } => 3,
            Instr::Shl { .. } | Instr::Shr { .. } | Instr::Sar { .. } => 1,
            Instr::Neg { .. } => 1,
            Instr::Not { .. } => 1,
            Instr::Lea { mem, .. } => {
                if mem.index.is_some() {
                    3
                } else {
                    1
                }
            }
            Instr::Set { .. } => 1,
            Instr::Cmov { .. } => 1,
            Instr::Div { .. } | Instr::IDiv { .. } => 26,
            Instr::Load { .. } | Instr::Store { .. } => 1,
        },
    }
}

pub fn program_latency(p: &Program) -> u64 {
    p.0.iter().map(latency).sum()
}

// ---------------------------------------------------------------- moves

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveKind {
    Opcode,
    Operand,
    Swap,
    Instruction,
}

/// Pick a move kind by the published probabilities (exposed for testing the
/// distribution).
pub fn pick_kind(cfg: &McmcConfig, rng: &mut Rng) -> MoveKind {
    // The published probabilities (Fig. 10) sum to 0.98; they are normalized
    // here so the four move kinds keep their published ratios.
    let total = cfg.p_opcode + cfg.p_operand + cfg.p_swap + cfg.p_instruction;
    let u = rng.next_f64();
    if u < cfg.p_opcode / total {
        MoveKind::Opcode
    } else if u < (cfg.p_opcode + cfg.p_operand) / total {
        MoveKind::Operand
    } else if u < (cfg.p_opcode + cfg.p_operand + cfg.p_swap) / total {
        MoveKind::Swap
    } else {
        MoveKind::Instruction
    }
}

/// An ALU-slot immediate: the paper's distribution resampled until the
/// value is x86-encodable (sign-extended imm32). Real ALU instructions
/// have no other immediate form; wide constants must be materialized via
/// movq/movabsq — as gcc does. Documented deviation from the paper's
/// unrestricted generator (ADR-011 encodability pass); the resample loop
/// terminates quickly because most of the paper's distribution is small.
pub fn paper_imm_alu(rng: &mut Rng) -> u64 {
    loop {
        let v = paper_imm(rng);
        if crate::instr::imm32_encodable(v) {
            return v;
        }
    }
}

/// The paper's immediate distribution plus the logged SWAR extension.
pub fn paper_imm(rng: &mut Rng) -> u64 {
    match rng.below(8) {
        0..=3 => {
            // [-16, 16]
            let v = rng.below(33) as i64 - 16;
            v as u64
        }
        4 => 1u64 << rng.below(64),
        5 => (1u64 << rng.below(64)).wrapping_neg(),
        6 => {
            let ext = [
                0x55u64, 0x33, 0x0f, 0x5555_5555_5555_5555, 0x3333_3333_3333_3333,
                0x0f0f_0f0f_0f0f_0f0f, 0xff,
            ];
            ext[rng.below(ext.len() as u64) as usize]
        }
        _ => {
            let ext = [
                0x55u64, 0x33, 0x0f, 0x5555_5555_5555_5555, 0x3333_3333_3333_3333,
                0x0f0f_0f0f_0f0f_0f0f, 0xff,
            ];
            ext[rng.below(ext.len() as u64) as usize].wrapping_neg()
        }
    }
}

/// The ABI-scratch register set (SysV AMD64 caller-clobbered): RAX, RCX,
/// RDX, RSI, RDI, R8-R11. Candidate programs are drop-in replacements for
/// gcc functions: a real function body may clobber only these — callee-
/// saved RBX/RBP/R12-R15 must be preserved for the caller, and RSP must
/// survive for `ret`. Found when a claimed p13_sign winner materialized
/// its constant in %rsp (VERIFICATION.md row 33): in-model correct (all 16
/// GPRs are equal, live-out was RAX only) but fatal on real hardware.
/// Reads of callee-saved registers are excluded too — they are arbitrary
/// inputs the miter rejects as input-dependence anyway.
pub fn random_reg(rng: &mut Rng) -> Gpr {
    const ABI_SCRATCH: [Gpr; 9] = [
        Gpr::Rax, Gpr::Rcx, Gpr::Rdx, Gpr::Rsi, Gpr::Rdi,
        Gpr::R8, Gpr::R9, Gpr::R10, Gpr::R11,
    ];
    ABI_SCRATCH[rng.below(ABI_SCRATCH.len() as u64) as usize]
}

fn random_cc(rng: &mut Rng) -> Cc {
    Cc::ALL[rng.below(10) as usize]
}

fn random_mem(rng: &mut Rng) -> MemAddr {
    let base = if rng.below(4) > 0 { Some(random_reg(rng)) } else { None };
    let mut index = if rng.below(2) == 0 { Some(random_reg(rng)) } else { None };
    let scale = [1u8, 1, 2, 4, 8][rng.below(5) as usize];
    let disp = paper_imm(rng) as i64;
    if base.is_none() && index.is_none() {
        // an addressing mode needs at least a base or an index
        index = Some(random_reg(rng));
    }
    MemAddr { base, index, scale, disp }
}

fn random_instr(rng: &mut Rng) -> Instr {
    match rng.below(16) {
        0 => Instr::Mov { src: Operand::Reg(random_reg(rng)), dst: random_reg(rng) },
        1 => Instr::Mov { src: Operand::Imm(paper_imm(rng)), dst: random_reg(rng) },
        2 => Instr::Add { src: Operand::Reg(random_reg(rng)), dst: random_reg(rng) },
        3 => Instr::Add { src: Operand::Imm(paper_imm_alu(rng)), dst: random_reg(rng) },
        4 => Instr::Sub { src: Operand::Imm(paper_imm_alu(rng)), dst: random_reg(rng) },
        5 => Instr::And { src: Operand::Imm(paper_imm_alu(rng)), dst: random_reg(rng) },
        6 => Instr::Or { src: Operand::Imm(paper_imm_alu(rng)), dst: random_reg(rng) },
        7 => Instr::Xor { src: Operand::Imm(paper_imm_alu(rng)), dst: random_reg(rng) },
        8 => Instr::Cmp { src: Operand::Reg(random_reg(rng)), dst: random_reg(rng) },
        9 => Instr::Imul { src: random_reg(rng), dst: random_reg(rng) },
        10 => {
            let count = if rng.below(2) == 0 {
                ShiftCount::Imm(paper_imm(rng) as u8)
            } else {
                ShiftCount::Cl
            };
            let dst = random_reg(rng);
            match rng.below(3) {
                0 => Instr::Shl { count, dst },
                1 => Instr::Shr { count, dst },
                _ => Instr::Sar { count, dst },
            }
        }
        11 => Instr::Lea { mem: random_mem(rng), dst: random_reg(rng) },
        12 => Instr::Set { cc: random_cc(rng), dst: random_reg(rng) },
        13 => Instr::Neg { dst: random_reg(rng) },
        14 => Instr::Not { dst: random_reg(rng) },
        _ => {
            let cc = random_cc(rng);
            let src = random_reg(rng);
            let dst = random_reg(rng);
            Instr::Cmov { cc, src, dst }
        }
    }
}

/// Apply one proposal to `cur` (returning a new vector). Returns the kind
/// actually applied. Some proposals are no-ops on degenerate draws (e.g.
/// swapping a slot with itself, a singleton opcode class); those are counted
/// as proposals with equal cost (accepted, harmless) — the paper's move
/// probabilities are respected by the *draws*.
pub fn apply_move(cur: &Program, kind: MoveKind, cfg: &McmcConfig, rng: &mut Rng) -> Program {
    let mut out = cur.clone();
    let n = out.0.len();
    match kind {
        MoveKind::Swap => {
            let i = rng.below(n as u64) as usize;
            let j = rng.below(n as u64) as usize;
            out.0.swap(i, j);
        }
        MoveKind::Instruction => {
            let i = rng.below(n as u64) as usize;
            if rng.next_f64() < cfg.p_unused {
                out.0[i] = Slot::Unused;
            } else {
                out.0[i] = Slot::Instr(random_instr(rng));
            }
        }
        MoveKind::Opcode => {
            // pick a slot holding an instruction
            let instr_slots: Vec<usize> = out
                .0
                .iter()
                .enumerate()
                .filter_map(|(i, s)| match s {
                    Slot::Instr(_) => Some(i),
                    _ => None,
                })
                .collect();
            if instr_slots.is_empty() {
                return out;
            }
            let slot = instr_slots[rng.below(instr_slots.len() as u64) as usize];
            if let Slot::Instr(i) = &mut out.0[slot] {
                *i = opcode_mutate(i, rng);
            }
        }
        MoveKind::Operand => {
            let instr_slots: Vec<usize> = out
                .0
                .iter()
                .enumerate()
                .filter_map(|(i, s)| match s {
                    Slot::Instr(_) => Some(i),
                    _ => None,
                })
                .collect();
            if instr_slots.is_empty() {
                return out;
            }
            let slot = instr_slots[rng.below(instr_slots.len() as u64) as usize];
            if let Slot::Instr(i) = &mut out.0[slot] {
                operand_mutate(i, rng);
            }
        }
    }
    out
}

/// Replace an opcode with a random opcode from the same equivalence class
/// (same operand shape), keeping operand values — the paper's opcode move.
fn opcode_mutate(i: &mut Instr, rng: &mut Rng) -> Instr {
    match *i {
        // binops with reg-or-imm source: one class
        Instr::Mov { src, dst } | Instr::Add { src, dst } | Instr::Sub { src, dst }
        | Instr::And { src, dst } | Instr::Or { src, dst } | Instr::Xor { src, dst }
        | Instr::Cmp { src, dst } => {
            let pick = rng.below(7);
            // Encodability carry-fix: an opcode move from Mov into an ALU
            // op must not carry a full-width immediate into a slot where
            // real x86 has only sign-extended imm32 — resample it.
            let src = if pick != 0 {
                match src {
                    Operand::Imm(v) if !crate::instr::imm32_encodable(v) => {
                        Operand::Imm(paper_imm_alu(rng))
                    }
                    s => s,
                }
            } else {
                src
            };
            let mk = |op: usize| match op {
                0 => Instr::Mov { src, dst },
                1 => Instr::Add { src, dst },
                2 => Instr::Sub { src, dst },
                3 => Instr::And { src, dst },
                4 => Instr::Or { src, dst },
                5 => Instr::Xor { src, dst },
                _ => Instr::Cmp { src, dst },
            };
            if pick == 6 && matches!(*i, Instr::Cmp { .. }) {
                *i // degenerate: same opcode
            } else {
                mk(pick as usize)
            }
        }
        Instr::Imul { .. } | Instr::Lea { .. } => *i, // singleton classes
        Instr::Div { .. } | Instr::IDiv { .. } => *i, // singleton classes
        Instr::Load { .. } | Instr::Store { .. } => *i, // singleton classes
        Instr::Neg { .. } => *i, // singleton class
        Instr::Not { .. } => *i, // singleton class
        Instr::Shl { count, dst } => {
            let pick = rng.below(3);
            match pick {
                0 => Instr::Shl { count, dst },
                1 => Instr::Shr { count, dst },
                _ => Instr::Sar { count, dst },
            }
        }
        Instr::Shr { count, dst } => {
            let pick = rng.below(3);
            match pick {
                0 => Instr::Shl { count, dst },
                1 => Instr::Shr { count, dst },
                _ => Instr::Sar { count, dst },
            }
        }
        Instr::Sar { count, dst } => {
            let pick = rng.below(3);
            match pick {
                0 => Instr::Shl { count, dst },
                1 => Instr::Shr { count, dst },
                _ => Instr::Sar { count, dst },
            }
        }
        Instr::Set { dst, .. } => Instr::Set { cc: random_cc(rng), dst },
        Instr::Cmov { src, dst, .. } => {
            Instr::Cmov { cc: random_cc(rng), src, dst }
        }
    }
}

/// Replace one operand with a random same-type operand — the paper's
/// operand move.
fn operand_mutate(i: &mut Instr, rng: &mut Rng) {
    let is_mov = matches!(*i, Instr::Mov { .. });
    match i {
        Instr::Mov { src, dst } | Instr::Add { src, dst } | Instr::Sub { src, dst }
        | Instr::And { src, dst } | Instr::Or { src, dst } | Instr::Xor { src, dst }
        | Instr::Cmp { src, dst } => {
            if rng.below(2) == 0 {
                *src = match src {
                    Operand::Reg(_) => Operand::Reg(random_reg(rng)),
                    // encodability: Mov keeps the full immediate range
                    // (movabsq); ALU slots only get imm32-encodable values.
                    Operand::Imm(_) => {
                        let v = if is_mov { paper_imm(rng) } else { paper_imm_alu(rng) };
                        Operand::Imm(v)
                    }
                    Operand::Mem(_) => unreachable!(),
                };
            } else {
                *dst = random_reg(rng);
            }
        }
        Instr::Imul { src, dst } => {
            if rng.below(2) == 0 {
                *src = random_reg(rng);
            } else {
                *dst = random_reg(rng);
            }
        }
        Instr::Shl { count, dst } | Instr::Shr { count, dst } | Instr::Sar { count, dst } => {
            if rng.below(2) == 0 {
                *count = ShiftCount::Imm(paper_imm(rng) as u8);
            } else {
                *dst = random_reg(rng);
            }
        }
        Instr::Lea { mem, dst } => {
            match rng.below(4) {
                0 => *dst = random_reg(rng),
                1 => {
                    mem.base = if rng.below(2) == 0 {
                        None
                    } else {
                        Some(random_reg(rng))
                    };
                    if mem.base.is_none() && mem.index.is_none() {
                        mem.base = Some(random_reg(rng));
                    }
                }
                2 => {
                    mem.index = if rng.below(2) == 0 {
                        None
                    } else {
                        Some(random_reg(rng))
                    };
                    if mem.base.is_none() && mem.index.is_none() {
                        mem.base = Some(random_reg(rng));
                    }
                }
                _ => {
                    mem.scale = [1u8, 2, 4, 8][rng.below(4) as usize];
                    mem.disp = paper_imm(rng) as i64;
                }
            }
        }
        Instr::Set { dst, .. } => *dst = random_reg(rng),
        Instr::Neg { dst } => *dst = random_reg(rng),
        Instr::Not { dst } => *dst = random_reg(rng),
        Instr::Cmov { src, dst, .. } => {
            if rng.below(2) == 0 {
                *src = random_reg(rng);
            } else {
                *dst = random_reg(rng);
            }
        }
        Instr::Div { src } | Instr::IDiv { src } => *src = random_reg(rng),
        Instr::Load { mem, dst } => {
            if rng.below(2) == 0 {
                *dst = random_reg(rng);
            } else {
                mem_operand_mutate(mem, rng);
            }
        }
        Instr::Store { src, mem } => {
            if rng.below(2) == 0 {
                *src = random_reg(rng);
            } else {
                mem_operand_mutate(mem, rng);
            }
        }
    }
}

/// Mutate the addressing-mode fields of a memory operand, keeping it valid
/// (at least a base or an index register).
fn mem_operand_mutate(mem: &mut MemAddr, rng: &mut Rng) {
    match rng.below(4) {
        0 => {
            mem.base = if rng.below(2) == 0 { None } else { Some(random_reg(rng)) };
            if mem.base.is_none() && mem.index.is_none() {
                mem.base = Some(random_reg(rng));
            }
        }
        1 => {
            mem.index = if rng.below(2) == 0 { None } else { Some(random_reg(rng)) };
            if mem.base.is_none() && mem.index.is_none() {
                mem.base = Some(random_reg(rng));
            }
        }
        2 => mem.scale = [1u8, 2, 4, 8][rng.below(4) as usize],
        _ => mem.disp = paper_imm(rng) as i64,
    }
}

// ---------------------------------------------------------------- cost

/// eq' with the §4.5 early exit: accumulate per-testcase cost until `bound`
/// is exceeded; None means "guaranteed reject".
pub fn eq_prime_early(prog: &Program, cases: &[TestCase], bound: u64) -> Option<u64> {
    let mut acc = 0u64;
    for t in cases {
        let s = eval_program(prog, t.inputs);
        acc += case_cost(&s.regs, t);
        if acc >= bound {
            return None;
        }
    }
    Some(acc)
}

// ---------------------------------------------------------------- chain

pub struct ChainStats {
    pub proposals: u64,
    pub accepts: u64,
    pub move_opcode: u64,
    pub move_operand: u64,
    pub move_swap: u64,
    pub move_instruction: u64,
    pub z3_queries: u64,
    pub z3_equivalent: u64,
    pub z3_unknown: u64,
    pub cegis_additions: u64,
}

pub struct ChainOutcome {
    pub phase: &'static str,
    pub chain: usize,
    pub best: Option<Program>,
    pub best_perf: u64,
    pub final_eq: u64,
    pub stats: ChainStats,
    /// All testcases the chain used: the first `seed_count` are seeds, the
    /// rest are CEGIS counterexamples (bounded by TESTCASE_LOG_CAP).
    pub all_cases: Vec<TestCase>,
    pub seed_count: usize,
    pub dropped_testcases: u64, // CEGIS additions beyond the logging cap
    pub millis: u64,
}

/// Initial testcases: the paper's 32, corner-seeded.
pub fn seed_testcases(spec: &Spec, n: usize, seed: u64) -> Vec<TestCase> {
    let mut rng = Rng::new(seed);
    let mut cases = Vec::new();
    for i in 0..n {
        let mut inputs = crate::fuzz::random_state(&mut rng);
        if i == 0 {
            inputs[0] = 0;
            inputs[3] = 0;
        } else if i == 1 {
            inputs[0] = u64::MAX;
            inputs[3] = 1;
        } else if i == 2 {
            inputs[0] = 1 << 63;
            inputs[3] = (1 << 63) | 1;
        } else if i == 3 {
            inputs[0] = 42;
            inputs[3] = 0xAAAA_AAAA_AAAA_AAAA;
        } else if i == 4 {
            inputs[0] = 255;
            inputs[3] = 0x5555_5555_5555_5555;
        }
        cases.push(TestCase::from_spec_inputs(spec, inputs));
    }
    cases
}

const TESTCASE_LOG_CAP: usize = 200;

/// One MCMC chain. `phase` is "optimization" (cost = eq' + perf; start from
/// `start`, which is the spec padded with UNUSED in round 1, or a synthesis
/// result in round 2 — the paper's §4.4 pipeline) or "synthesis" (cost =
/// eq' only, random start, stops at the first verified-equivalent rewrite).
pub fn run_chain(
    spec: &Spec,
    cfg: &McmcConfig,
    phase: &'static str,
    chain: usize,
    seed: u64,
    seconds: u64,
    start: Program,
) -> Result<ChainOutcome, String> {
    // Short per-query timeout: `unknown` means "cannot confirm within
    // budget", never "inequivalent" — the candidate is skipped and the chain
    // keeps moving (final results are re-verified long-timeout by the CLI).
    let mut z3 = Z3Session::new_with_timeout(4_000)?;
    let mut rng = Rng::new(seed | 1);
    let mut cases = seed_testcases(spec, cfg.testcases, seed ^ 0x7E57);

    let ell = cfg.ell;
    let mut cur: Program = match phase {
        "synthesis" => Program((0..ell).map(|_| Slot::Instr(random_instr(&mut rng))).collect()),
        _ => {
            let mut v = start.0.clone();
            v.resize(ell, Slot::Unused);
            Program(v)
        }
    };

    let use_perf = phase != "synthesis";
    let mut cur_perf = if use_perf { program_latency(&cur) } else { 0 };
    let mut cur_eq = eq_prime_early(&cur, &cases, u64::MAX).unwrap_or(0);
    let mut cur_cost = cur_perf + cur_eq;

    let mut stats = ChainStats {
        proposals: 0,
        accepts: 0,
        move_opcode: 0,
        move_operand: 0,
        move_swap: 0,
        move_instruction: 0,
        z3_queries: 0,
        z3_equivalent: 0,
        z3_unknown: 0,
        cegis_additions: 0,
    };
    let mut verified_true: HashMap<String, ()> = HashMap::new();
    let mut verified_false: HashMap<String, ()> = HashMap::new();
    let mut unknown: HashMap<String, ()> = HashMap::new();
    let mut best: Option<Program> = None;
    let mut best_perf = u64::MAX;
    let mut added: Vec<TestCase> = Vec::new();
    let mut dropped = 0u64;

    let deadline = Instant::now() + Duration::from_secs(seconds);
    let start = Instant::now();

    // The start program is known correct in the paper's optimization
    // phases (round 1: the spec itself; round 2: a synthesis result, chain-
    // verified), so it seeds `best`. It is NOT known correct in the seeded
    // phase (starts are deliberate structural mutations of the gcc
    // form) or in synthesis (random) — pre-seeding there would report an
    // unverified program as the chain's best, which the final long-timeout
    // verification then refutes (found live by the first straggler
    // run; VERIFICATION.md row 31).
    if matches!(phase, "optimization" | "optimization_from_synthesis") {
        best = Some(cur.clone());
        best_perf = cur_perf;
    }

    while Instant::now() < deadline {
        let kind = pick_kind(cfg, &mut rng);
        let mut new = apply_move(&cur, kind, cfg, &mut rng);
        // Optimization phases normalize away provably-dead code before
        // costing (sound by the dead-code lemma; saves a solver call per
        // noise mutation). Synthesis stays paper-faithful: junk slots are
        // its raw material for building new correct regions.
        if phase != "synthesis" {
            new = crate::enumerator::strip_dead(&new, &spec.live_outs);
        }
        stats.proposals += 1;
        match kind {
            MoveKind::Opcode => stats.move_opcode += 1,
            MoveKind::Operand => stats.move_operand += 1,
            MoveKind::Swap => stats.move_swap += 1,
            MoveKind::Instruction => stats.move_instruction += 1,
        }

        let new_perf = if use_perf { program_latency(&new) } else { 0 };
        // §4.5: sample p first; the proposal is acceptable iff its total
        // cost is below c_cur - ln(p)/beta.
        let p = rng.next_f64().max(1e-300);
        let bound_f = cur_cost as f64 - p.ln() / cfg.beta;
        let bound = if bound_f >= u64::MAX as f64 { u64::MAX } else { bound_f as u64 };

        if new_perf >= bound {
            continue; // reject without touching the testcases
        }
        let eq = match eq_prime_early(&new, &cases, bound - new_perf) {
            None => continue, // early-exit reject
            Some(e) => e,
        };
        let new_cost = new_perf + eq;

        if new_cost >= bound {
            continue;
        }

        // eq* discipline: consult the solver only on zero-cost candidates.
        let mut accepted = true;
        if eq == 0 {
            let key = new.to_string();
            if verified_true.contains_key(&key) {
                stats.z3_equivalent += 1;
                let perf = if use_perf { program_latency(&new) } else { 0 };
                if perf < best_perf || best.is_none() {
                    best = Some(new.clone());
                    best_perf = perf;
                }
                if phase == "synthesis" {
                    break;
                }
            } else if verified_false.contains_key(&key) || unknown.contains_key(&key) {
                if verified_false.contains_key(&key) {
                    accepted = false; // known-bad: reject, no new testcase
                } else {
                    accepted = false; // unprovable within budget: skip
                }
            } else {
                stats.z3_queries += 1;
                match verify(&new, spec, &mut z3) {
                    Ok(Verdict::Equivalent) => {
                        verified_true.insert(key, ());
                        stats.z3_equivalent += 1;
                        let perf = if use_perf { program_latency(&new) } else { 0 };
                        if perf < best_perf || best.is_none() {
                            best = Some(new.clone());
                            best_perf = perf;
                        }
                        if phase == "synthesis" {
                            break;
                        }
                    }
                    Ok(Verdict::Counterexample(tc)) => {
                        verified_false.insert(key, ());
                        // CEGIS refinement; the candidate is rejected.
                        accepted = false;
                        stats.cegis_additions += 1;
                        if added.len() < TESTCASE_LOG_CAP {
                            added.push(tc.clone());
                        } else {
                            dropped += 1;
                        }
                        cases.push(tc);
                        // T changed: the current state's eq' may have grown.
                        cur_eq = eq_prime_early(&cur, &cases, u64::MAX).unwrap_or(0);
                        cur_cost = cur_perf + cur_eq;
                    }
                    Err(_) => {
                        // `unknown` (per-query timeout) or a solver hiccup:
                        // cannot confirm -> skip the candidate. Never treated
                        // as inequivalent; never recorded as equivalent.
                        stats.z3_unknown += 1;
                        unknown.insert(key, ());
                        accepted = false;
                    }
                }
            }
        }

        if accepted {
            cur = new;
            cur_perf = new_perf;
            cur_eq = eq;
            cur_cost = new_cost;
            stats.accepts += 1;
        }
    }

    Ok(ChainOutcome {
        phase,
        chain,
        best,
        best_perf,
        final_eq: cur_eq,
        stats,
        all_cases: cases,
        seed_count: cfg.testcases,
        dropped_testcases: dropped,
        millis: start.elapsed().as_millis() as u64,
    })
}

/// Pad a spec program to `ell` slots (the optimization-phase start).
pub fn pad_spec(spec: &str, ell: usize) -> Result<Program, String> {
    let p = parse_program(spec).map_err(|e| e.to_string())?;
    let mut v = p.0;
    v.resize(ell, Slot::Unused);
    Ok(Program(v))
}

/// Full cost (used by tests): perf + full eq'.
pub fn full_cost(prog: &Program, cases: &[TestCase]) -> u64 {
    program_latency(prog)
        + cases
            .iter()
            .map(|t| case_cost(&eval_program(prog, t.inputs).regs, t))
            .sum::<u64>()
}

// tests live in tests/mcmc.rs
