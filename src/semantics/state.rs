//! Concrete machine state and total concrete semantics for the v0 subset.
//!
//! Every instruction here is a total function on `CState` — no panics, no
//! host-language UB, no silent truncation. Flag semantics follow the Intel
//! SDM (ADR-002). Undefined flags carry `*_def = false` and the canonical
//! value `false`; only flag *reads* (setcc/cmovcc) affect the program
//! definedness `CState.def` (ADR-002/003).

use crate::instr::{Cc, Gpr, Instr, MemAddr, Operand, Program, ShiftCount, Slot};

/// v0 sandbox: a fixed 256-byte memory at a fixed base address (ADR-008).
/// Addresses outside [MEM_BASE, MEM_BASE + 256) are a definedness violation
/// (canonical: OOB loads read 0, OOB stores are dropped, `def` goes false).
pub const MEM_BYTES: usize = 256;
pub const MEM_BASE: u64 = 0x0000_7000_0000_0000;

/// The 16 general-purpose registers, 64-bit only (ADR-001).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MachineState {
    pub regs: [u64; 16],
}

/// Flag values + per-flag definedness.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Flags {
    pub zf: bool,
    pub sf: bool,
    pub cf: bool,
    pub of: bool,
    pub zf_def: bool,
    pub sf_def: bool,
    pub cf_def: bool,
    pub of_def: bool,
}

impl Flags {
    /// Initial flags at function entry (ADR-010): per the SysV AMD64
    /// ABI, incoming RFLAGS are caller-clobbered and effectively undefined —
    /// a callee must not read them before writing. Canonical value false,
    /// ALL UNDEFINED: reading any flag at entry is a definedness violation,
    /// exactly like reading an undefined flag left by IMUL. (Earlier phases
    /// treated entry flags as defined-false, which let candidates like
    /// `sete %rax` at entry produce fake constants — soundness gap found by
    /// compiling real gcc output whose ABI never reads entry flags.)
    pub fn fresh() -> Flags {
        Flags {
            zf: false, sf: false, cf: false, of: false,
            zf_def: false, sf_def: false, cf_def: false, of_def: false,
        }
    }

    /// Flag state after an instruction that writes all four (add/sub/etc.).
    pub fn all_defined(zf: bool, sf: bool, cf: bool, of: bool) -> Flags {
        Flags { zf, sf, cf, of, zf_def: true, sf_def: true, cf_def: true, of_def: true }
    }
}

/// Concrete execution state: registers, flags, the accumulated definedness
/// predicate over consumed flags, and the memory sandbox.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CState {
    pub regs: [u64; 16],
    pub flags: Flags,
    pub def: bool,
    pub mem: [u8; MEM_BYTES],
    pub mem_base: u64,
}

impl CState {
    pub fn from_regs(regs: [u64; 16]) -> CState {
        CState {
            regs,
            flags: Flags::fresh(),
            def: true,
            mem: [0u8; MEM_BYTES],
            mem_base: MEM_BASE,
        }
    }

    pub fn from_regs_mem(regs: [u64; 16], mem: [u8; MEM_BYTES], base: u64) -> CState {
        CState { regs, flags: Flags::fresh(), def: true, mem, mem_base: base }
    }

    #[inline]
    fn addr_in_range(&self, addr: u64) -> bool {
        let off = addr.wrapping_sub(self.mem_base);
        off <= (MEM_BYTES - 8) as u64
    }

    #[inline]
    fn read_mem(&self, addr: u64) -> u64 {
        let off = (addr - self.mem_base) as usize;
        u64::from_le_bytes(self.mem[off..off + 8].try_into().unwrap())
    }

    #[inline]
    fn write_mem(&mut self, addr: u64, v: u64) {
        let off = (addr - self.mem_base) as usize;
        self.mem[off..off + 8].copy_from_slice(&v.to_le_bytes());
    }
}

#[inline]
fn msb(x: u64) -> bool {
    x >> 63 == 1
}

#[inline]
fn val(src: &Operand, s: &CState) -> u64 {
    match src {
        Operand::Reg(r) => s.regs[r.idx()],
        Operand::Imm(v) => *v,
        Operand::Mem(_) => unreachable!("memory operands are rejected at parse time (ADR-004)"),
    }
}

#[inline]
fn shift_count(c: &ShiftCount, s: &CState) -> u64 {
    match c {
        // Hardware masks the count to 6 bits for 64-bit shifts (SDM). Explicit,
        // not incidental. For `%cl` the mask subsumes the byte extraction.
        ShiftCount::Imm(v) => (*v as u64) & 63,
        ShiftCount::Cl => s.regs[Gpr::Rcx.idx()] & 63,
    }
}

