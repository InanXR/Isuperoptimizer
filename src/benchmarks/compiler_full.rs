//! Full-provenance compiler batch. The registry below is every P1-P25
//! program of Gulwani, Jha, Tiwari, Venkatesan (PLDI 2011, Figure 4, the
//! Hacker's Delight set STOKE ran), ported to uint64_t, plus the core
//! batch's extras that are not among the 25
//! (smin/umin/umax/sat_add/select_bits/sign_mask).
//!
//! Reconciliation at -O2/-O3 (gcc 15.2.0, identical output at both
//! levels for the whole batch): 23 survivors, 8 exclusions with
//! empirically-confirmed parse errors. The two extensions this batch
//! forced are recorded in ADR-011: `notq` (full semantics-layer pyramid)
//! and the `salq` syntax alias for `shlq`.
//!
//! Porting decisions (documented per function in
//! `fixtures/compiler-full/candidates.c`):
//! the four 32-bit-constant programs are scaled by doubling the word
//! size, as the core batch's next_pow2 already did; P20's and P23's Figure-4
//! lines that mis-transcribe HD (verified against HD's own formulas)
//! follow HD; every source is committed verbatim and is the compiled
//! oracle's input.

use crate::compiler_core::CompilerFunc;

pub const GCC_FLAGS_BASE: &str = crate::compiler_core::GCC_FLAGS_BASE;

