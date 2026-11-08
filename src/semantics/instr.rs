//! The v0 x86-64 subset: instructions, operands, GAS AT&T parser, printer.
//!
//! Rejection policy (ADR-001): anything outside the subset is an explicit
//! parse error — never a silent truncation, no-op, or name-mangling. In
//! particular, partial-register operands (`%al`, `%eax`, `%ah`, ...) are
//! rejected everywhere except the single documented read of `%cl` as a shift
//! count (hardware uses CL; with 6-bit masking, `RCX & 0x3F` is equivalent).

use std::fmt;

// ------------------------------------------------------------------- registers

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Gpr {
    Rax, Rcx, Rdx, Rbx, Rsp, Rbp, Rsi, Rdi,
    R8, R9, R10, R11, R12, R13, R14, R15,
}

impl Gpr {
    pub const ALL: [Gpr; 16] = [
        Gpr::Rax, Gpr::Rcx, Gpr::Rdx, Gpr::Rbx, Gpr::Rsp, Gpr::Rbp, Gpr::Rsi, Gpr::Rdi,
        Gpr::R8, Gpr::R9, Gpr::R10, Gpr::R11, Gpr::R12, Gpr::R13, Gpr::R14, Gpr::R15,
    ];

    pub fn idx(self) -> usize {
        self as usize
    }

    pub fn name(self) -> &'static str {
        match self {
            Gpr::Rax => "rax", Gpr::Rcx => "rcx", Gpr::Rdx => "rdx", Gpr::Rbx => "rbx",
            Gpr::Rsp => "rsp", Gpr::Rbp => "rbp", Gpr::Rsi => "rsi", Gpr::Rdi => "rdi",
            Gpr::R8 => "r8", Gpr::R9 => "r9", Gpr::R10 => "r10", Gpr::R11 => "r11",
            Gpr::R12 => "r12", Gpr::R13 => "r13", Gpr::R14 => "r14", Gpr::R15 => "r15",
        }
    }

    /// Accepts full 64-bit register names only. This is the single gate that
    /// implements the partial-register ban.
    pub fn from_name(s: &str) -> Option<Gpr> {
        Some(match s {
            "rax" => Gpr::Rax, "rcx" => Gpr::Rcx, "rdx" => Gpr::Rdx, "rbx" => Gpr::Rbx,
            "rsp" => Gpr::Rsp, "rbp" => Gpr::Rbp, "rsi" => Gpr::Rsi, "rdi" => Gpr::Rdi,
            "r8" => Gpr::R8, "r9" => Gpr::R9, "r10" => Gpr::R10, "r11" => Gpr::R11,
            "r12" => Gpr::R12, "r13" => Gpr::R13, "r14" => Gpr::R14, "r15" => Gpr::R15,
            _ => return None,
        })
    }
}

// ------------------------------------------------------------------- operands

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Operand {
    Reg(Gpr),
    /// Full 64-bit immediate (sign-extended imm32 or movabs imm64 both accepted).
    Imm(u64),
    /// Memory reference — legal only as the source of `leaq` in v0.
    Mem(MemAddr),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MemAddr {
    pub base: Option<Gpr>,
    pub index: Option<Gpr>,
    pub scale: u8, // 1, 2, 4, or 8
    pub disp: i64,
}

/// Shift count: immediate byte, or the CL register (low byte of RCX).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShiftCount {
    Imm(u8),
    Cl,
}