/// Flags written by add/sub/cmp (SDM: "set according to the result").
#[inline]
fn flags_add(a: u64, b: u64, r: u64) -> Flags {
    Flags {
        zf: r == 0,
        sf: msb(r),
        cf: r < a, // unsigned carry
        of: msb(a) == msb(b) && msb(r) != msb(a),
        zf_def: true, sf_def: true, cf_def: true, of_def: true,
    }
}

#[inline]
fn flags_sub(a: u64, b: u64, r: u64) -> Flags {
    Flags {
        zf: r == 0,
        sf: msb(r),
        cf: a < b, // unsigned borrow
        of: msb(a) != msb(b) && msb(r) != msb(a),
        zf_def: true, sf_def: true, cf_def: true, of_def: true,
    }
}

#[inline]
fn flags_logic(r: u64) -> Flags {
    Flags {
        zf: r == 0,
        sf: msb(r),
        cf: false,
        of: false,
        zf_def: true, sf_def: true, cf_def: true, of_def: true,
    }
}

/// Condition-code evaluation (SDM SETcc table) — pure function of flags.
#[inline]
pub fn cond(cc: Cc, f: &Flags) -> bool {
    match cc {
        Cc::E => f.zf,
        Cc::Ne => !f.zf,
        Cc::A => !f.cf && !f.zf,
        Cc::Ae => !f.cf,
        Cc::B => f.cf,
        Cc::Be => f.cf || f.zf,
        Cc::G => !f.zf && (f.sf == f.of),
        Cc::Ge => f.sf == f.of,
        Cc::L => f.sf != f.of,
        Cc::Le => f.zf || (f.sf != f.of),
    }
}

/// Which flags a condition code reads is encoded in `defs_of_reads` below;
/// the condition itself is `cond` above.

