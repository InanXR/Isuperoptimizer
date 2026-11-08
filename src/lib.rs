//! Isuperoptimizer — a certificate-first superoptimizer for a curated
//! x86-64 subset.
//!
//! The crate is organized by component:
//!
//! - [`semantics`]: instruction AST, parser, concrete and symbolic states,
//!   and the cost model.
//! - [`verifier`]: the SMT equivalence miter and the persistent z3 process.
//! - [`enumeration`]: exhaustive search with optimality certificates.
//! - [`search`]: STOKE-style MCMC, RNG, and fuzz generators.
//! - [`artifact`]: the append-only JSONL artifact of record.
//! - [`benchmarks`]: the hand-written suite and the two compiler batches.
//!
//! Leaf modules are re-exported at the crate root so the common API is
//! available without spelling the component path.

pub mod artifact;
pub mod benchmarks;
pub mod enumeration;
pub mod search;
pub mod semantics;
pub mod verifier;

pub use artifact::reportlog;
pub use benchmarks::{compiler_core, compiler_full};
pub use enumeration::enumerator;
pub use search::{fuzz, mcmc, rng};
pub use semantics::{ast, cost, instr, state, sym};
pub use verifier::{verify, z3proc};

pub use semantics::cost::{cost_on_testcases, TestCase, W_M};
pub use semantics::instr::{parse_instr, parse_program, Cc, Gpr, Instr, MemAddr, Operand, Program, ShiftCount, Slot};
pub use semantics::state::{eval_program, CState, Flags, MachineState};
pub use verifier::verify::{verify, Spec, Verdict};
pub use verifier::z3proc::{Answer, Z3Session};

/// The supported instruction set, as opcode strings (for enumeration/tests).
/// The subset includes the two-operand ALU and shift forms, `leaq`,
/// `setcc`/`cmovcc`, `negq`/`notq`, two-operand `imulq`, one-operand
/// `divq`/`idivq`, `movabsq`, and the accepted one-operand shift forms and
/// cc aliases on `setcc`/`cmovcc` below.
pub const OPCODES: &[&str] = &[
    "movq", "movabsq", "addq", "subq", "imulq", "andq", "orq", "xorq", "cmpq", "shlq", "salq", "shrq", "sarq",
    "leaq", "negq", "notq", "sete", "setne", "seta", "setae", "setb", "setbe", "setg", "setge", "setl", "setle",
    "cmove", "cmovne", "cmova", "cmovae", "cmovb", "cmovbe", "cmovg", "cmovge", "cmovl", "cmovle",
    "divq", "idivq",
];