/// True iff `v` is encodable as a sign-extended imm32 — the only immediate
/// form real x86-64 ALU instructions have. Full-width constants must be
/// materialized into a register via movq/movabsq first, exactly as gcc
/// does. The v0 *model* (parser/evaluator) accepts wider ALU immediates as
/// an abstraction — committed artifacts' rejection witnesses replay such
/// candidates — but the searched subset (enumeration alphabet and MCMC
/// generators) is restricted to encodable forms so "beats the compiler"
/// compares like against like (ADR-011 encodability pass; the p23
/// first-run result rested on non-encodable `andq $imm64` forms).
pub fn imm32_encodable(v: u64) -> bool {
    let s = v as i64;
    (i32::MIN as i64..=i32::MAX as i64).contains(&s)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Cc {
    E, Ne, A, Ae, B, Be, G, Ge, L, Le,
}

impl Cc {
    pub const ALL: [Cc; 10] = [Cc::E, Cc::Ne, Cc::A, Cc::Ae, Cc::B, Cc::Be, Cc::G, Cc::Ge, Cc::L, Cc::Le];

    /// Canonical suffix. `aliases()` lists the alternative AT&T condition-code
    /// spellings real assemblers and compilers emit (z/nz/nb/nc/nae/c/na/
    /// nle/nl/nge/ng); `from_suffix` accepts them all and normalizes to the
    /// canonical code — the underlying condition is the same instruction
    /// (ADR-010).
    pub fn suffix(self) -> &'static str {
        match self {
            Cc::E => "e", Cc::Ne => "ne", Cc::A => "a", Cc::Ae => "ae", Cc::B => "b",
            Cc::Be => "be", Cc::G => "g", Cc::Ge => "ge", Cc::L => "l", Cc::Le => "le",
        }
    }

    pub fn from_suffix(s: &str) -> Option<Cc> {
        Some(match s {
            "e" | "z" => Cc::E,
            "ne" | "nz" => Cc::Ne,
            "a" | "nbe" => Cc::A,
            "ae" | "nb" | "nc" => Cc::Ae,
            "b" | "c" | "nae" => Cc::B,
            "be" | "na" => Cc::Be,
            "g" | "nle" => Cc::G,
            "ge" | "nl" => Cc::Ge,
            "l" | "nge" => Cc::L,
            "le" | "ng" => Cc::Le,
            _ => return None,
        })
    }
}

// ---------------------------------------------------------------- instructions

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Instr {
    Mov { src: Operand, dst: Gpr },
    Add { src: Operand, dst: Gpr },
    Sub { src: Operand, dst: Gpr },
    Imul { src: Gpr, dst: Gpr },
    And { src: Operand, dst: Gpr },
    Or { src: Operand, dst: Gpr },
    Xor { src: Operand, dst: Gpr },
    Cmp { src: Operand, dst: Gpr },
    Shl { count: ShiftCount, dst: Gpr },
    Shr { count: ShiftCount, dst: Gpr },
    Sar { count: ShiftCount, dst: Gpr },
    Lea { mem: MemAddr, dst: Gpr },
    /// `set<cc>` — v0 writes the full 64-bit register zero-extended (ADR-001).
    Set { cc: Cc, dst: Gpr },
    Cmov { cc: Cc, src: Gpr, dst: Gpr },
    /// `negq %r` — two's complement negate: dst := 0 - dst. Flags per SDM:
    /// CF = (dst != 0), OF = (dst == INT64_MIN), ZF/SF from the result.
    /// (Compiler-baseline extension; full pyramid treatment per ADR-010.)
    Neg { dst: Gpr },
    /// `notq %r` — bitwise complement: dst := !dst. Flags: UNAFFECTED
    /// (SDM: NOT, like MOV/LEA, performs no flag updates — the same
    /// flags-carry-through category). Full-provenance extension; full pyramid
    /// treatment per ADR-011.
    Not { dst: Gpr },
    /// Unsigned divide RDX:RAX by src (x86 one-operand DIV).
    Div { src: Gpr },
    /// Signed divide RDX:RAX by src (x86 one-operand IDIV).
    IDiv { src: Gpr },
    /// `movq mem, reg` — qword load from the sandbox (ADR-008).
    Load { mem: MemAddr, dst: Gpr },
    /// `movq reg, mem` — qword store (register source only in v0).
    Store { src: Gpr, mem: MemAddr },
}

/// A program slot. `Unused` is the STOKE padding token (fixed-ℓ search space).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Slot {
    Instr(Instr),
    Unused,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program(pub Vec<Slot>);

// ------------------------------------------------------------------- errors

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub line: String,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "parse error in `{}`: {}", self.line, self.message)
    }
}
impl std::error::Error for ParseError {}

fn err(line: &str, msg: impl Into<String>) -> ParseError {
    ParseError { line: line.to_string(), message: msg.into() }
}

// -------------------------------------------------------------------- parser