/// Total concrete step for one instruction.
pub fn eval_instr(i: &Instr, s: &CState) -> CState {
    let mut out = *s;
    match *i {
        Instr::Mov { src, dst } => {
            out.regs[dst.idx()] = val(&src, s);
        }
        Instr::Add { src, dst } => {
            let (a, b) = (s.regs[dst.idx()], val(&src, s));
            let r = a.wrapping_add(b);
            out.regs[dst.idx()] = r;
            out.flags = flags_add(a, b, r);
        }
        Instr::Sub { src, dst } => {
            let (a, b) = (s.regs[dst.idx()], val(&src, s));
            let r = a.wrapping_sub(b);
            out.regs[dst.idx()] = r;
            out.flags = flags_sub(a, b, r);
        }
        Instr::Imul { src, dst } => {
            // Two-operand IMUL: low 64 bits of the *signed* product; CF=OF set
            // iff the signed product differs from the sign-extended truncation;
            // ZF/SF undefined (SDM). NOTE: operands are sign-extended into
            // i128 — zero-extension silently corrupts the overflow flag (caught
            // by the corner test; the symbolic construction was already right).
            let (a, b) = (s.regs[dst.idx()], s.regs[src.idx()]);
            let p = (a as i64 as i128).wrapping_mul(b as i64 as i128);
            let lo = p as u64;
            out.regs[dst.idx()] = lo;
            let ovf = p != (lo as i64) as i128;
            out.flags = Flags {
                zf: false,
                sf: false,
                cf: ovf,
                of: ovf,
                zf_def: false, // undefined per SDM
                sf_def: false,
                cf_def: true,
                of_def: true,
            };
        }
        Instr::And { src, dst } => {
            let r = s.regs[dst.idx()] & val(&src, s);
            out.regs[dst.idx()] = r;
            out.flags = flags_logic(r);
        }
        Instr::Or { src, dst } => {
            let r = s.regs[dst.idx()] | val(&src, s);
            out.regs[dst.idx()] = r;
            out.flags = flags_logic(r);
        }
        Instr::Xor { src, dst } => {
            let r = s.regs[dst.idx()] ^ val(&src, s);
            out.regs[dst.idx()] = r;
            out.flags = flags_logic(r);
        }
        Instr::Cmp { src, dst } => {
            let (a, b) = (s.regs[dst.idx()], val(&src, s));
            out.flags = flags_sub(a, b, a.wrapping_sub(b));
        }
        Instr::Shl { count, dst } => {
            let c = shift_count(&count, s);
            if c == 0 {
                return out; // SDM: "if the count is 0, the flags are not affected"
            }
            let a = s.regs[dst.idx()];
            let r = a << c;
            let cf = (a >> (64 - c)) & 1 == 1; // last bit shifted out
            out.regs[dst.idx()] = r;
            out.flags = Flags {
                zf: r == 0,
                sf: msb(r),
                cf,
                // SDM: OF affected only for 1-bit shifts; SHL: OF = msb(result) XOR CF.
                of: c == 1 && (msb(r) != cf),
                zf_def: true,
                sf_def: true,
                cf_def: true,
                of_def: c == 1,
            };
        }
        Instr::Shr { count, dst } => {
            let c = shift_count(&count, s);
            if c == 0 {
                return out;
            }
            let a = s.regs[dst.idx()];
            let r = a >> c;
            let cf = (a >> (c - 1)) & 1 == 1;
            out.regs[dst.idx()] = r;
            out.flags = Flags {
                zf: r == 0,
                sf: msb(r),
                cf,
                // SHR: OF (1-bit) = msb of the original operand.
                of: c == 1 && msb(a),
                zf_def: true,
                sf_def: true,
                cf_def: true,
                of_def: c == 1,
            };
        }
        Instr::Sar { count, dst } => {
            let c = shift_count(&count, s);
            if c == 0 {
                return out;
            }
            let a = s.regs[dst.idx()];
            let sa = a as i64;
            let r = (sa >> c) as u64;
            let cf = (a >> (c - 1)) & 1 == 1;
            out.regs[dst.idx()] = r;
            out.flags = Flags {
                zf: r == 0,
                sf: msb(r),
                cf,
                // SAR: OF cleared for 1-bit shifts.
                of: false,
                zf_def: true,
                sf_def: true,
                cf_def: true,
                of_def: c == 1,
            };
        }
        Instr::Lea { mem, dst } => {
            out.regs[dst.idx()] = effective_addr(&mem, s);
        }
        Instr::Set { cc, dst } => {
            // v0 divergence (ADR-001): writes the full 64-bit register.
            let c = cond(cc, &s.flags);
            out.regs[dst.idx()] = c as u64;
            out.def &= defs_of_reads(cc, &s.flags);
        }
        Instr::Cmov { cc, src, dst } => {
            if cond(cc, &s.flags) {
                out.regs[dst.idx()] = s.regs[src.idx()];
            }
            out.def &= defs_of_reads(cc, &s.flags);
        }
        Instr::Neg { dst } => {
            // dst := 0 - dst. SDM: CF set iff the operand is nonzero; OF set
            // iff the operand is INT64_MIN (the only negate that overflows);
            // ZF/SF from the result. All four flags defined.
            let old = s.regs[dst.idx()];
            let r = old.wrapping_neg();
            out.regs[dst.idx()] = r;
            out.flags = Flags {
                zf: r == 0,
                sf: msb(r),
                cf: old != 0,
                of: old == (i64::MIN as u64),
                zf_def: true,
                sf_def: true,
                cf_def: true,
                of_def: true,
            };
        }
        Instr::Not { dst } => {
            // dst := !dst. Flags UNAFFECTED per the SDM (NOT, like
            // MOV/LEA, performs no flag updates): both the values and the
            // definedness of all four flags carry through unchanged —
            // `out` starts as a copy of `s`, and this arm never touches
            // `out.flags`.
            out.regs[dst.idx()] = !s.regs[dst.idx()];
        }
        Instr::Div { src } => {
            // Unsigned divide of RDX:RAX by src: quotient -> RAX, remainder
            // -> RDX. #DE (def violation) when the divisor is 0 or the
            // quotient does not fit in 64 bits. Flags: all four undefined.
            let divisor = s.regs[src.idx()];
            let dividend = ((s.regs[Gpr::Rdx.idx()] as u128) << 64) | s.regs[Gpr::Rax.idx()] as u128;
            let ok = divisor != 0 && dividend / divisor as u128 <= u64::MAX as u128;
            if ok {
                out.regs[Gpr::Rax.idx()] = (dividend / divisor as u128) as u64;
                out.regs[Gpr::Rdx.idx()] = (dividend % divisor as u128) as u64;
            } else {
                // canonical values on the undefined path
                out.regs[Gpr::Rax.idx()] = 0;
                out.regs[Gpr::Rdx.idx()] = 0;
                out.def = false;
            }
            out.flags = Flags {
                zf: false, sf: false, cf: false, of: false,
                zf_def: false, sf_def: false, cf_def: false, of_def: false,
            };
        }
        Instr::IDiv { src } => {
            // Signed divide of RDX:RAX by src (remainder takes the
            // dividend's sign). #DE when the divisor is 0 or the signed
            // quotient does not fit in 64 bits (INT64_MIN / -1). Flags: all
            // four undefined.
            let divisor = s.regs[src.idx()] as i64 as i128;
            let dividend = (((s.regs[Gpr::Rdx.idx()] as u128) << 64) | s.regs[Gpr::Rax.idx()] as u128) as i128;
            // wrapping_div: i128::MIN / -1 overflows real division (which
            // panics in Rust even in release); wrapping yields MIN, which the
            // range check then correctly rejects as #DE (VERIFICATION.md row 27).
            let ok = divisor != 0 && {
                let q = dividend.wrapping_div(divisor);
                q >= i64::MIN as i128 && q <= i64::MAX as i128
            };
            if ok {
                out.regs[Gpr::Rax.idx()] = dividend.wrapping_div(divisor) as u64;
                out.regs[Gpr::Rdx.idx()] = dividend.wrapping_rem(divisor) as u64;
            } else {
                out.regs[Gpr::Rax.idx()] = 0;
                out.regs[Gpr::Rdx.idx()] = 0;
                out.def = false;
            }
            out.flags = Flags {
                zf: false, sf: false, cf: false, of: false,
                zf_def: false, sf_def: false, cf_def: false, of_def: false,
            };
        }
        Instr::Load { mem, dst } => {
            let addr = effective_addr(&mem, s);
            if s.addr_in_range(addr) {
                out.regs[dst.idx()] = s.read_mem(addr);
            } else {
                out.regs[dst.idx()] = 0; // canonical value on the undefined path
                out.def = false;
            }
        }
        Instr::Store { src, mem } => {
            let addr = effective_addr(&mem, s);
            if s.addr_in_range(addr) {
                out.write_mem(addr, s.regs[src.idx()]);
            } else {
                out.def = false; // canonical: the store is dropped
            }
        }
    }
    out
}

