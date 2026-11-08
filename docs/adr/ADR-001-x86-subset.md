# ADR-001: v0 operates on a subset of x86-64, parsed from GAS AT&T syntax; partial-register writes are rejected

## Status
Accepted

## Context
The input language and register model for the semantics layer must be chosen.
Real x86-64 has 16 GPRs with 8/16/32/64-bit views, aliasing rules (including
the `ah` quirk), and sub-register write semantics (32-bit writes zero-extend,
8/16-bit writes merge). STOKE's own codebase shows the cost: their validator
zeroes the upper half on 32-bit sub-register writes while their *sandbox*
`CpuState::gp_update()` merge-writes the same case — two components of the
same project disagree on the semantics. (Sources: StanfordPL/stoke
`src/validator/README.md`, `src/state/cpu_state.h`.)

## Decision
1. v0 input is a curated subset of x86-64 written in GAS AT&T syntax, 64-bit
   operand forms only: `movq, addq, subq, imulq, andq, orq, xorq, cmpq,
   shlq, shrq, sarq, leaq, set<cc>, cmov<cc>`.
2. The only register operands accepted are the 16 full-width names
   (`%rax`...`%r15`). **Any other register name is a parse-time rejection**
   with an explicit error. There is exactly one documented exception: the
   shift count operand may be `%cl`, which is a *read* of the low byte of
   `%rcx` (hardware uses CL; masking makes `RCX & 0x3F` equivalent).
3. `set<cc>` writes the full 64-bit register, zero-extended (`dst := cond?1:0`).
   This **diverges from hardware**, which writes one byte. It is an explicit,
   documented modeling simplification that keeps the no-partial-write
   invariant; it will be replaced together with full partial-register
   support at a later milestone. Consequence: v0 output must not be
   copy-pasted into real assembly containing `set<cc>` without noting this.
4. `imulq` accepts a register source only (the immediate form is the
   three-operand IMUL, out of scope); rejected at parse time.

## Alternatives considered
- Model all sub-register views now (STOKE-style `read_gp/update` with
  width+start): rejected — this is precisely where silent unsoundness creeps
  in, and STOKE's own validator/sandbox divergence is the evidence.
- Invent a clean toy ISA: rejected — this loses the hardware differential-testing
  oracle that real x86-64 mnemonics give us for free at the x86 milestone.

## Consequences
Programs that a real assembler accepts may be rejected by the parser (e.g.,
`addl`, `%eax`, `sete %al`) — every rejection is an explicit error, never a
silent truncation or no-op.