/// Parse one instruction line (with optional trailing comment) into a `Slot`.
pub fn parse_instr(line: &str) -> Result<Slot, ParseError> {
    let stripped = match line.find('#') {
        Some(i) => &line[..i],
        None => line,
    };
    let t = stripped.trim();
    if t.is_empty() {
        return Err(err(line, "empty line"));
    }
    let mut parts = t.split_whitespace();
    let opcode = parts.next().unwrap();
    let rest = t[opcode.len()..].trim();
    // Collect comma-separated operands, preserving parenthesized commas.
    let ops = split_operands(rest);

    if opcode == "unused" {
        if !ops.is_empty() {
            return Err(err(line, "`unused` takes no operands"));
        }
        return Ok(Slot::Unused);
    }

    macro_rules! reg {
        ($s:expr) => {
            match $s.strip_prefix('%') {
                Some(name) => Gpr::from_name(name).ok_or_else(|| {
                    err(
                        line,
                        format!(
                            "register `%{}` is not a 64-bit register; partial-register \
                             operands are rejected in v0 (ADR-001)",
                            name
                        ),
                    )
                })?,
                None => return Err(err(line, format!("expected register, got `{}`", $s))),
            }
        };
    }

    macro_rules! imm {
        ($s:expr) => {
            parse_imm($s, line)?
        };
    }

    let instr = match opcode {
        "movq" | "addq" | "subq" | "andq" | "orq" | "xorq" | "cmpq" | "movabsq" => {
            if ops.len() != 2 {
                return Err(err(line, format!("`{}` takes exactly 2 operands", opcode)));
            }
            // movabsq is imm64 -> reg only (ADR-010/011: it is movq with a
            // full-width immediate). Accepted for real in the full-provenance batch —
            // ADR-010 had claimed this, but the opcode was never in the
            // parser and the core-batch pinning test's `unwrap_or_else`
            // fallback made it self-neutralizing (VERIFICATION.md row 29).
            if opcode == "movabsq" && !ops[0].starts_with('$') {
                return Err(err(line, "`movabsq` takes an immediate source (imm64 -> reg)"));
            }
            let src = match ops[0].as_str() {
                s if s.starts_with('%') => Operand::Reg(reg!(s)),
                s if s.starts_with('$') => Operand::Imm(imm!(s)),
                s if s.contains('(') => {
                    if opcode != "movq" {
                        return Err(err(
                            line,
                            "memory operands are only supported by `leaq` and `movq` \
                             in v0 (ADR-004/008)",
                        ));
                    }
                    Operand::Mem(parse_mem(&ops[0], line)?)
                }
                s => return Err(err(line, format!("bad source operand `{}`", s))),
            };
            let dst = match ops[1].as_str() {
                s if s.starts_with('%') => reg!(s),
                s if s.contains('(') => {
                    if opcode != "movq" {
                        return Err(err(line, "memory destinations are only supported by `movq` in v0"));
                    }
                    let mem = parse_mem(&ops[1], line)?;
                    match src {
                        Operand::Reg(r) => return Ok(Slot::Instr(Instr::Store { src: r, mem })),
                        _ => {
                            return Err(err(
                                line,
                                "v0 stores take a register source only \
                                 (immediate stores are rejected)",
                            ))
                        }
                    }
                }
                _ => {
                    return Err(err(
                        line,
                        "memory operands are not supported as destinations in v0 \
                         (no memory model; ADR-004)",
                    ))
                }
            };
            match src {
                Operand::Mem(mem) if opcode == "movq" => {
                    return Ok(Slot::Instr(Instr::Load { mem, dst }))
                }
                src => match opcode {
                    "movq" | "movabsq" => Instr::Mov { src, dst },
                    "addq" => Instr::Add { src, dst },
                    "subq" => Instr::Sub { src, dst },
                    "andq" => Instr::And { src, dst },
                    "orq" => Instr::Or { src, dst },
                    "xorq" => Instr::Xor { src, dst },
                    _ => Instr::Cmp { src, dst },
                },
            }
        }
        "divq" | "idivq" => {
            if ops.len() != 1 {
                return Err(err(line, format!("`{}` takes exactly 1 register operand", opcode)));
            }
            let src = reg!(ops[0].as_str());
            if opcode == "divq" {
                Instr::Div { src }
            } else {
                Instr::IDiv { src }
            }
        }
        "imulq" => {
            if ops.len() != 2 {
                return Err(err(line, "`imulq` takes exactly 2 operands (reg src, reg dst)"));
            }
            // The immediate form is three-operand IMUL — out of v0 scope.
            let src = match ops[0].as_str() {
                s if s.starts_with('%') => reg!(s),
                _ => {
                    return Err(err(
                        line,
                        "`imulq` source must be a register; the immediate form is the \
                         three-operand IMUL, out of v0 scope",
                    ))
                }
            };
            let dst = reg!(ops[1].as_str());
            Instr::Imul { src, dst }
        }
        // SAL is the traditional synonym of SHL (identical opcode on every
        // x86; gas and gcc emit `salq` for variable left shifts — e.g. the
        // P19 exchange-fields program). Syntax acceptance only, per the
        // ADR-010 alias precedent: normalizes to Shl.
        "shlq" | "shrq" | "sarq" | "shl" | "shr" | "sar" | "salq" | "sal" => {
            if ops.len() == 1 {
                // one-operand AT&T form: shift by exactly 1 (opcode D1/E1)
                let dst = reg!(ops[0].as_str());
                match opcode {
                    "shlq" | "shl" | "salq" | "sal" => Instr::Shl { count: ShiftCount::Imm(1), dst },
                    "shrq" | "shr" => Instr::Shr { count: ShiftCount::Imm(1), dst },
                    _ => Instr::Sar { count: ShiftCount::Imm(1), dst },
                }
            } else if ops.len() != 2 {
                return Err(err(line, format!("`{}` takes 1 or 2 operands", opcode)));
            } else {
                let count = match ops[0].as_str() {
                    s if s.starts_with('%') => {
                        if s != "%cl" {
                            return Err(err(
                                line,
                                "shift count register must be `%cl`; other register operands \
                                 are rejected in v0",
                            ));
                        }
                        ShiftCount::Cl
                    }
                    s if s.starts_with('$') => ShiftCount::Imm(imm_shift(s, line)?),
                    s => return Err(err(line, format!("bad shift count `{}`", s))),
                };
                let dst = reg!(ops[1].as_str());
                match opcode {
                    "shlq" | "shl" | "salq" | "sal" => Instr::Shl { count, dst },
                    "shrq" | "shr" => Instr::Shr { count, dst },
                    _ => Instr::Sar { count, dst },
                }
            }
        }
        "leaq" => {
            if ops.len() != 2 {
                return Err(err(line, "`leaq` takes exactly 2 operands (mem src, reg dst)"));
            }
            let mem = parse_mem(&ops[0], line)?;
            let dst = reg!(ops[1].as_str());
            Instr::Lea { mem, dst }
        }
        op if op.starts_with("set") => {
            if ops.len() != 1 {
                return Err(err(line, format!("`{}` takes exactly 1 operand", op)));
            }
            let cc = Cc::from_suffix(&op[3..]).ok_or_else(|| err(line, format!("unknown setcc `{}`", op)))?;
            let dst = reg!(ops[0].as_str());
            Instr::Set { cc, dst }
        }
        op if op.starts_with("cmov") => {
            let suffix = op[4..].strip_suffix('q').unwrap_or(&op[4..]);
            let cc = Cc::from_suffix(suffix).ok_or_else(|| err(line, format!("unknown cmovcc `{}`", op)))?;
            if ops.len() != 2 {
                return Err(err(line, format!("`{}` takes exactly 2 operands", op)));
            }
            // Un-suffixed `cmovcc %r64, %r64` is the 64-bit form (gas infers
            // the size from the register operands — ADR-010); anything with a
            // non-64-bit register was already rejected by reg! above.
            let src = reg!(ops[0].as_str());
            let dst = reg!(ops[1].as_str());
            Instr::Cmov { cc, src, dst }
        }
        "negq" | "neg" => {
            if ops.len() != 1 {
                return Err(err(line, format!("`{}` takes exactly 1 register operand", opcode)));
            }
            let dst = reg!(ops[0].as_str());
            Instr::Neg { dst }
        }
        "notq" | "not" => {
            if ops.len() != 1 {
                return Err(err(line, format!("`{}` takes exactly 1 register operand", opcode)));
            }
            let dst = reg!(ops[0].as_str());
            Instr::Not { dst }
        }
        other => {
            return Err(err(
                line,
                format!("opcode `{}` is not in the v0 subset; rejected", other),
            ))
        }
    };
    Ok(Slot::Instr(instr))
}