/// The 23 functions that survive universe reconciliation, in
/// `fixtures/compiler-full/candidates.c` order. `enumerate_max` scopes the honest certificate:
/// Some(n) = exhaustive enumeration to the gcc length (optimality
/// proven in-universe, every shorter candidate rejected with a
/// witness); None = MCMC from the gcc baseline, with the measured
/// infeasibility reason in the comment (the core batch's discipline).
pub const FULL_FUNCS: &[CompilerFunc] = &[
    CompilerFunc {
        name: "turn_off_rightmost",
        source: "uint64_t turn_off_rightmost(uint64_t x) { return x & (x - 1); }",
        nargs: 1,
        enumerate_max: Some(2),
    },
    CompilerFunc {
        name: "p02_test_2n_minus_1",
        source: "uint64_t p02_test_2n_minus_1(uint64_t x) { return x & (x + 1); }",
        nargs: 1,
        enumerate_max: Some(2),
    },
    CompilerFunc {
        name: "lowest_set_bit",
        source: "uint64_t lowest_set_bit(uint64_t x) { return x & (-x); }",
        nargs: 1,
        enumerate_max: Some(3),
    },
    CompilerFunc {
        name: "p04_mask_lowest_ones",
        source: "uint64_t p04_mask_lowest_ones(uint64_t x) { return x ^ (x - 1); }",
        nargs: 1,
        enumerate_max: Some(2),
    },
    CompilerFunc {
        name: "p05_smear_rightmost",
        source: "uint64_t p05_smear_rightmost(uint64_t x) { return x | (x - 1); }",
        nargs: 1,
        enumerate_max: Some(2),
    },
    CompilerFunc {
        name: "p06_turn_on_rightmost0",
        source: "uint64_t p06_turn_on_rightmost0(uint64_t x) { return x | (x + 1); }",
        nargs: 1,
        enumerate_max: Some(2),
    },
    CompilerFunc {
        name: "p07_isolate_rightmost0",
        source: "uint64_t p07_isolate_rightmost0(uint64_t x) { return ~x & (x + 1); }",
        nargs: 1,
        // gcc's form is 4 instructions (mov/add/not/and). The core batch measured
        // 1-arg length-4 enumeration infeasible over the gcc universe
        // (iabs): the length-4 class is the length-3 class times a
        // ~10^3+ factor — the 1.08M-candidate L3 sweep becomes ~10^9+.
        // MCMC from the gcc baseline; honest no-improvement is a result.
        enumerate_max: None,
    },
    CompilerFunc {
        name: "p08_mask_trailing_zeros",
        source: "uint64_t p08_mask_trailing_zeros(uint64_t x) { return (x - 1) & ~x; }",
        nargs: 1,
        enumerate_max: Some(3),
    },
    CompilerFunc {
        name: "iabs",
        source: "int64_t iabs(int64_t x) { return (x ^ (x >> 63)) - (x >> 63); }",
        nargs: 1,
        // gcc's form is 5; 1-arg L4 measured infeasible (core batch). MCMC.
        enumerate_max: None,
    },
    CompilerFunc {
        name: "p13_sign",
        source: "uint64_t p13_sign(int64_t x) {\n    uint64_t s = (uint64_t)x;\n    uint64_t n = 0 - s;\n    return ((uint64_t)(x >> 63)) | (n >> 63);\n}",
        nargs: 1,
        // gcc's form is 5 (mov/sar/neg/shr/or); same measured L4+
        // infeasibility as iabs. MCMC.
        enumerate_max: None,
    },
    CompilerFunc {
        name: "avg",
        source: "uint64_t avg(uint64_t a, uint64_t b) { return (a & b) + ((a ^ b) >> 1); }",
        nargs: 2,
        // gcc's form is 5; 2-arg L4 measured infeasible (core batch: both
        // argument registers stay live through liveness pruning — a
        // 60-minute run on avg did not finish the L4 class).
        enumerate_max: None,
    },
    CompilerFunc {
        name: "p15_avg_ceil",
        source: "uint64_t p15_avg_ceil(uint64_t a, uint64_t b) { return (a | b) - ((a ^ b) >> 1); }",
        nargs: 2,
        // gcc's form is 5; same measured 2-arg L4 infeasibility as avg.
        enumerate_max: None,
    },
    CompilerFunc {
        name: "p16_umax_bitwise",
        source: "uint64_t p16_umax_bitwise(uint64_t x, uint64_t y) {\n    uint64_t m = 0 - (uint64_t)(x >= y);\n    return ((x ^ y) & m) ^ y;\n}",
        nargs: 2,
        // gcc recognizes the branchless max and emits a 3-instruction
        // cmov form: enumerate to 3 (the smin/umin/umax class, ~180K
        // candidates, seconds).
        enumerate_max: Some(3),
    },
    CompilerFunc {
        name: "p17_turn_off_run",
        source: "uint64_t p17_turn_off_run(uint64_t x) { return (((x | (x - 1)) + 1) & x); }",
        nargs: 1,
        // gcc's form is 4 (lea/or/add/and); 1-arg L4 measured
        // infeasible (see p07). MCMC.
        enumerate_max: None,
    },
    CompilerFunc {
        name: "p19_exchange_fields",
        source: "uint64_t p19_exchange_fields(uint64_t x, uint64_t m, uint64_t k) {\n    uint64_t o1 = x >> k;\n    uint64_t o2 = x ^ o1;\n    uint64_t o3 = o2 & m;\n    uint64_t o4 = o3 << k;\n    uint64_t o5 = o4 ^ o3;\n    return o5 ^ x;\n}",
        nargs: 3,
        // gcc's form is 9 including two %cl variable shifts (in the
        // subset since v0) and the salq alias (ADR-011). 3 live argument
        // registers: enumeration beyond length ~3 infeasible at 9-deep;
        // MCMC from the gcc baseline.
        enumerate_max: None,
    },
    CompilerFunc {
        name: "p23_popcnt_bytes",
        source: "uint64_t p23_popcnt_bytes(uint64_t x) {\n    uint64_t o1 = x >> 1;\n    uint64_t o2 = o1 & 0x5555555555555555;\n    uint64_t o3 = x - o2;\n    uint64_t o4 = o3 & 0x3333333333333333;\n    uint64_t o5 = (o3 >> 2) & 0x3333333333333333;\n    uint64_t o7 = o4 + o5;\n    uint64_t o8 = o7 >> 4;\n    uint64_t o9 = o8 + o7;\n    return o9 & 0x0F0F0F0F0F0F0F0F;\n}",
        nargs: 1,
        // gcc's form is 16. Enumeration to 16 is not remotely feasible;
        // MCMC from the gcc baseline.
        enumerate_max: None,
    },
    CompilerFunc {
        name: "next_pow2",
        source: "uint64_t next_pow2(uint64_t x) {\n    x = x - 1;\n    x |= x >> 1; x |= x >> 2; x |= x >> 4;\n    x |= x >> 8; x |= x >> 16; x |= x >> 32;\n    return x + 1;\n}",
        nargs: 1,
        // gcc's form is 20 (core batch). MCMC.
        enumerate_max: None,
    },
    CompilerFunc {
        name: "smin",
        source: "int64_t smin(int64_t a, int64_t b) { return a < b ? a : b; }",
        nargs: 2,
        enumerate_max: Some(3),
    },
    CompilerFunc {
        name: "umin",
        source: "uint64_t umin(uint64_t a, uint64_t b) { return a < b ? a : b; }",
        nargs: 2,
        enumerate_max: Some(3),
    },
    CompilerFunc {
        name: "umax",
        source: "uint64_t umax(uint64_t a, uint64_t b) { return a > b ? a : b; }",
        nargs: 2,
        enumerate_max: Some(3),
    },
    CompilerFunc {
        name: "sign_mask",
        source: "uint64_t sign_mask(int64_t x) { return (uint64_t)(x >> 63); }",
        nargs: 1,
        enumerate_max: Some(2),
    },
    CompilerFunc {
        name: "select_bits",
        source: "uint64_t select_bits(uint64_t x, uint64_t m, uint64_t y) { return (x & m) | (y & ~m); }",
        nargs: 3,
        // 3 live argument registers: L4 enumeration infeasible (core batch,
        // measured); gcc's form is 4. MCMC. The straggler pass gives this
        // straggler a documented second attempt (seeded synthesis).
        enumerate_max: None,
    },
    CompilerFunc {
        name: "sat_add",
        source: "uint64_t sat_add(uint64_t a, uint64_t b) {\n    uint64_t s = a + b;\n    return s < a ? UINT64_MAX : s;\n}",
        nargs: 2,
        enumerate_max: Some(3),
    },
];