/// Effective address (mod 2^64), shared by lea/load/store.
#[inline]
pub fn effective_addr(mem: &MemAddr, s: &CState) -> u64 {
    let mut addr: u64 = mem.disp as u64;
    if let Some(b) = mem.base {
        addr = addr.wrapping_add(s.regs[b.idx()]);
    }
    if let Some(ix) = mem.index {
        addr = addr.wrapping_add(s.regs[ix.idx()].wrapping_mul(mem.scale as u64));
    }
    addr
}

/// Definedness of the flags a condition reads.
#[inline]
fn defs_of_reads(cc: Cc, f: &Flags) -> bool {
    match cc {
        Cc::E | Cc::Ne => f.zf_def,
        Cc::A => f.cf_def && f.zf_def,
        Cc::Ae | Cc::B => f.cf_def,
        Cc::Be => f.cf_def && f.zf_def,
        Cc::G => f.zf_def && f.sf_def && f.of_def,
        Cc::Ge | Cc::L => f.sf_def && f.of_def,
        Cc::Le => f.zf_def && f.sf_def && f.of_def,
    }
}

/// Execute a program from given input registers; returns final state
/// (memory sandbox zero-filled at the default base).
pub fn eval_program(p: &Program, inputs: [u64; 16]) -> CState {
    eval_program_in(p, inputs, [0u8; MEM_BYTES], MEM_BASE)
}