/// Parse a whole program: one instruction per line; `#` comments; blank lines skipped.
pub fn parse_program(text: &str) -> Result<Program, ParseError> {
    let mut v = Vec::new();
    for line in text.lines() {
        let stripped = match line.find('#') {
            Some(i) => &line[..i],
            None => line,
        };
        if stripped.trim().is_empty() {
            continue;
        }
        v.push(parse_instr(line)?);
    }
    Ok(Program(v))
}

/// Parse a real `gcc -S` assembly file into (function name, program) pairs
/// (ADR-010). This is the compiled-binary transcription layer that the
/// `compiler_oracle` stress mode validates empirically.
///
/// Explicit rules — nothing is silently dropped:
/// - dot-directive lines are skipped (`.text`, `.globl`, `.cfi_*`, `.size`,
///   ...): they carry no architectural state semantics;
/// - `endbr64` is skipped: a CET landing-pad NOP with no effect on GPRs or
///   flags (the compiler oracle validates the resulting behavior empirically);
/// - exactly one function label per function; gcc local labels (`.L...:`)
///   are a HARD ERROR (control flow), as is any second label;
/// - a single trailing `ret` / `rep ret` ends the function (the call/return
///   protocol is outside the straight-line model; dropping it is the defined
///   transformation from "function" to "program");
/// - every remaining line must parse as a subset instruction.
pub fn parse_gcc_asm(text: &str) -> Result<Vec<(String, Program)>, ParseError> {
    let mut funcs: Vec<(String, Program)> = Vec::new();
    let mut cur_name: Option<String> = None;
    let mut slots: Vec<Slot> = Vec::new();
    let mut seen_ret = false;

    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('.') && !t.starts_with(".L") {
            // directive (but .L labels fall through to the label check below)
            if t.starts_with('.') && !t.contains(':') {
                continue;
            }
        }
        if t.starts_with(".L") {
            if t.starts_with(".LFB") || t.starts_with(".LFE") {
                // unconditional frame-begin/end markers gcc emits per
                // function; not jump targets
                continue;
            }
            return Err(err(line, "gcc local label — control flow outside the straight-line subset"));
        }
        if let Some(label) = t.strip_suffix(':') {
            if !label.starts_with('.') {
                if let Some(name) = cur_name.take() {
                    if !slots.is_empty() {
                        return Err(err(line, format!("label `{}` follows instructions of `{}` without a ret — control flow", label, name)));
                    }
                    // a function with no body (declaration only)
                    funcs.push((name, Program(slots)));
                    slots = Vec::new();
                }
                if seen_ret {
                    return Err(err(line, "label after ret — control flow"));
                }
                seen_ret = false;
                cur_name = Some(label.to_string());
                continue;
            }
        }
        if t == "endbr64" {
            continue;
        }
        if t == "ret" || t == "rep ret" || t == "rep\tret" {
            let name = cur_name
                .take()
                .ok_or_else(|| err(line, "ret outside any function"))?;
            funcs.push((name, Program(std::mem::take(&mut slots))));
            seen_ret = true;
            continue;
        }
        if cur_name.is_none() {
            if t.starts_with('.') || t.is_empty() {
                continue;
            }
            return Err(err(line, "instruction outside any function label"));
        }
        if seen_ret {
            return Err(err(line, "instruction after ret — control flow"));
        }
        slots.push(parse_instr(line)?);
    }
    if let Some(name) = cur_name {
        if !slots.is_empty() {
            return Err(err(
                "end of file",
                format!("function `{}` has instructions but no ret", name),
            ));
        }
        funcs.push((name, Program(slots)));
    }
    Ok(funcs)
}

