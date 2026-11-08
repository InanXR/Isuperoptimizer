//! Symbolic evaluator: the concrete semantics of `state.rs` ported
//! case-by-case onto the `ast` bit-vector AST.
//!
//! Derivation discipline (mission requirement): every case here mirrors the
//! corresponding case of `state::eval_instr` with the same structure — same
//! flag formulas, same definedness cases, same masking — with exactly one
//! deliberate difference documented inline: IMUL's 64×64→128 product is
//! computed here via four 32-bit multiplies (the AST has no 128-bit sort),
//! while the concrete evaluator uses native i128. The million-case
//! differential harness is what proves the two agree; a dedicated edge-case
//! unit test pins the corners.
//!
//! Constant inputs make `sym_eval_program` fold to concrete values, which is
//! what lets the differential harness run millions of cases with no solver.

use crate::ast::*;
use crate::instr::{Cc, Gpr, Instr, MemAddr, Operand, Program, ShiftCount, Slot};
use crate::state::{MEM_BASE, MEM_BYTES};

#[derive(Clone, Debug)]
pub struct SFlags {
    pub zf: Bool,
    pub sf: Bool,
    pub cf: Bool,
    pub of: Bool,
    pub zf_def: Bool,
    pub sf_def: Bool,
    pub cf_def: Bool,
    pub of_def: Bool,
}

impl SFlags {
    pub fn fresh() -> SFlags {
        // Entry flags undefined (ADR-010): mirror of state::Flags::fresh().
        SFlags {
            zf: Bool::Const(false),
            sf: Bool::Const(false),
            cf: Bool::Const(false),
            of: Bool::Const(false),
            zf_def: Bool::Const(false),
            sf_def: Bool::Const(false),
            cf_def: Bool::Const(false),
            of_def: Bool::Const(false),
        }
    }

    /// All four flags DEFINED — the spread for instructions that write
    /// ZF/SF/CF/OF (add/sub/logic). Not to be confused with `fresh()`
    /// (entry state), which is all-undefined.
    pub fn defined() -> SFlags {
        SFlags {
            zf: Bool::Const(false),
            sf: Bool::Const(false),
            cf: Bool::Const(false),
            of: Bool::Const(false),
            zf_def: Bool::Const(true),
            sf_def: Bool::Const(true),
            cf_def: Bool::Const(true),
            of_def: Bool::Const(true),
        }
    }
}

#[derive(Clone, Debug)]
pub struct SState {
    pub regs: [BV; 16],
    pub flags: SFlags,
    pub def: Bool,
    /// Symbolic memory: the initial array plus store chains.
    pub mem: MemExpr,
    /// Store addresses (symbolic), in program order — used by the miter to
    /// compare the candidate's memory side effects against the target's
    /// write set (ADR-008).
    pub stores: Vec<BV>,
}

impl SState {
    /// Initial symbolic state: register i is the free variable `Var(i)`.
    pub fn from_vars() -> SState {
        SState {
            regs: std::array::from_fn(BV::Var),
            flags: SFlags::fresh(),
            def: Bool::Const(true),
            mem: MemExpr::Base { name: "mem".to_string() },
            stores: Vec::new(),
        }
    }
}

fn val(src: &Operand, s: &SState) -> BV {
    match src {
        Operand::Reg(r) => s.regs[r.idx()].clone(),
        Operand::Imm(v) => BV::Const(*v),
        Operand::Mem(_) => unreachable!("memory operands are rejected at parse time (ADR-004)"),
    }
}

/// Symbolic condition for `cc` (mirror of `state::cond`).
fn cond_sym(cc: Cc, f: &SFlags) -> Bool {
    match cc {
        Cc::E => f.zf.clone(),
        Cc::Ne => bool_not(f.zf.clone()),
        Cc::A => bool_and(bool_not(f.cf.clone()), bool_not(f.zf.clone())),
        Cc::Ae => bool_not(f.cf.clone()),
        Cc::B => f.cf.clone(),
        Cc::Be => bool_or(f.cf.clone(), f.zf.clone()),
        Cc::G => bool_and(bool_not(f.zf.clone()), beq(f.sf.clone(), f.of.clone())),
        Cc::Ge => beq(f.sf.clone(), f.of.clone()),
        Cc::L => bool_not(beq(f.sf.clone(), f.of.clone())),
        Cc::Le => bool_or(f.zf.clone(), bool_not(beq(f.sf.clone(), f.of.clone()))),
    }
}

