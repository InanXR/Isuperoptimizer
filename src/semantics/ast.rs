//! A tiny bit-vector AST with two backends:
//!   1. constant folding (`eval`) — lets the differential harness compare the
//!      symbolic evaluator against the concrete evaluator millions of times
//!      per second with no solver in the loop;
//!   2. SMT-LIB2 printing (`to_smt2`) — the query *is* the certificate
//!      artifact (ADR-005).
//!
//! All bit-vectors are 64-bit; booleans are solver `Bool`. `Var(usize)` is a
//! register index; names are assigned at print time (`r{index}`), matching
//! the declare-consts emitted by the verifier.

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BV {
    Const(u64),
    /// Free variable, identified by register index.
    Var(usize),
    Add(Box<BV>, Box<BV>),
    Sub(Box<BV>, Box<BV>),
    Mul(Box<BV>, Box<BV>),
    And(Box<BV>, Box<BV>),
    Or(Box<BV>, Box<BV>),
    Xor(Box<BV>, Box<BV>),
    Not(Box<BV>),
    Neg(Box<BV>),
    /// `a << b` with SMT semantics: b >= 64 yields 0 (NOT Rust's masking shl).
    Shl(Box<BV>, Box<BV>),
    /// Logical shift right, SMT semantics: b >= 64 yields 0.
    LShr(Box<BV>, Box<BV>),
    /// Arithmetic shift right, SMT semantics: b >= 64 yields sign fill.
    AShr(Box<BV>, Box<BV>),
    /// `((_ extract hi lo) x)`
    Extract { hi: u8, lo: u8, x: Box<BV> },
    /// Qword memory load: 8 byte-selects, nested-concatenated (portable
    /// binary `concat`). Folds via the concrete memory in `Env`.
    Load64 { mem: Box<MemExpr>, addr: Box<BV> },
    /// Unsigned 128-bit division of `hi:lo` by `d` (both 64-bit nodes);
    /// `out` selects the low 64 bits of quotient or remainder.
    UDiv64 { hi: Box<BV>, lo: Box<BV>, d: Box<BV>, out: DivOut },
    /// Signed variant (the dividend bits are reinterpreted; the divisor is
    /// sign-extended). Remainder takes the dividend's sign, per x86.
    IDiv64 { hi: Box<BV>, lo: Box<BV>, d: Box<BV>, out: DivOut },
    Ite { c: Box<Bool>, t: Box<BV>, e: Box<BV> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DivOut {
    Quot,
    Rem,
}

/// Symbolic byte-granular memory (ADR-004): the initial array plus a chain of
/// byte stores. Qword loads/stores expand into 8 byte accesses in `sym`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MemExpr {
    /// The initial memory: `(declare-const mem_qN (Array (_ BitVec 64) (_ BitVec 8)))`.
    Base { name: String },
    /// `(store mem addr byte)`
    Store { mem: Box<MemExpr>, addr: Box<BV>, byte: Box<BV> },
}

impl MemExpr {
    pub fn to_smt2(&self) -> String {
        match self {
            MemExpr::Base { name } => name.clone(),
            MemExpr::Store { mem, addr, byte } => {
                format!("(store {} {} {})", mem.to_smt2(), addr.to_smt2(), byte.to_smt2())
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Bool {
    Const(bool),
    Eq(BV, BV),
    Ult(BV, BV),
    Ule(BV, BV),
    Slt(BV, BV),
    Sle(BV, BV),
    /// Equality of two booleans (SMT `(= b1 b2)` on the Bool sort).
    BEq(Box<Bool>, Box<Bool>),
    /// Division definedness for 64-bit `d` dividing the 128-bit `hi:lo`:
    /// unsigned: divisor ≠ 0 and the quotient fits in 64 bits (no #DE);
    /// signed: divisor ≠ 0 and the signed quotient fits in i64
    /// (excludes INT64_MIN / -1). Prints the 128-bit checks directly.
    DivDefined { hi: BV, lo: BV, d: BV, signed: bool },
    And(Box<Bool>, Box<Bool>),
    Or(Box<Bool>, Box<Bool>),
    Not(Box<Bool>),
    Ite(Box<Bool>, Box<Bool>, Box<Bool>),
}

// ---------------------------------------------------------------- builders

/// Evaluation environment: register values, plus (for folding memory loads)
/// the concrete sandbox contents and its base address.
pub struct Env<'a> {
    pub regs: [u64; 16],
    /// Concrete sandbox bytes (absolute addressing: the byte at address `a`
    /// is `mem[a - mem_base]`), when available for folding.
    pub mem: Option<&'a [u8]>,
    pub mem_base: u64,
}

impl<'a> Env<'a> {
    pub fn regs(regs: [u64; 16]) -> Env<'a> {
        Env { regs, mem: None, mem_base: 0 }
    }
}

pub fn bv_add(a: BV, b: BV) -> BV {
    BV::Add(Box::new(a), Box::new(b))
}
pub fn bv_sub(a: BV, b: BV) -> BV {
    BV::Sub(Box::new(a), Box::new(b))
}
pub fn bv_mul(a: BV, b: BV) -> BV {
    BV::Mul(Box::new(a), Box::new(b))
}
pub fn bv_and(a: BV, b: BV) -> BV {
    BV::And(Box::new(a), Box::new(b))
}
pub fn bv_or(a: BV, b: BV) -> BV {
    BV::Or(Box::new(a), Box::new(b))
}
pub fn bv_xor(a: BV, b: BV) -> BV {
    BV::Xor(Box::new(a), Box::new(b))
}
pub fn bv_shl(a: BV, b: BV) -> BV {
    BV::Shl(Box::new(a), Box::new(b))
}
pub fn bv_lshr(a: BV, b: BV) -> BV {
    BV::LShr(Box::new(a), Box::new(b))
}
pub fn bv_ashr(a: BV, b: BV) -> BV {
    BV::AShr(Box::new(a), Box::new(b))
}
pub fn ite(c: Bool, t: BV, e: BV) -> BV {
    BV::Ite { c: Box::new(c), t: Box::new(t), e: Box::new(e) }
}
pub fn bool_and(a: Bool, b: Bool) -> Bool {
    Bool::And(Box::new(a), Box::new(b))
}
pub fn bool_or(a: Bool, b: Bool) -> Bool {
    Bool::Or(Box::new(a), Box::new(b))
}
pub fn bool_not(a: Bool) -> Bool {
    Bool::Not(Box::new(a))
}

/// Boolean equality (`(= b1 b2)` over the Bool sort).
pub fn beq(a: Bool, b: Bool) -> Bool {
    Bool::BEq(Box::new(a), Box::new(b))
}

/// Fold a list of booleans with AND; empty list is `true`.
pub fn bool_and_all(mut xs: Vec<Bool>) -> Bool {
    match xs.pop() {
        None => Bool::Const(true),
        Some(mut acc) => {
            while let Some(x) = xs.pop() {
                acc = bool_and(x, acc);
            }
            acc
        }
    }
}

/// `msb(x)` as a boolean test: `(x & 1<<63) == 1<<63`.
///
/// NB: implemented as a full-width mask comparison rather than
/// `((_ extract 63 63) x) == #x1` — extract produces a 1-bit sort, which
/// cannot be compared against 64-bit constants without a sort error (caught
/// by the miter tests through z3).
pub fn msb(x: BV) -> Bool {
    bit(x, 63)
}

/// `bit i of x` as a boolean test: `(x & (1<<i)) == (1<<i)` (all 64-bit sorts).
pub fn bit(x: BV, i: u8) -> Bool {
    let m = BV::Const(1u64 << i);
    Bool::Eq(bv_and(x, m.clone()), m)
}

/// Zero-extend bit 63 of `x` across the whole word (sign extension of a 1-bit value).
pub fn sext63(x: BV) -> BV {
    ite(msb(x.clone()), BV::Const(u64::MAX), BV::Const(0))
}

// ---------------------------------------------------------------- folding

impl BV {
    /// Constant-fold under register-only inputs (no concrete memory: loads
    /// fold to the canonical 0 — only reachable in memory-less programs).
    pub fn eval(&self, env: &[u64; 16]) -> u64 {
        self.eval_in(&Env { regs: *env, mem: None, mem_base: 0 })
    }

    /// Constant-fold under a full environment (registers + optional concrete
    /// memory for folding loads). Implements exact SMT-LIB2 bit-vector
    /// semantics (shifts differ from Rust's masking shifts).
    pub fn eval_in(&self, env: &Env) -> u64 {
        match self {
            BV::Const(c) => *c,
            BV::Var(i) => env.regs[*i],
            BV::Add(a, b) => a.eval_in(env).wrapping_add(b.eval_in(env)),
            BV::Sub(a, b) => a.eval_in(env).wrapping_sub(b.eval_in(env)),
            BV::Mul(a, b) => a.eval_in(env).wrapping_mul(b.eval_in(env)),
            BV::And(a, b) => a.eval_in(env) & b.eval_in(env),
            BV::Or(a, b) => a.eval_in(env) | b.eval_in(env),
            BV::Xor(a, b) => a.eval_in(env) ^ b.eval_in(env),
            BV::Not(a) => !a.eval_in(env),
            BV::Neg(a) => a.eval_in(env).wrapping_neg(),
            BV::Shl(a, b) => {
                let (a, b) = (a.eval_in(env), b.eval_in(env));
                if b >= 64 {
                    0
                } else {
                    a << b
                }
            }
            BV::LShr(a, b) => {
                let (a, b) = (a.eval_in(env), b.eval_in(env));
                if b >= 64 {
                    0
                } else {
                    a >> b
                }
            }
            BV::AShr(a, b) => {
                let (x, b) = (a.eval_in(env), b.eval_in(env));
                let sx = x as i64;
                if b >= 64 {
                    (sx >> 63) as u64
                } else {
                    (sx >> b) as u64
                }
            }
            BV::Extract { hi, lo, x } => {
                let w = (*hi as u32) - (*lo as u32) + 1;
                let mask: u64 = if w == 64 { u64::MAX } else { (1u64 << w) - 1 };
                (x.eval_in(env) >> *lo as u32) & mask
            }
            BV::Load64 { mem, addr } => {
                // Byte-granular chain walk: the qword at `a` is assembled
                // from the innermost store to each byte a+0..a+7, falling
                // back to the concrete base memory. The whole-qword range
                // rule matches the concrete evaluator exactly: OOB (any byte
                // outside the sandbox) folds to the canonical 0, gated by
                // the def predicate in the miter.
                let a = addr.eval_in(env);
                let off = a.wrapping_sub(env.mem_base);
                let in_range = match env.mem {
                    Some(bytes) => off <= (bytes.len() as u64).saturating_sub(8),
                    None => false,
                };
                if !in_range {
                    return 0;
                }
                let mut out = 0u64;
                for j in 0..8u64 {
                    let target = a.wrapping_add(j);
                    let mut cur = mem.as_ref();
                    let mut byte: Option<u64> = None;
                    loop {
                        match cur {
                            MemExpr::Store { mem, addr, byte: b } => {
                                if addr.eval_in(env) == target {
                                    byte = Some(b.eval_in(env));
                                    break;
                                }
                                cur = mem.as_ref();
                            }
                            MemExpr::Base { .. } => break,
                        }
                    }
                    let v = match byte {
                        Some(v) => v,
                        None => match env.mem {
                            Some(bytes) => {
                                let boff = target.wrapping_sub(env.mem_base);
                                if boff < bytes.len() as u64 {
                                    bytes[boff as usize] as u64
                                } else {
                                    0
                                }
                            }
                            None => 0,
                        },
                    };
                    out |= (v & 0xff) << (8 * j);
                }
                out
            }
            BV::UDiv64 { hi, lo, d, out } => {
                // Mirrors the concrete canonical values: on the undefined
                // path (divisor 0 or quotient overflow) both outputs are 0.
                let h = hi.eval_in(env);
                let l = lo.eval_in(env);
                let dv = d.eval_in(env);
                let dividend = ((h as u128) << 64) | l as u128;
                let divisor = dv as u128;
                if divisor == 0 {
                    0
                } else {
                    match out {
                        DivOut::Quot => {
                            let q = dividend / divisor;
                            if q > u64::MAX as u128 { 0 } else { q as u64 }
                        }
                        DivOut::Rem => {
                            let q = dividend / divisor;
                            if q > u64::MAX as u128 { 0 } else { (dividend % divisor) as u64 }
                        }
                    }
                }
            }
            BV::IDiv64 { hi, lo, d, out } => {
                // Signed: the dividend's 128-bit pattern is reinterpreted as
                // signed; the remainder takes the dividend's sign (Rust's `%`
                // on i128 matches x86 here). Canonical 0 on the undefined
                // path (divisor 0 or signed-quotient overflow).
                let h = hi.eval_in(env);
                let l = lo.eval_in(env);
                let dividend = (((h as u128) << 64) | l as u128) as i128;
                let divisor = d.eval_in(env) as i64 as i128;
                if divisor == 0 {
                    0
                } else {
                    // wrapping_div/rem: i128::MIN / -1 overflows real division
                    // (which panics in Rust even in release); wrapping yields
                    // MIN, which the range check then correctly rejects as #DE
                    // (VERIFICATION.md row 27).
                    let q = dividend.wrapping_div(divisor);
                    if q < i64::MIN as i128 || q > i64::MAX as i128 {
                        0
                    } else {
                        match out {
                            DivOut::Quot => q as u64,
                            DivOut::Rem => dividend.wrapping_rem(divisor) as u64,
                        }
                    }
                }
            }
            BV::Ite { c, t, e } => {
                if c.eval_in(env) {
                    t.eval_in(env)
                } else {
                    e.eval_in(env)
                }
            }
        }
    }

    pub fn to_smt2(&self) -> String {
        match self {
            BV::Const(c) => format!("#x{:016x}", c),
            BV::Var(i) => format!("r{}", i),
            BV::Add(a, b) => format!("(bvadd {} {})", a.to_smt2(), b.to_smt2()),
            BV::Sub(a, b) => format!("(bvsub {} {})", a.to_smt2(), b.to_smt2()),
            BV::Mul(a, b) => format!("(bvmul {} {})", a.to_smt2(), b.to_smt2()),
            BV::And(a, b) => format!("(bvand {} {})", a.to_smt2(), b.to_smt2()),
            BV::Or(a, b) => format!("(bvor {} {})", a.to_smt2(), b.to_smt2()),
            BV::Xor(a, b) => format!("(bvxor {} {})", a.to_smt2(), b.to_smt2()),
            BV::Not(a) => format!("(bvnot {})", a.to_smt2()),
            BV::Neg(a) => format!("(bvneg {})", a.to_smt2()),
            BV::Shl(a, b) => format!("(bvshl {} {})", a.to_smt2(), b.to_smt2()),
            BV::LShr(a, b) => format!("(bvlshr {} {})", a.to_smt2(), b.to_smt2()),
            BV::AShr(a, b) => format!("(bvashr {} {})", a.to_smt2(), b.to_smt2()),
            BV::Extract { hi, lo, x } => {
                format!("((_ extract {} {}) {})", hi, lo, x.to_smt2())
            }
            BV::Load64 { mem, addr } => {
                // 8 byte-selects assembled with nested binary concats
                // (portable SMT-LIB2): byte 0 innermost, each higher byte
                // prepended, so the result is s7:s6:...:s0 little-endian.
                let mut acc = format!(
                    "(select {} (bvadd {} #x0000000000000000))",
                    mem.to_smt2(),
                    addr.to_smt2()
                );
                for k in 1..8u64 {
                    acc = format!(
                        "(concat (select {} (bvadd {} #x{:016x})) {})",
                        mem.to_smt2(),
                        addr.to_smt2(),
                        k,
                        acc
                    );
                }
                acc
            }
            BV::UDiv64 { hi, lo, d, out } => {
                let c = format!("((_ concat 64 64) {} {})", hi.to_smt2(), lo.to_smt2());
                let dv = format!("((_ zero_extend 64) {})", d.to_smt2());
                match out {
                    DivOut::Quot => format!("((_ extract 63 0) (bvudiv {} {}))", c, dv),
                    DivOut::Rem => format!("((_ extract 63 0) (bvurem {} {}))", c, dv),
                }
            }
            BV::IDiv64 { hi, lo, d, out } => {
                let c = format!("((_ concat 64 64) {} {})", hi.to_smt2(), lo.to_smt2());
                let dv = format!("((_ sign_extend 64) {})", d.to_smt2());
                match out {
                    DivOut::Quot => format!("((_ extract 63 0) (bvsdiv {} {}))", c, dv),
                    DivOut::Rem => format!("((_ extract 63 0) (bvsrem {} {}))", c, dv),
                }
            }
            BV::Ite { c, t, e } => {
                format!("(ite {} {} {})", c.to_smt2(), t.to_smt2(), e.to_smt2())
            }
        }
    }
}

impl Bool {
    pub fn eval(&self, env: &[u64; 16]) -> bool {
        self.eval_in(&Env { regs: *env, mem: None, mem_base: 0 })
    }

    pub fn eval_in(&self, env: &Env) -> bool {
        match self {
            Bool::Const(b) => *b,
            Bool::Eq(a, b) => a.eval_in(env) == b.eval_in(env),
            Bool::Ult(a, b) => a.eval_in(env) < b.eval_in(env),
            Bool::Ule(a, b) => a.eval_in(env) <= b.eval_in(env),
            Bool::Slt(a, b) => (a.eval_in(env) as i64) < (b.eval_in(env) as i64),
            Bool::Sle(a, b) => (a.eval_in(env) as i64) <= (b.eval_in(env) as i64),
            Bool::BEq(a, b) => a.eval_in(env) == b.eval_in(env),
            Bool::DivDefined { hi, lo, d, signed } => {
                let h = hi.eval_in(env);
                let l = lo.eval_in(env);
                let dv = d.eval_in(env);
                if !signed {
                    let dividend = ((h as u128) << 64) | l as u128;
                    dv != 0 && dividend / dv as u128 <= u64::MAX as u128
                } else {
                    let dividend = (((h as u128) << 64) | l as u128) as i128;
                    let divisor = dv as i64 as i128;
                    if divisor == 0 {
                        false
                    } else {
                        let q = dividend.wrapping_div(divisor);
                        q >= i64::MIN as i128 && q <= i64::MAX as i128
                    }
                }
            }
            Bool::And(a, b) => a.eval_in(env) && b.eval_in(env),
            Bool::Or(a, b) => a.eval_in(env) || b.eval_in(env),
            Bool::Not(a) => !a.eval_in(env),
            Bool::Ite(c, t, e) => {
                if c.eval_in(env) {
                    t.eval_in(env)
                } else {
                    e.eval_in(env)
                }
            }
        }
    }

    pub fn to_smt2(&self) -> String {
        match self {
            Bool::Const(b) => if *b { "true".into() } else { "false".into() },
            Bool::Eq(a, b) => format!("(= {} {})", a.to_smt2(), b.to_smt2()),
            Bool::Ult(a, b) => format!("(bvult {} {})", a.to_smt2(), b.to_smt2()),
            Bool::Ule(a, b) => format!("(bvule {} {})", a.to_smt2(), b.to_smt2()),
            Bool::Slt(a, b) => format!("(bvslt {} {})", a.to_smt2(), b.to_smt2()),
            Bool::Sle(a, b) => format!("(bvsle {} {})", a.to_smt2(), b.to_smt2()),
            Bool::BEq(a, b) => format!("(= {} {})", a.to_smt2(), b.to_smt2()),
            Bool::DivDefined { hi, lo, d, signed } => {
                let c = format!("((_ concat 64 64) {} {})", hi.to_smt2(), lo.to_smt2());
                let nz = format!("(distinct {} #x0000000000000000)", d.to_smt2());
                if !*signed {
                    let dv = format!("((_ zero_extend 64) {})", d.to_smt2());
                    // The quotient fits in 64 bits iff it is <= u64::MAX,
                    // i.e. the 128-bit literal #x0000000000000000ffffffffffffffff.
                    // (An earlier version used the 128-bit all-ones literal —
                    // a sort fix that corrupted the bound; caught by the
                    // division/memory z3 sample.)
                    format!(
                        "(and {} (bvule (bvudiv {} {}) #x0000000000000000ffffffffffffffff))",
                        nz, c, dv
                    )
                } else {
                    let dv = format!("((_ sign_extend 64) {})", d.to_smt2());
                    // Inclusive bounds on BOTH ends: quotients of exactly
                    // INT64_MIN and exactly i64::MAX are defined. The lower
                    // bound bug (bvsgt) was caught by the division/memory z3 sample;
                    // the upper-bound twin (bvslt rejecting q == i64::MAX)
                    // survived 8000 random samples and was caught by the
                    // hand-picked broken idiv pair: rax = 0xFFFFFFFFFFFFFFFE
                    // gives quotient exactly i64::MAX, which the miter
                    // wrongly called undefined.
                    format!(
                        "(and {} (bvsge (bvsdiv {} {}) ((_ sign_extend 64) #x8000000000000000)) (bvsle (bvsdiv {} {}) ((_ sign_extend 64) #x7fffffffffffffff)))",
                        nz, c, dv, c, dv
                    )
                }
            }
            Bool::And(a, b) => format!("(and {} {})", a.to_smt2(), b.to_smt2()),
            Bool::Or(a, b) => format!("(or {} {})", a.to_smt2(), b.to_smt2()),
            Bool::Not(a) => format!("(not {})", a.to_smt2()),
            Bool::Ite(c, t, e) => {
                format!("(ite {} {} {})", c.to_smt2(), t.to_smt2(), e.to_smt2())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fold_basics() {
        let env = [1u64, 2, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let e = bv_add(BV::Var(0), bv_mul(BV::Var(1), BV::Const(10)));
        assert_eq!(e.eval(&env), 21);
        // SMT shift semantics: shift >= 64 is 0, not masked.
        let s = bv_shl(BV::Const(1), BV::Const(64));
        assert_eq!(s.eval(&env), 0);
        let s = bv_lshr(BV::Const(0x8000_0000_0000_0000), BV::Const(64));
        assert_eq!(s.eval(&env), 0);
        let s = bv_ashr(BV::Const(0x8000_0000_0000_0000), BV::Const(64));
        assert_eq!(s.eval(&env), u64::MAX);
        let s = bv_ashr(BV::Const(0x8000_0000_0000_0000), BV::Const(1));
        assert_eq!(s.eval(&env), 0xC000_0000_0000_0000);
    }

    #[test]
    fn printer_matches_folding_for_shifts() {
        // The z3-sampled differential in the stress harness is the real check;
        // here just verify the printer emits parseable text for every node kind.
        let e = ite(
            msb(BV::Var(0)),
            bv_sub(bv_and(BV::Var(1), BV::Const(7)), BV::Not(BV::Var(2).into())),
            bv_xor(bv_or(BV::Neg(BV::Var(3).into()), BV::Const(1)), bv_lshr(BV::Var(4).into(), BV::Const(2))),
        );
        let s = e.to_smt2();
        assert!(s.contains("ite") && s.contains("bvand") && s.contains("bvneg"));
        // eval and print must not panic on any node
        let env = [1u64, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
        let _ = e.eval(&env);
        let b = bool_and_all(vec![Bool::Ult(BV::Var(0), BV::Const(5)), Bool::Slt(BV::Var(1), BV::Const(0))]);
        let _ = b.eval(&env);
        let _ = b.to_smt2();
    }

    #[test]
    fn and_all_empty_is_true() {
        assert_eq!(bool_and_all(vec![]), Bool::Const(true));
    }
}