fn split_operands(s: &str) -> Vec<String> {
    let s = s.trim();
    if s.is_empty() {
        return vec![];
    }
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut depth = 0usize;
    for c in s.chars() {
        match c {
            '(' => {
                depth += 1;
                cur.push(c);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                cur.push(c);
            }
            ',' if depth == 0 => {
                out.push(cur.trim().to_string());
                cur = String::new();
            }
            _ => cur.push(c),
        }
    }
    out.push(cur.trim().to_string());
    out
}

fn parse_imm(s: &str, line: &str) -> Result<u64, ParseError> {
    let body = s.strip_prefix('$').unwrap();
    let v: i128 = if let Some(h) = body.strip_prefix("0x").or_else(|| body.strip_prefix("-0x")) {
        let neg = body.starts_with('-');
        let v = i128::from_str_radix(h, 16).map_err(|_| err(line, format!("bad hex immediate `{}`", s)))?;
        if neg { -v } else { v }
    } else {
        body.parse::<i128>().map_err(|_| err(line, format!("bad immediate `{}`", s)))?
    };
    // Accept sign-extended imm32 or movabs imm64 — i.e. anything in i64 range.
    if v < i64::MIN as i128 || v > u64::MAX as i128 {
        return Err(err(line, format!("immediate `{}` does not fit in 64 bits", s)));
    }
    Ok(v as u64)
}

