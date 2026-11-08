//! Compiler-baseline registry for the core batch: survivor functions,
//! excluded functions with logged reasons, the SysV AMD64 ABI mapping, and
//! the compiled-binary oracle harness generation (ADR-010).

use crate::instr::{parse_gcc_asm, Gpr, Program};

/// Flags used for every gcc invocation in this batch (both -S and
/// harness builds); logged in every `compiler_baseline` event.
pub const GCC_FLAGS_BASE: &str = "-m64 -fno-tree-vectorize -fno-tree-slp-vectorize";

/// A survivor function: the C source (verbatim from
/// `fixtures/compiler-core/candidates.c`), its argument count, and its
/// search strategy.
pub struct CompilerFunc {
    pub name: &'static str,
    pub source: &'static str,
    pub nargs: usize,
    /// enumerate up to this length (spec_len) when small; None = MCMC only
    pub enumerate_max: Option<usize>,
}

pub const CORE_FUNCS: &[CompilerFunc] = &[
    CompilerFunc {
        name: "turn_off_rightmost",
        source: "uint64_t turn_off_rightmost(uint64_t x) { return x & (x - 1); }",
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
        name: "avg",
        source: "uint64_t avg(uint64_t a, uint64_t b) { return (a & b) + ((a ^ b) >> 1); }",
        nargs: 2,
        // gcc's baseline is 5; exhaustive enumeration beyond length 3 is
        // infeasible over the 2-arg gcc universe (liveness keeps both RDI
        // and RSI live -> the length-4 class alone is ~10^10 candidates;
        // measured, not guessed — a 60-minute run did not finish it).
        // Scope: MCMC from the gcc baseline, honest no-improvement if it
        // survives the budget.
        enumerate_max: None,
    },
    CompilerFunc {
        name: "iabs",
        source: "int64_t iabs(int64_t x) { return (x ^ (x >> 63)) - (x >> 63); }",
        nargs: 1,
        // 1-arg: enumeration to the gcc length (4: gcc folds the source's
        // sar/xor/sub into 4) proved infeasible at 5; length 4 for 1-arg is
        // the same order as lowest_set_bit's length 3 times the length-4
        // factor — measured infeasible in the first beat run too. MCMC.
        enumerate_max: None,
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
        // 3 live argument registers: length-4 enumeration infeasible (same
        // measured reason as avg). MCMC from the gcc baseline.
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
        name: "sat_add",
        source: "uint64_t sat_add(uint64_t a, uint64_t b) {\n    uint64_t s = a + b;\n    return s < a ? UINT64_MAX : s;\n}",
        nargs: 2,
        enumerate_max: Some(3),
    },
    CompilerFunc {
        name: "next_pow2",
        source: "uint64_t next_pow2(uint64_t x) {\n    x = x - 1;\n    x |= x >> 1; x |= x >> 2; x |= x >> 4;\n    x |= x >> 8; x |= x >> 16; x |= x >> 32;\n    return x + 1;\n}",
        nargs: 1,
        enumerate_max: None, // 20 instructions at -O2/-O3: MCMC territory
    },
];

/// Every candidate that did NOT survive universe reconciliation, with the
/// logged reason (ADR-010: no silent exclusions). Reasons cite the specific
/// opcodes in the emitted assembly.
pub const CORE_EXCLUDED: &[(&str, &str)] = &[
    ("cycle3", "emits `je` (branch): control flow outside the straight-line model"),
    ("parity8", "emits `andl`/`movzbl`: 32-bit and 8-bit partial-register writes, banned in v0 (ADR-001)"),
    ("popcount8", "emits `andl`/`movzbl`: 32-bit and 8-bit partial-register writes, banned in v0 (ADR-001)"),
    ("ult_flag", "emits `xorl`: 32-bit partial-register zeroing idiom, banned in v0 (ADR-001)"),
    ("ctz64", "emits `rep bsf` (tzcnt) and `cltq`/`xorl`: unsupported opcode + partial-register writes"),
    ("bswap64", "emits `bswap`: opcode outside the v0 subset"),
    ("div7", "emits `movabsq`+`mulq` (one-operand, RDX:RAX): the 128-bit mul opcode is outside the v0 subset"),
    ("avg2", "emits `andl`: 32-bit partial-register writes, banned in v0 (ADR-001)"),
];

/// SysV AMD64 ABI mapping: argument i lands in these registers, results
/// come back in RAX.
pub fn arg_reg(i: usize) -> Gpr {
    match i {
        0 => Gpr::Rdi,
        1 => Gpr::Rsi,
        2 => Gpr::Rdx,
        3 => Gpr::Rcx,
        4 => Gpr::R8,
        _ => Gpr::R9,
    }
}

pub const RET_REG: Gpr = Gpr::Rax;

/// Build the harness C source for the compiled-binary oracle: includes the
/// committed candidates.c verbatim (#include "fixtures/compiler-core/candidates.c") and adds
/// a dispatcher that reads `name arg0 arg1 arg2` (hex) lines from stdin and
/// prints the function's return value (hex) on stdout.
pub fn harness_source(include_path: &str) -> String {
    harness_source_for(CORE_FUNCS, include_path)
}