/// Definedness of the flags a condition reads (mirror of `state::defs_of_reads`).
fn defs_of_reads_sym(cc: Cc, f: &SFlags) -> Bool {
    match cc {
        Cc::E | Cc::Ne => f.zf_def.clone(),
        Cc::A => bool_and(f.cf_def.clone(), f.zf_def.clone()),
        Cc::Ae | Cc::B => f.cf_def.clone(),
        Cc::Be => bool_and(f.cf_def.clone(), f.zf_def.clone()),
        Cc::G => bool_and(bool_and(f.zf_def.clone(), f.sf_def.clone()), f.of_def.clone()),
        Cc::Ge | Cc::L => bool_and(f.sf_def.clone(), f.of_def.clone()),
        Cc::Le => bool_and(bool_and(f.zf_def.clone(), f.sf_def.clone()), f.of_def.clone()),
    }
}

fn shift_count_sym(c: &ShiftCount, s: &SState) -> BV {
    // SDM: count masked to 6 bits for 64-bit operands. The mask subsumes the
    // `%cl` byte extraction.
    let raw = match c {
        ShiftCount::Imm(v) => BV::Const(*v as u64),
        ShiftCount::Cl => s.regs[Gpr::Rcx.idx()].clone(),
    };
    bv_and(raw, BV::Const(63))
}

/// Low 64 bits and the CF/OF overflow bit of the signed 64x64 product,
/// built from four 32-bit multiplies (no 128-bit sort in the AST).
///
/// With ah = a >> 32, al = a & 0xffffffff (same for b):
///   lo = t0 + ((t1 + t2) << 32)        t0 = al*bl, t1 = al*bh, t2 = ah*bl
///   hi = t3 + ((t1 + t2) >> 32) + carry(lo)
/// The *signed* high word is hi_s = hi - (msb(a) ? b : 0) - (msb(b) ? a : 0)
/// (mod 2^64), and CF = OF = (hi_s != sext(msb(lo))).
/// SDM: "CF and OF are set when the signed integer value of the intermediate
/// product differs from the sign extended operand-size-truncated product."
pub fn imul_signed_overflow(a: &BV, b: &BV) -> (BV, Bool) {
    let ah = bv_lshr(a.clone(), BV::Const(32));
    let al = bv_and(a.clone(), BV::Const(0xFFFF_FFFF));
    let bh = bv_lshr(b.clone(), BV::Const(32));
    let bl = bv_and(b.clone(), BV::Const(0xFFFF_FFFF));

    let t0 = bv_mul(al.clone(), bl.clone());
    let t1 = bv_mul(al, bh.clone());
    let t2 = bv_mul(ah.clone(), bl);
    let t3 = bv_mul(ah, bh);

    // t1 + t2 can exceed 64 bits (each half-product is < 2^64, their sum can
    // reach 2^65-2): carry_s is that lost high bit, which contributes
    // carry_s << 32 to `hi`. Found by the corner test: for a = b = -1,
    // t1 + t2 = 0x1_FFFFFFFC_00000002 overflows u64.
    let s12 = bv_add(t1.clone(), t2);
    let carry_s = Bool::Ult(s12.clone(), t1);
    let lo = bv_add(t0.clone(), bv_shl(s12.clone(), BV::Const(32)));
    let carry = Bool::Ult(lo.clone(), t0);
    let hi = bv_add(
        bv_add(
            bv_add(t3, bv_lshr(s12, BV::Const(32))),
            ite(carry_s, BV::Const(1u64 << 32), BV::Const(0)),
        ),
        ite(carry, BV::Const(1), BV::Const(0)),
    );

    let sa = msb(a.clone());
    let sb = msb(b.clone());
    let hi_s = bv_sub(
        bv_sub(hi, ite(sa, b.clone(), BV::Const(0))),
        ite(sb, a.clone(), BV::Const(0)),
    );
    let ovf = bool_not(Bool::Eq(hi_s, sext63(lo.clone())));
    (lo, ovf)
}