fn imm_shift(s: &str, line: &str) -> Result<u8, ParseError> {
    let v = parse_imm(s, line)? as i64;
    if !(-128..=255).contains(&v) {
        return Err(err(line, format!("shift count `{}` does not fit in imm8", s)));
    }
    Ok(v as u8)
}

fn parse_mem(s: &str, line: &str) -> Result<MemAddr, ParseError> {
    let open = s.find('(').ok_or_else(|| err(line, format!("`{}` is not a memory reference", s)))?;
    let close = s.rfind(')').ok_or_else(|| err(line, format!("unbalanced parentheses in `{}`", s)))?;
    if close != s.len() - 1 {
        return Err(err(line, format!("trailing characters after memory operand `{}`", s)));
    }
    let disp_s = s[..open].trim();
    let disp: i64 = if disp_s.is_empty() {
        0
    } else {
        disp_s.parse().map_err(|_| err(line, format!("bad displacement `{}`", disp_s)))?
    };
    let inner = &s[open + 1..close];
    let fields: Vec<Option<&str>> = inner.split(',').map(|f| {
        let f = f.trim();
        if f.is_empty() { None } else { Some(f) }
    }).collect();
    let (base, index, scale) = match fields.len() {
        1 => (reg_field(fields[0], line)?, None, 1),
        2 => (reg_field(fields[0], line)?, reg_field(fields[1], line)?, 1),
        3 => {
            let scale = match fields[2] {
                None | Some("1") => 1u8,
                Some("2") => 2,
                Some("4") => 4,
                Some("8") => 8,
                Some(x) => return Err(err(line, format!("scale `{}` must be 1, 2, 4, or 8", x))),
            };
            (reg_field(fields[0], line)?, reg_field(fields[1], line)?, scale)
        }
        n => return Err(err(line, format!("memory operand has {} fields, expected 1-3", n))),
    };
    if base.is_none() && index.is_none() {
        return Err(err(line, "memory operand needs at least a base or index register"));
    }
    Ok(MemAddr { base, index, scale, disp })
}

fn reg_field<'a>(f: Option<&'a str>, line: &str) -> Result<Option<Gpr>, ParseError> {
    match f {
        None => Ok(None),
        Some(r) => match r.strip_prefix('%') {
            Some(name) => Gpr::from_name(name).map(Some).ok_or_else(|| {
                err(
                    line,
                    format!(
                        "register `%{}` is not a 64-bit register; partial-register \
                         operands are rejected in v0 (ADR-001)",
                        name
                    ),
                )
            }),
            None => Err(err(line, format!("expected register in memory operand, got `{}`", r))),
        },
    }
}

// ------------------------------------------------------------------ printing

impl fmt::Display for Operand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Operand::Reg(r) => write!(f, "%{}", r.name()),
            Operand::Imm(v) => {
                if *v <= i64::MAX as u64 {
                    write!(f, "${}", *v as i64)
                } else {
                    write!(f, "$0x{:x}", v)
                }
            }
            Operand::Mem(m) => write!(f, "{}", m),
        }
    }
}