/// Batch-parameterized harness generator: dispatches over any registry
/// of survivor functions. The full-provenance batch's survivors take at
/// most 3 ABI arguments, so the 3-argument line protocol still suffices.
pub fn harness_source_for(funcs: &[CompilerFunc], include_path: &str) -> String {
    let mut dispatch = String::new();
    for f in funcs {
        let mut args = String::new();
        let mut call_args = String::new();
        for i in 0..f.nargs {
            args.push_str(&format!("a{i}, "));
            call_args.push_str(&format!("a{i}, "));
        }
        let call_args = call_args.trim_end_matches(", ");
        dispatch.push_str(&format!(
            "        if (!strcmp(name, \"{name}\")) {{ printf(\"%llx\\n\", (unsigned long long){name}({call_args})); fflush(stdout); continue; }}\n",
            name = f.name,
            call_args = call_args
        ));
    }
    // candidates.c is included first, so every function is already defined
    // before main — no forward declarations (they would conflict with the
    // uint64_t/int64_t signatures).
    format!(
        "#include <stdint.h>\n#include <stdio.h>\n#include <string.h>\n#include \"{include_path}\"\n\nint main(void) {{\n    char name[64];\n    unsigned long long a0, a1, a2;\n    while (scanf(\"%63s %llx %llx %llx\", name, &a0, &a1, &a2) == 4) {{\n{dispatch}        fprintf(stderr, \"unknown function %s\\n\", name);\n        return 2;\n    }}\n    return 0;\n}}\n",
        include_path = include_path,
        dispatch = dispatch,
    )
}

/// Split a `gcc -S` file into (name, chunk) pairs by top-level function
/// label. Chunk includes the label and everything up to (and including) the
/// next function label — parse_gcc_asm is then run per chunk so that one
/// excluded function cannot poison the survivors.
pub fn split_asm_functions(text: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut cur: Option<(String, Vec<&str>)> = None;
    for line in text.lines() {
        let t = line.trim();
        // Chunk-boundary rule: close the chunk at gcc's `.LFEn:` function-end
        // marker. Assembly files end with a `.note.gnu.property` trailer
        // whose numeric labels (`0:`, `1:`) previously landed inside the
        // LAST function's chunk, where parse_gcc_asm correctly rejects
        // them ("label after ret") — a false exclusion when the last
        // function is a survivor (the full batch's sat_add). No instruction of any
        // function ever follows its `.LFE`.
        if t.starts_with(".LFE") && t.ends_with(':') {
            if let Some((name, body)) = cur.take() {
                out.push((name, body.join("\n")));
            }
            continue;
        }
        if let Some(label) = t.strip_suffix(':') {
            if !label.starts_with('.') && !t.contains('(') {
                // Chunk-boundary rule (2): only identifier-shaped labels open chunks.
                // gcc function symbols are C identifiers; the GNU-notes
                // trailer uses numeric local labels (`0:`, `1:`) which must
                // not become phantom chunks.
                if !label.chars().next().map_or(false, |c| c.is_ascii_alphabetic() || c == '_') {
                    continue;
                }
                if let Some((name, body)) = cur.take() {
                    out.push((name, body.join("\n")));
                }
                cur = Some((label.to_string(), vec![line]));
                continue;
            }
        }
        if let Some((_, ref mut body)) = cur {
            body.push(line);
        }
    }
    if let Some((name, body)) = cur {
        out.push((name, body.join("\n")));
    }
    out
}

/// Parse a committed gcc assembly file and return the survivor functions'
/// programs, keyed by name. Parses each function chunk in isolation; errors
/// loudly on any reconciliation surprise.
pub fn parse_survivors(asm_text: &str) -> Result<Vec<(&'static str, Program)>, String> {
    parse_survivors_in(CORE_FUNCS, asm_text)
}

/// Batch-parameterized survivor parser: parse the committed gcc
/// assembly for any registry, erroring loudly on reconciliation surprises.
pub fn parse_survivors_in(
    funcs: &[CompilerFunc],
    asm_text: &str,
) -> Result<Vec<(&'static str, Program)>, String> {
    let chunks = split_asm_functions(asm_text);
    let mut out = Vec::new();
    for f in funcs {
        let chunk = chunks
            .iter()
            .find(|(name, _)| name == f.name)
            .ok_or_else(|| format!("function `{}` missing from gcc assembly", f.name))?;
        let funcs = parse_gcc_asm(&chunk.1).map_err(|e| e.to_string())?;
        let (_, prog) = funcs
            .first()
            .ok_or_else(|| format!("no parse result for `{}`", f.name))?;
        out.push((f.name, prog.clone()));
    }
    Ok(out)
}

/// Empirically confirm the logged exclusion reasons: every excluded
/// function's chunk must FAIL to parse (with the stated opcode/label as
/// the cause). Returns (function, observed error) pairs.
pub fn validate_exclusions(asm_text: &str) -> Vec<(&'static str, String)> {
    validate_exclusions_in(CORE_EXCLUDED, asm_text)
}

/// Batch-parameterized exclusion validator: empirically confirm
/// every logged exclusion reason against the committed assembly.
pub fn validate_exclusions_in(
    excluded: &[(&'static str, &'static str)],
    asm_text: &str,
) -> Vec<(&'static str, String)> {
    let chunks = split_asm_functions(asm_text);
    let mut out = Vec::new();
    for (name, reason) in excluded {
        match chunks.iter().find(|(n, _)| n == name) {
            None => out.push((*name, "MISSING FROM ASSEMBLY".to_string())),
            Some((_, chunk)) => match parse_gcc_asm(chunk) {
                Ok(fs) => {
                    // parsed when it should not have — report what got in
                    let n = fs.first().map(|f| f.1 .0.len()).unwrap_or(0);
                    out.push((name, format!("UNEXPECTEDLY PARSED ({} slots) — exclusion reason wrong", n)));
                }
                Err(e) => out.push((name, e.message)),
            },
        }
        let _ = reason;
    }
    out
}