/// The 8 candidates that did NOT survive reconciliation, with the
/// logged reason (ADR-010 discipline: no silent exclusions). The reason
/// cites the first opcode that actually fails to parse; every exclusion
/// is re-confirmed empirically per level against the committed assembly
/// (`exclusion_confirmed` events carry the observed parse error).
pub const FULL_EXCLUDED: &[(&str, &str)] = &[
    (
        "p10_nlz_eq",
        "emits `setnb %al` + `movzbl %al, %eax`: 8-bit partial-register operands, banned in v0 (ADR-001)",
    ),
    (
        "p11_nlz_lt",
        "emits `setb %al` + `movzbl %al, %eax`: 8-bit partial-register operands, banned in v0 (ADR-001)",
    ),
    (
        "p12_nlz_le",
        "emits `setnb %al` + `movzbl %al, %eax`: 8-bit partial-register operands, banned in v0 (ADR-001)",
    ),
    (
        "p18_is_pow2",
        "emits `testq` (flags-only AND-test, outside the v0 subset) plus `xorl %eax, %eax` and `sete %dl` partial-register ops",
    ),
    (
        "p20_snoob",
        "emits `xorl %edx, %edx` (32-bit zeroing idiom before divq): partial-register write, banned in v0 (ADR-001)",
    ),
    (
        "p21_cycle3",
        "emits `xorl %esi, %esi` (32-bit zeroing idiom) inside otherwise all-cmov code: banned in v0 (ADR-001)",
    ),
    (
        "p22_parity",
        "final `& 1` compiles to `andl $1, %eax`: 32-bit partial-register write, banned in v0 (ADR-001)",
    ),
    (
        "p25_mulhu",
        "gcc narrows the 32-bit parts to `movl %edi, %eax`-style 32-bit moves: banned in v0 (ADR-001)",
    ),
];

pub fn parse_survivors(asm_text: &str) -> Result<Vec<(&'static str, crate::instr::Program)>, String> {
    crate::compiler_core::parse_survivors_in(FULL_FUNCS, asm_text)
}

pub fn validate_exclusions(asm_text: &str) -> Vec<(&'static str, String)> {
    crate::compiler_core::validate_exclusions_in(FULL_EXCLUDED, asm_text)
}

pub fn harness_source(include_path: &str) -> String {
    crate::compiler_core::harness_source_for(FULL_FUNCS, include_path)
}