/// Symbolic step — mirror of `state::eval_instr`.
pub fn sym_eval_instr(i: &Instr, s: &SState) -> SState {
    let mut out = s.clone();
    match *i {
        Instr::Mov { src, dst } => {
            out.regs[dst.idx()] = val(&src, s);
        }
        Instr::Add { src, dst } => {
            let (a, b) = (s.regs[dst.idx()].clone(), val(&src, s));
            let r = bv_add(a.clone(), b.clone());
            out.regs[dst.idx()] = r.clone();
            out.flags = SFlags {
                zf: Bool::Eq(r.clone(), BV::Const(0)),
                sf: msb(r.clone()),
                cf: Bool::Ult(r.clone(), a.clone()),
                of: bool_and(
                    beq(msb(a.clone()), msb(b.clone())),
                    bool_not(beq(msb(r.clone()), msb(a))),
                ),
                ..SFlags::defined()
            };
        }
        Instr::Sub { src, dst } => {
            let (a, b) = (s.regs[dst.idx()].clone(), val(&src, s));
            let r = bv_sub(a.clone(), b.clone());
            out.regs[dst.idx()] = r.clone();
            out.flags = SFlags {
                zf: Bool::Eq(r.clone(), BV::Const(0)),
                sf: msb(r.clone()),
                cf: Bool::Ult(a.clone(), b.clone()),
                of: bool_and(
                    bool_not(beq(msb(a.clone()), msb(b.clone()))),
                    bool_not(beq(msb(r.clone()), msb(a))),
                ),
                ..SFlags::defined()
            };
        }
        Instr::Imul { src, dst } => {
            // See `imul_signed_overflow` for the deliberate structural
            // difference from the concrete case (which uses native i128).
            let (a, b) = (s.regs[dst.idx()].clone(), s.regs[src.idx()].clone());
            let (lo, ovf) = imul_signed_overflow(&a, &b);
            out.regs[dst.idx()] = lo.clone();
            out.flags = SFlags {
                // ZF/SF undefined per SDM: canonical value false, def false.
                zf: Bool::Const(false),
                sf: Bool::Const(false),
                cf: ovf.clone(),
                of: ovf,
                zf_def: Bool::Const(false),
                sf_def: Bool::Const(false),
                cf_def: Bool::Const(true),
                of_def: Bool::Const(true),
            };
        }
        Instr::And { src, dst } => {
            let r = bv_and(s.regs[dst.idx()].clone(), val(&src, s));
            out.regs[dst.idx()] = r.clone();
            out.flags = SFlags {
                zf: Bool::Eq(r.clone(), BV::Const(0)),
                sf: msb(r),
                cf: Bool::Const(false),
                of: Bool::Const(false),
                ..SFlags::defined()
            };
        }
        Instr::Or { src, dst } => {
            let r = bv_or(s.regs[dst.idx()].clone(), val(&src, s));
            out.regs[dst.idx()] = r.clone();
            out.flags = SFlags {
                zf: Bool::Eq(r.clone(), BV::Const(0)),
                sf: msb(r),
                cf: Bool::Const(false),
                of: Bool::Const(false),
                ..SFlags::defined()
            };
        }
        Instr::Xor { src, dst } => {
            let r = bv_xor(s.regs[dst.idx()].clone(), val(&src, s));
            out.regs[dst.idx()] = r.clone();
            out.flags = SFlags {
                zf: Bool::Eq(r.clone(), BV::Const(0)),
                sf: msb(r),
                cf: Bool::Const(false),
                of: Bool::Const(false),
                ..SFlags::defined()
            };
        }
        Instr::Cmp { src, dst } => {
            let (a, b) = (s.regs[dst.idx()].clone(), val(&src, s));
            let r = bv_sub(a.clone(), b.clone());
            out.flags = SFlags {
                zf: Bool::Eq(r.clone(), BV::Const(0)),
                sf: msb(r.clone()),
                cf: Bool::Ult(a.clone(), b.clone()),
                of: bool_and(
                    bool_not(beq(msb(a.clone()), msb(b.clone()))),
                    bool_not(beq(msb(r.clone()), msb(a))),
                ),
                ..SFlags::defined()
            };
        }
        Instr::Shl { count, dst } => {
            let a = s.regs[dst.idx()].clone();
            let c = shift_count_sym(&count, s);
            let is0 = Bool::Eq(c.clone(), BV::Const(0));
            let is1 = Bool::Eq(c.clone(), BV::Const(1));
            let r = bv_shl(a.clone(), c.clone());
            out.regs[dst.idx()] = r.clone();
            // CF = last bit shifted out = bit (64-c) of a; c in [0,63] after mask.
            let cf_new = bit(bv_lshr(a.clone(), bv_sub(BV::Const(64), c.clone())), 0);
            // OF (1-bit shifts, SDM): msb(result) XOR CF — both are Bools here.
            let of_new = ite_b(
                is1.clone(),
                bool_not(beq(msb(r.clone()), cf_new.clone())),
                Bool::Const(false),
            );
            let old = &s.flags;
            out.flags = SFlags {
                zf: ite_b(is0.clone(), old.zf.clone(), Bool::Eq(r.clone(), BV::Const(0))),
                sf: ite_b(is0.clone(), old.sf.clone(), msb(r.clone())),
                cf: ite_b(is0.clone(), old.cf.clone(), cf_new),
                of: ite_b(is0.clone(), old.of.clone(), of_new),
                zf_def: ite_b(is0.clone(), old.zf_def.clone(), Bool::Const(true)),
                sf_def: ite_b(is0.clone(), old.sf_def.clone(), Bool::Const(true)),
                cf_def: ite_b(is0.clone(), old.cf_def.clone(), Bool::Const(true)),
                of_def: ite_b(is0.clone(), old.of_def.clone(), is1),
            };
        }
        Instr::Shr { count, dst } => {
            let a = s.regs[dst.idx()].clone();
            let c = shift_count_sym(&count, s);
            let is0 = Bool::Eq(c.clone(), BV::Const(0));
            let is1 = Bool::Eq(c.clone(), BV::Const(1));
            let r = bv_lshr(a.clone(), c.clone());
            out.regs[dst.idx()] = r.clone();
            let cf_new = bit(bv_lshr(a.clone(), bv_sub(c.clone(), BV::Const(1))), 0);
            let old = &s.flags;
            out.flags = SFlags {
                zf: ite_b(is0.clone(), old.zf.clone(), Bool::Eq(r.clone(), BV::Const(0))),
                sf: ite_b(is0.clone(), old.sf.clone(), msb(r.clone())),
                cf: ite_b(is0.clone(), old.cf.clone(), cf_new),
                // SHR: OF (1-bit) = msb of the original operand.
                of: ite_b(is0.clone(), old.of.clone(), ite_b(is1.clone(), msb(a.clone()), Bool::Const(false))),
                zf_def: ite_b(is0.clone(), old.zf_def.clone(), Bool::Const(true)),
                sf_def: ite_b(is0.clone(), old.sf_def.clone(), Bool::Const(true)),
                cf_def: ite_b(is0.clone(), old.cf_def.clone(), Bool::Const(true)),
                of_def: ite_b(is0.clone(), old.of_def.clone(), is1),
            };
        }
        Instr::Sar { count, dst } => {
            let a = s.regs[dst.idx()].clone();
            let c = shift_count_sym(&count, s);
            let is0 = Bool::Eq(c.clone(), BV::Const(0));
            let is1 = Bool::Eq(c.clone(), BV::Const(1));
            let r = bv_ashr(a.clone(), c.clone());
            out.regs[dst.idx()] = r.clone();
            let cf_new = bit(bv_lshr(a.clone(), bv_sub(c.clone(), BV::Const(1))), 0);
            let old = &s.flags;
            out.flags = SFlags {
                zf: ite_b(is0.clone(), old.zf.clone(), Bool::Eq(r.clone(), BV::Const(0))),
                sf: ite_b(is0.clone(), old.sf.clone(), msb(r.clone())),
                cf: ite_b(is0.clone(), old.cf.clone(), cf_new),
                // SAR: OF cleared for 1-bit shifts.
                of: ite_b(is0.clone(), old.of.clone(), Bool::Const(false)),
                zf_def: ite_b(is0.clone(), old.zf_def.clone(), Bool::Const(true)),
                sf_def: ite_b(is0.clone(), old.sf_def.clone(), Bool::Const(true)),
                cf_def: ite_b(is0.clone(), old.cf_def.clone(), Bool::Const(true)),
                of_def: ite_b(is0.clone(), old.of_def.clone(), is1),
            };
        }
        Instr::Lea { mem, dst } => {
            out.regs[dst.idx()] = addr_expr(&mem, s);
        }
        Instr::Set { cc, dst } => {
            let c = cond_sym(cc, &s.flags);
            out.regs[dst.idx()] = ite(c, BV::Const(1), BV::Const(0));
            out.def = bool_and(s.def.clone(), defs_of_reads_sym(cc, &s.flags));
        }
        Instr::Cmov { cc, src, dst } => {
            let c = cond_sym(cc, &s.flags);
            out.regs[dst.idx()] = ite(c, s.regs[src.idx()].clone(), s.regs[dst.idx()].clone());
            out.def = bool_and(s.def.clone(), defs_of_reads_sym(cc, &s.flags));
        }
        Instr::Neg { dst } => {
            // Mirror of the concrete case (SDM): r = 0 - dst with
            // CF = (dst != 0) and OF = (dst == INT64_MIN).
            let old = s.regs[dst.idx()].clone();
            let r = BV::Neg(Box::new(old.clone()));
            out.regs[dst.idx()] = r.clone();
            out.flags = SFlags {
                zf: Bool::Eq(r.clone(), BV::Const(0)),
                sf: msb(r.clone()),
                cf: bool_not(Bool::Eq(old.clone(), BV::Const(0))),
                of: Bool::Eq(old.clone(), BV::Const(i64::MIN as u64)),
                ..SFlags::defined()
            };
        }
        Instr::Not { dst } => {
            // dst := ~dst, flags untouched (SDM) — the exact mirror of the
            // concrete arm: `out.flags` carries `s.flags` through both as
            // values and as definedness.
            out.regs[dst.idx()] = BV::Not(Box::new(s.regs[dst.idx()].clone()));
        }
        Instr::Div { src } | Instr::IDiv { src } => {
            let signed = matches!(*i, Instr::IDiv { .. });
            let lo = s.regs[Gpr::Rax.idx()].clone();
            let hi = s.regs[Gpr::Rdx.idx()].clone();
            let d = s.regs[src.idx()].clone();
            // The #DE predicate gates the *values* as well as the definedness:
            // on the undefined path both outputs take the canonical 0 (the
            // concrete evaluator does the same), so the differential sees
            // exact agreement.
            let def_ok = Bool::DivDefined { hi: hi.clone(), lo: lo.clone(), d: d.clone(), signed };
            out.regs[Gpr::Rax.idx()] = ite(
                def_ok.clone(),
                BV::UDiv64 {
                    hi: Box::new(hi.clone()),
                    lo: Box::new(lo.clone()),
                    d: Box::new(d.clone()),
                    out: DivOut::Quot,
                }
                .signed(signed),
                BV::Const(0),
            );
            out.regs[Gpr::Rdx.idx()] = ite(
                def_ok.clone(),
                BV::UDiv64 {
                    hi: Box::new(hi.clone()),
                    lo: Box::new(lo.clone()),
                    d: Box::new(d.clone()),
                    out: DivOut::Rem,
                }
                .signed(signed),
                BV::Const(0),
            );
            out.def = bool_and(s.def.clone(), def_ok);
            out.flags = SFlags {
                zf: Bool::Const(false),
                sf: Bool::Const(false),
                cf: Bool::Const(false),
                of: Bool::Const(false),
                zf_def: Bool::Const(false),
                sf_def: Bool::Const(false),
                cf_def: Bool::Const(false),
                of_def: Bool::Const(false),
            };
        }
        Instr::Load { mem, dst } => {
            let addr = addr_expr(&mem, s);
            // In-sandbox check on the effective address (ADR-008); the
            // *value* is gated too, so an OOB load takes the canonical 0
            // exactly like the concrete evaluator (without the gate, a free
            // array read at an unpinned index lets z3 pick arbitrary values
            // on the undefined path — caught by the division/memory z3 sample).
            let ok = Bool::Ule(
                bv_sub(addr.clone(), BV::Const(MEM_BASE)),
                BV::Const(MEM_BYTES as u64 - 8),
            );
            out.regs[dst.idx()] = ite(
                ok.clone(),
                BV::Load64 { mem: Box::new(s.mem.clone()), addr: Box::new(addr) },
                BV::Const(0),
            );
            out.def = bool_and(s.def.clone(), ok);
        }
        Instr::Store { src, mem } => {
            let addr = addr_expr(&mem, s);
            let ok = Bool::Ule(
                bv_sub(addr.clone(), BV::Const(MEM_BASE)),
                BV::Const(MEM_BYTES as u64 - 8),
            );
            let val = s.regs[src.idx()].clone();
            let mut next = s.mem.clone();
            // On the undefined path the concrete evaluator drops the store
            // entirely; here the bytes are redirected to a scratch address
            // just below the sandbox (never readable: loads only observe
            // in-range addresses), so folds agree with the concrete and the
            // def predicate still records the violation.
            let scratch = BV::Const(MEM_BASE.wrapping_sub(1));
            for k in 0..8u64 {
                let real = bv_add(addr.clone(), BV::Const(k));
                let at = BV::Ite {
                    c: Box::new(ok.clone()),
                    t: Box::new(real),
                    e: Box::new(scratch.clone()),
                };
                next = MemExpr::Store {
                    mem: Box::new(next),
                    addr: Box::new(at),
                    byte: Box::new(BV::Extract {
                        hi: (8 * k + 7) as u8,
                        lo: (8 * k) as u8,
                        x: Box::new(val.clone()),
                    }),
                };
            }
            out.mem = next;
            out.stores.push(addr);
            out.def = bool_and(s.def.clone(), ok);
        }
    }
    out
}