impl fmt::Display for MemAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut s = String::new();
        if self.disp != 0 {
            s.push_str(&self.disp.to_string());
        }
        s.push('(');
        if let Some(b) = self.base {
            s.push_str(&format!("%{}", b.name()));
        }
        if let Some(i) = self.index {
            // GAS needs the leading comma when the base is absent (e.g. `(,%rdx,8)`).
            s.push(',');
            s.push_str(&format!("%{}", i.name()));
            if self.scale != 1 {
                s.push_str(&format!(",{}", self.scale));
            }
        } else if self.base.is_none() {
            // unreachable via the parser; kept for total Display
            s.push_str(",");
        }
        s.push(')');
        write!(f, "{}", s)
    }
}

impl fmt::Display for ShiftCount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ShiftCount::Imm(v) => write!(f, "${}", *v as i8),
            ShiftCount::Cl => write!(f, "%cl"),
        }
    }
}

impl fmt::Display for Instr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Instr::Mov { src, dst } => write!(f, "movq {}, {}", src, dst.name_prefixed()),
            Instr::Add { src, dst } => write!(f, "addq {}, {}", src, dst.name_prefixed()),
            Instr::Sub { src, dst } => write!(f, "subq {}, {}", src, dst.name_prefixed()),
            Instr::Imul { src, dst } => write!(f, "imulq %{}, %{}", src.name(), dst.name()),
            Instr::And { src, dst } => write!(f, "andq {}, {}", src, dst.name_prefixed()),
            Instr::Or { src, dst } => write!(f, "orq {}, {}", src, dst.name_prefixed()),
            Instr::Xor { src, dst } => write!(f, "xorq {}, {}", src, dst.name_prefixed()),
            Instr::Cmp { src, dst } => write!(f, "cmpq {}, {}", src, dst.name_prefixed()),
            Instr::Shl { count, dst } => write!(f, "shlq {}, {}", count, dst.name_prefixed()),
            Instr::Shr { count, dst } => write!(f, "shrq {}, {}", count, dst.name_prefixed()),
            Instr::Sar { count, dst } => write!(f, "sarq {}, {}", count, dst.name_prefixed()),
            Instr::Lea { mem, dst } => write!(f, "leaq {}, {}", mem, dst.name_prefixed()),
            Instr::Set { cc, dst } => write!(f, "set{} %{}", cc.suffix(), dst.name()),
            Instr::Cmov { cc, src, dst } => write!(f, "cmov{}q %{}, %{}", cc.suffix(), src.name(), dst.name()),
            Instr::Neg { dst } => write!(f, "negq %{}", dst.name()),
            Instr::Not { dst } => write!(f, "notq %{}", dst.name()),
            Instr::Div { src } => write!(f, "divq %{}", src.name()),
            Instr::IDiv { src } => write!(f, "idivq %{}", src.name()),
            Instr::Load { mem, dst } => write!(f, "movq {}, %{}", mem, dst.name()),
            Instr::Store { src, mem } => write!(f, "movq %{}, {}", src.name(), mem),
        }
    }
}

impl Gpr {
    pub fn name_prefixed(self) -> String {
        format!("%{}", self.name())
    }
}

impl fmt::Display for Slot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Slot::Instr(i) => write!(f, "{}", i),
            Slot::Unused => write!(f, "unused"),
        }
    }
}

impl fmt::Display for Program {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for s in &self.0 {
            writeln!(f, "{}", s)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_forms() {
        let p = parse_program(
            "# comment\n\
             movq %rax, %rbx\n\
             movq $-1, %rcx\n\
             movq $0xdeadbeef, %rdx\n\
             addq $5, %rbx\n\
             imulq %rsi, %rdi\n\
             shlq $1, %rax\n\
             shrq %cl, %rbx\n\
             sarq $63, %r12\n\
             leaq 8(%rsi,%rcx,4), %r8\n\
             leaq (%rbp), %r9\n\
             leaq -16(,%rdx,8), %r10\n\
             cmpq %rax, %rdx\n\
             sete %r11\n\
             cmoveq %rsi, %rdi\n\
             unused\n",
        )
        .unwrap();
        assert_eq!(p.0.len(), 15);
        assert_eq!(p.0[14], Slot::Unused);
    }