/// Execute a program with an explicit initial memory sandbox.
pub fn eval_program_in(
    p: &Program,
    inputs: [u64; 16],
    mem: [u8; MEM_BYTES],
    base: u64,
) -> CState {
    let mut s = CState::from_regs_mem(inputs, mem, base);
    for slot in &p.0 {
        match slot {
            Slot::Instr(i) => s = eval_instr(i, &s),
            Slot::Unused => {}
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instr::parse_program;

    fn prog(src: &str) -> crate::instr::Program {
        parse_program(src).unwrap()
    }

    fn regs(rax: u64, rbx: u64, rcx: u64) -> [u64; 16] {
        let mut r = [0u64; 16];
        r[0] = rax;
        r[1] = rcx;
        r[3] = rbx;
        r
    }

    #[test]
    fn add_flags() {
        // 2^63 + 2^63: carry, overflow, zero, sign
        let s = eval_program(&prog("addq %rbx, %rax"), regs(1 << 63, 1 << 63, 0));
        assert_eq!(s.regs[0], 0);
        let f = s.flags;
        assert!(f.cf, "carry");
        assert!(f.of, "overflow: -2^63 + -2^63 wraps");
        assert!(f.zf);
        assert!(!f.sf);
    }

    #[test]
    fn sub_borrow_and_overflow() {
        // 0 - 1: borrow, result all ones
        let s = eval_program(&prog("subq %rbx, %rax"), regs(0, 1, 0));
        assert_eq!(s.regs[0], u64::MAX);
        assert!(s.flags.cf);
        assert!(!s.flags.of, "0-1 overflows unsigned but not signed");
        // positive minus negative -> wraps negative: overflow
        // MIN - 1 wraps to MAX: signed overflow, no unsigned borrow
        let s = eval_program(&prog("subq %rbx, %rax"), regs(i64::MIN as u64, 1, 0));
        assert!(s.flags.of, "signed overflow: MIN - 1 wraps to MAX");
        assert!(!s.flags.cf, "MIN - 1 does not borrow unsigned");
    }

    #[test]
    fn shift_count_zero_leaves_flags() {
        // set flags with an add into rcx, then shift rax by 0: both the value
        // and the flags must be untouched by the shift.
        let p = prog("addq %rbx, %rcx\nshlq $0, %rax");
        let s = eval_program(&p, regs(1, u64::MAX, 1));
        assert_eq!(s.regs[0], 1, "count 0: value unchanged");
        assert_eq!(s.regs[1], 0, "rcx = 1 + MAX wraps to 0");
        assert!(s.flags.cf, "count 0: CF from the add survives");
        // count 64 (masked to 0) also leaves flags
        let p = prog("addq %rbx, %rcx\nshlq $64, %rax");
        let s = eval_program(&p, regs(1, u64::MAX, 1));
        assert_eq!(s.regs[0], 1);
        assert!(s.flags.cf, "count 64 masks to 0: flags survive");
    }

    #[test]
    fn shift_of_definedness() {
        // shl by 2: OF undefined; setl must make the program undefined.
        let s = eval_program(&prog("shlq $2, %rax"), regs(3, 0, 0));
        assert!(s.def, "writing undefined OF does not make the program undefined");
        assert!(!s.flags.of_def);
        let s = eval_program(&prog("shlq $2, %rax\nsetl %rbx"), regs(3, 0, 0));
        assert!(!s.def, "setl reads SF/OF: OF undefined => program undefined");
        let s = eval_program(&prog("shlq $2, %rax\nsete %rbx"), regs(3, 0, 0));
        assert!(s.def, "sete reads only ZF, which is defined for shifts");
    }

    #[test]
    fn imul_flags_and_definedness() {
        // -1 * -1 = 1: no truncation, CF=OF=0; ZF/SF undefined
        let s = eval_program(&prog("imulq %rbx, %rax"), regs(u64::MAX, u64::MAX, 0));
        assert_eq!(s.regs[0], 1);
        assert!(!s.flags.cf && !s.flags.of);
        assert!(!s.flags.zf_def && !s.flags.sf_def);
        // 2^62 * 4: signed product 2^64 does not fit => CF=OF=1
        let s = eval_program(&prog("imulq %rbx, %rax"), regs(1 << 62, 4, 0));
        assert!(s.flags.cf && s.flags.of);
        // reading ZF after imul makes the program undefined
        let s = eval_program(&prog("imulq %rbx, %rax\nsete %rcx"), regs(1, 2, 0));
        assert!(!s.def);
    }

    #[test]
    fn lea_addressing() {
        let s = eval_program(
            &prog("leaq 8(%rsi,%rcx,4), %r8"),
            { let mut r = regs(0, 0, 3); r[6] = 0x1000; r },
        );
        assert_eq!(s.regs[8], 0x1000 + 3 * 4 + 8);
        let s = eval_program(&prog("leaq -16(,%rdx,8), %r8"), regs(0, 0, 0));
        assert_eq!(s.regs[8], (-16i64) as u64);
    }

    #[test]
    fn cmov_and_setcc() {
        // signed min via cmovg: rax = min(rax, rbx)
        let p = prog("cmpq %rbx, %rax\ncmovgq %rbx, %rax");
        let s = eval_program(&p, regs(5, (-3i64) as u64, 0));
        assert_eq!(s.regs[0], (-3i64) as u64);
        let s = eval_program(&p, regs((-3i64) as u64, 5, 0));
        assert_eq!(s.regs[0], (-3i64) as u64);
        // sete writes full register
        let s = eval_program(&prog("cmpq %rbx, %rax\nsete %r9"), regs(7, 7, 0));
        assert_eq!(s.regs[9], 1);
    }

    #[test]
    fn cl_shift_count() {
        // %cl reads low byte of %rcx; masked to 6 bits
        let s = eval_program(&prog("shrq %cl, %rax"), regs(0b1010_1010, 0, 200));
        assert_eq!(s.regs[0], 0b1010_1010u64 >> (200 & 63));
    }
}