/// Effective-address expression (mod 2^64) for lea/load/store.
fn addr_expr(mem: &MemAddr, s: &SState) -> BV {
    let mut addr = BV::Const(mem.disp as u64);
    if let Some(b) = mem.base {
        addr = bv_add(addr, s.regs[b.idx()].clone());
    }
    if let Some(ix) = mem.index {
        addr = bv_add(addr, bv_mul(s.regs[ix.idx()].clone(), BV::Const(mem.scale as u64)));
    }
    addr
}

impl BV {
    /// Switch an unsigned division node to its signed variant (helper for the
    /// div/idiv cases).
    fn signed(self, signed: bool) -> BV {
        if !signed {
            return self;
        }
        match self {
            BV::UDiv64 { hi, lo, d, out } => BV::IDiv64 { hi, lo, d, out },
            other => other,
        }
    }
}

/// Boolean ite helper.
pub fn ite_b(c: Bool, t: Bool, e: Bool) -> Bool {
    Bool::Ite(Box::new(c), Box::new(t), Box::new(e))
}

pub fn sym_eval_program(p: &Program, init: SState) -> SState {
    let mut s = init;
    for slot in &p.0 {
        match slot {
            Slot::Instr(i) => s = sym_eval_instr(i, &s),
            Slot::Unused => {}
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instr::parse_program;
    use crate::state::eval_program;

    /// The SHL OF formula (msb(result) XOR CF) must fold to the concrete value.
    #[test]
    fn shl_of_matches_concrete() {
        for (a, c) in [(3u64, 1u64), (1 << 63, 1), (1 << 62, 1), (u64::MAX, 1), (5, 0), (7, 2)] {
            let src = format!("shlq ${}, %rax", c);
            let p = parse_program(&src).unwrap();
            let mut env = [0u64; 16];
            env[0] = a;
            let concrete = eval_program(&p, env);
            let sym = sym_eval_program(&p, SState::from_vars());
            assert_eq!(sym.regs[0].eval(&env), concrete.regs[0], "shl value");
            assert_eq!(sym.flags.of.eval(&env), concrete.flags.of, "shl OF a={} c={}", a, c);
            assert_eq!(sym.flags.cf.eval(&env), concrete.flags.cf, "shl CF");
            assert_eq!(sym.flags.of_def.eval(&env), concrete.flags.of_def, "shl OF def");
        }
    }

    /// The IMUL 32-bit-split construction must match native i128 semantics
    /// on the corners (the differential harness covers the random middle).
    #[test]
    fn imul_construction_corners() {
        let corners = [
            0u64, 1, 2, u64::MAX, u64::MAX - 1, 1 << 63, (1 << 63) - 1, 1 << 32, (1 << 32) - 1,
            0xdead_beef_cafe_babe, 12345, 0x7FFF_FFFF_FFFF_FFFF,
        ];
        for &a in &corners {
            for &b in &corners {
                let av = BV::Const(a);
                let bv = BV::Const(b);
                let (lo, ovf) = imul_signed_overflow(&av, &bv);
                let env = [0u64; 16];
                // The signed 64x64 product: operands are *sign*-extended.
                let p = (a as i64 as i128).wrapping_mul(b as i64 as i128);
                assert_eq!(lo.eval(&env), p as u64, "imul lo a={} b={}", a, b);
                let expect_ovf = p != ((p as u64) as i64) as i128;
                assert_eq!(ovf.eval(&env), expect_ovf, "imul ovf a={} b={}", a, b);
            }
        }
    }
}
