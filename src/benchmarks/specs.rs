//! Benchmark suite for the superoptimizer. Specs are written as deliberately
//! naive straight-line x86-64 (the kind of code a compiler or a careful human
//! produces *before* superoptimizing); the tools must find provably-equivalent
//! cheaper programs.

use crate::instr::Gpr;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strategy {
    /// Exhaustive enumeration up to `max_length` with an optimality
    /// certificate (every cheaper candidate rejected with a witness).
    Enumerate { max_length: usize },
    /// STOKE-style stochastic search (synthesis + optimization phases).
    Mcmc,
}

pub struct Benchmark {
    pub name: &'static str,
    pub desc: &'static str,
    pub spec: &'static str,
    pub live_outs: &'static [Gpr],
    pub strategy: Strategy,
}

pub const BENCHMARKS: &[Benchmark] = &[
    Benchmark {
        name: "zero_flag",
        desc: "(x == 0) ? 1 : 0",
        spec: "cmpq $0, %rax\nsete %rcx",
        live_outs: &[Gpr::Rcx],
        strategy: Strategy::Enumerate { max_length: 2 },
    },
    Benchmark {
        name: "sign_mask",
        desc: "x < 0 ? -1 : 0",
        // NB: the original spec used `shl $63` (which extracts
        // bit 0, not bit 63) — the enumerator optimized the program as
        // written and exposed the desc/spec mismatch. The spec below really
        // computes the description.
        spec: "cmpq $0, %rax\nsetl %rcx\nmovq $-1, %rdx\nimulq %rdx, %rcx",
        live_outs: &[Gpr::Rcx],
        strategy: Strategy::Enumerate { max_length: 3 },
    },
    Benchmark {
        name: "mask_nonneg",
        desc: "x >= 0 ? -1 : 0",
        spec: "movq $-1, %rdx\ncmpq $0, %rax\nsetge %rcx\nimulq %rdx, %rcx",
        live_outs: &[Gpr::Rcx],
        strategy: Strategy::Enumerate { max_length: 3 },
    },
    Benchmark {
        name: "triple",
        desc: "x * 3",
        spec: "movq $3, %rcx\nimulq %rcx, %rax",
        live_outs: &[Gpr::Rax],
        strategy: Strategy::Enumerate { max_length: 2 },
    },
    Benchmark {
        name: "sum",
        desc: "a + b",
        spec: "movq %rax, %rcx\naddq %rbx, %rcx",
        live_outs: &[Gpr::Rcx],
        strategy: Strategy::Enumerate { max_length: 2 },
    },
    Benchmark {
        name: "min_u64",
        desc: "unsigned min(a, b)",
        spec: "movq %rbx, %rcx\ncmpq %rax, %rcx\ncmovbq %rcx, %rax",
        live_outs: &[Gpr::Rax],
        strategy: Strategy::Enumerate { max_length: 3 },
    },
    Benchmark {
        name: "abs",
        desc: "|x| (naive cmov form)",
        // Naive: y = x; if (x < 0) y = 0 - x.
        spec: "movq %rax, %rcx\nmovq $0, %rdx\nsubq %rax, %rdx\ncmpq $0, %rax\ncmovlq %rdx, %rcx",
        live_outs: &[Gpr::Rcx],
        strategy: Strategy::Mcmc,
    },
    Benchmark {
        name: "max_u64",
        desc: "unsigned max(a, b)",
        spec: "movq %rbx, %rcx\ncmpq %rax, %rcx\ncmovaq %rcx, %rax",
        live_outs: &[Gpr::Rax],
        strategy: Strategy::Mcmc,
    },
    Benchmark {
        name: "sext32",
        desc: "sign-extend the low 32 bits of x to 64",
        // NB: the spec is already optimal under this live-out contract —
        // the tempting 2-instruction form (`shlq $32, %rax; sarq $32, %rax`)
        // writes RAX, but the live-out here is RCX, so the 3-instruction
        // form (mov + shl + sar) is minimal. Kept deliberately: "no
        // improvement" results that are *correct* behavior are part of
        // the suite's honesty.
        spec: "movq %rax, %rcx\nshlq $32, %rcx\nsarq $32, %rcx",
        live_outs: &[Gpr::Rcx],
        strategy: Strategy::Mcmc,
    },
    Benchmark {
        name: "avg_floor",
        desc: "overflow-safe average (naive carry form)",
        // Naive: s = a+b; c = s < a; return (s >> 1) | (c << 63).
        spec: "movq %rax, %rcx\naddq %rbx, %rcx\nmovq %rax, %rdx\ncmpq %rcx, %rdx\nsetb %rdx\nshlq $63, %rdx\nshrq $1, %rcx\norq %rdx, %rcx",
        live_outs: &[Gpr::Rcx],
        strategy: Strategy::Mcmc,
    },
    Benchmark {
        name: "popcount8",
        desc: "population count of the low byte (naive shift-and-add)",
        // Naive: sum the bits one at a time (29 instructions).
        spec: "movq %rax, %rcx\nandq $1, %rcx\nshrq $1, %rax\nmovq %rax, %rdx\nandq $1, %rdx\naddq %rdx, %rcx\nshrq $1, %rax\nmovq %rax, %rdx\nandq $1, %rdx\naddq %rdx, %rcx\nshrq $1, %rax\nmovq %rax, %rdx\nandq $1, %rdx\naddq %rdx, %rcx\nshrq $1, %rax\nmovq %rax, %rdx\nandq $1, %rdx\naddq %rdx, %rcx\nshrq $1, %rax\nmovq %rax, %rdx\nandq $1, %rdx\naddq %rdx, %rcx\nshrq $1, %rax\nmovq %rax, %rdx\nandq $1, %rdx\naddq %rdx, %rcx\nshrq $1, %rax\nmovq %rax, %rdx\nandq $1, %rdx\naddq %rdx, %rcx\nshrq $1, %rax\nandq $1, %rax\naddq %rax, %rcx",
        live_outs: &[Gpr::Rcx],
        strategy: Strategy::Mcmc,
    },
];

pub fn by_name(name: &str) -> Option<&'static Benchmark> {
    BENCHMARKS.iter().find(|b| b.name == name)
}

/// A borrowed view of a benchmark — lets the static `BENCHMARKS` table and
/// the compiler-baseline batch's dynamically-parsed output share the run
/// functions.
pub struct AnyBenchmark<'a> {
    pub name: &'a str,
    pub desc: &'a str,
    pub spec: &'a str,
    pub live_outs: &'a [Gpr],
    pub strategy: Strategy,
}

impl<'a> From<&'a Benchmark> for AnyBenchmark<'a> {
    fn from(b: &'a Benchmark) -> Self {
        AnyBenchmark {
            name: b.name,
            desc: b.desc,
            spec: b.spec,
            live_outs: b.live_outs,
            strategy: b.strategy,
        }
    }
}

/// A dynamic benchmark built at runtime (compiler-baseline output).
pub struct DynBenchmark {
    pub name: String,
    pub desc: String,
    pub spec: String,
    pub live_outs: Vec<Gpr>,
    pub strategy: Strategy,
}

impl DynBenchmark {
    pub fn view(&self) -> AnyBenchmark<'_> {
        AnyBenchmark {
            name: &self.name,
            desc: &self.desc,
            spec: &self.spec,
            live_outs: &self.live_outs,
            strategy: self.strategy,
        }
    }
}