    #[test]
    fn rejects_partial_registers() {
        for bad in ["movl %eax, %ebx", "addq %al, %rbx", "sete %al", "movq %rax, %eax", "shlq %dl, %rax"] {
            assert!(parse_instr(bad).is_err(), "should reject: {}", bad);
        }
        let e = parse_instr("addq %al, %rbx").unwrap_err();
        assert!(e.message.contains("partial-register"), "{}", e);
    }

    #[test]
    fn rejects_out_of_subset() {
        for bad in [
            "movq (%rax), (%rbx)",   // memory-to-memory
            "movq $5, (%rax)",       // immediate stores rejected in v0
            "addq %rax, (%rbx)",     // arithmetic with a memory destination
            "jne .L2",               // no branches
            "pushq %rax",
            "shlq %dl, %rax",        // only %cl allowed
            "divq $5",               // division takes a register only
            "divl %eax",             // 32-bit forms
            "negl %eax",             // neg is 64-bit register only in v0
            "notl %eax",             // not is 64-bit register only in v0
            "salq %dl, %rax",        // salq is the shlq synonym: only %cl allowed
        ] {
            assert!(parse_instr(bad).is_err(), "should reject: {}", bad);
        }
    }

    #[test]
    fn parses_neg_not_and_cmov_aliases() {
        assert_eq!(
            parse_instr("negq %rcx").unwrap(),
            Slot::Instr(Instr::Neg { dst: Gpr::Rcx })
        );
        // The full-provenance batch: notq joins the subset (full pyramid per ADR-011).
        assert_eq!(
            parse_instr("notq %rcx").unwrap(),
            Slot::Instr(Instr::Not { dst: Gpr::Rcx })
        );
        assert_eq!(
            parse_instr("not %rcx").unwrap(),
            parse_instr("notq %rcx").unwrap(),
            "unsuffixed not == notq with 64-bit register"
        );
        // salq is the shlq synonym (gcc emits it for P19's variable shift)
        assert_eq!(
            parse_instr("salq %cl, %rax").unwrap(),
            parse_instr("shlq %cl, %rax").unwrap(),
            "salq normalizes to Shl"
        );
        assert_eq!(
            parse_instr("salq %rax").unwrap(),
            parse_instr("shlq $1, %rax").unwrap(),
            "one-operand salq == shift by 1"
        );
        // unsuffixed cmov with 64-bit registers is the 64-bit form
        assert_eq!(
            parse_instr("cmovnb %rbx, %rax").unwrap(),
            Slot::Instr(Instr::Cmov { cc: Cc::Ae, src: Gpr::Rbx, dst: Gpr::Rax })
        );
        assert_eq!(
            parse_instr("cmovz %rbx, %rax").unwrap(),
            Slot::Instr(Instr::Cmov { cc: Cc::E, src: Gpr::Rbx, dst: Gpr::Rax })
        );
        // condition-code aliases on setcc too, but still 64-bit regs only
        assert!(parse_instr("setnz %cl").is_err());
    }

    #[test]
    fn gcc_asm_parser_rules() {
        let good = "\
f:
\tendbr64
\tmovq %rdi, %rax
\tnegq %rax
\tret
";
        let fs = parse_gcc_asm(good).unwrap();
        assert_eq!(fs.len(), 1);
        assert_eq!(fs[0].0, "f");
        assert_eq!(fs[0].1 .0.len(), 2);

        // local label = hard error
        assert!(parse_gcc_asm("f:\n.L3:\n\tret\n").is_err());
        // branch = hard error
        assert!(parse_gcc_asm("f:\n\tje .L3\n\tret\n").is_err());
        // two labels without ret = error
        assert!(parse_gcc_asm("f:\n\tmovq %rax, %rbx\ng:\n\tret\n").is_err());
        // instruction after ret = error
        assert!(parse_gcc_asm("f:\n\tret\n\tmovq %rax, %rbx\n").is_err());
    }

    #[test]
    fn display_roundtrip() {
        let src = "leaq 8(%rsi,%rcx,4), %r8\nshlq %cl, %rbx\nsetle %r11\nimulq %rsi, %rdi\n";
        let p = parse_program(src).unwrap();
        let printed = p.to_string();
        let p2 = parse_program(&printed).unwrap();
        assert_eq!(p, p2);
    }
}
