# Isuperoptimizer

A certificate-first superoptimizer for a curated subset of x86-64. Given a
short straight-line function, it searches for a shorter or cheaper
instruction sequence and **proves it equivalent with an SMT solver (z3)** in
a declared universe, with machine-checkable certificates.

```
$ isuperopt optimize --bench abs
  PASS abs: spec 5 instr (perf 5) -> 3 instr (perf 3) via optimization:
    leaq -1(%rax), %rcx
    xorq $0xffffffffffffffff, %rcx
    cmovlq %rax, %rcx
```

That is `|x|` in three instructions, one shorter than the textbook
four-instruction `sar` form, discovered by stochastic search. The artifact
records z3's proof of equivalence, so the claim is checkable by anyone.

## Components

- **Semantics** (`src/semantics/`): a total-semantics model of a curated
  64-bit x86-64 subset (`mov`, `movabs`, ALU and shift forms, `lea`,
  `setcc`/`cmovcc`, `neg`, `not`, two-operand `imul`, one-operand
  `div`/`idiv`, and load/store over a 256-byte sandbox), with per-flag
  definedness. Partial-register writes are rejected at parse time.
- **Verifier** (`src/verifier/`): the miter
  `∀s0. def_T(s0) → def_R(s0) ∧ outs_T(s0) = outs_R(s0)`, emitted as
  QF_AUFBV SMT-LIB2 to a persistent z3 process. A satisfiable model becomes
  a concretely replayable test case.
- **Enumerator** (`src/enumeration/`): exhaustive backward-liveness-pruned
  search that emits optimality certificates. Every shorter candidate is
  rejected with a logged witness.
- **Search** (`src/search/`): STOKE-style MCMC with the published move set
  and hyperparameters, plus synthesis and optimization phases.
- **Artifact pipeline** (`src/artifact/`): every run writes an append-only
  JSONL artifact of record. `recheck --from-log` re-derives every
  concretely checkable claim from the log alone, and
  `tools/audit_claims.py` re-derives the headline numbers.
- **Compiler comparison** (`src/benchmarks/`): gcc baselines, the SysV
  AMD64 ABI mapping, the compiled-binary oracle, and the two benchmark
  batches (core and full provenance).

## Results

The hand-written suite holds eight verified certificates from eleven
attempts (`artifacts/optimize-certificates.jsonl`):

| benchmark | spec → optimized | method | evidence |
|---|---|---|---|
| zero_flag | 2 → 2 (`and %rax,%rax; sete %rcx`, proof no 1-instruction program exists) | enumeration | `optimize-certificates.jsonl` |
| sign_mask | 4 → 2 | enumeration | ″ |
| mask_nonneg | 4 → 3 (`shr $63; sub $1`, xor-free complement) | enumeration | ″ |
| triple | 2 → 1 (`lea`) | enumeration | ″ |
| sum | 2 → 1 (`lea`) | enumeration | ″ |
| min_u64 | 3 → 2 | enumeration | ″ |
| **abs** | **5 → 3** (`lea -1; xor $-1; cmovl`, beats the textbook 4) | MCMC | ″ |
| max_u64 | 3 → 2 | MCMC | ″ |

Three more ran honestly to budget (`sext32`, `avg_floor`, `popcount8`).

The compiler comparison covers the full Gulwani/STOKE provenance: all 25
Hacker's Delight programs, ported to 64-bit C, with 23 survivors after
universe reconciliation. Both `-O2` and `-O3` are logged and challenged
separately (46 baselines), even though gcc emitted identical assembly at
both levels for every function.

**gcc is beaten on 3 of 23 functions**:

- `iabs` 5 → 3 (`andq $0; subq %rdi; cmovleq`)
- `p13_sign` 5 → 4 (`orq $-1; addq %rdi,%rdi; seta; cmovbq`)
- `p19_exchange_fields` 9 → 8

**13 more are matched with exhaustive optimality proofs** (26 optima,
14,216,086 rejected candidates), and **7 survive honestly**, including
three stragglers after a seeded-synthesis second attempt at five times the
primary per-chain budget. Every claimed program is x86-encodable and
ABI-scratch-clean; the audit enforces both properties on every claim.

Evidence: `artifacts/compiler-comparison-full.jsonl`. Live recheck:
64,120 events, 0 failures.

## Verification

The verification pyramid has seven layers, and no layer trusts another:
unit and corner tests, a two-million-program concrete-vs-symbolic
differential, an 8,000-query z3-sampled differential, a 5,344-program
self-verification grid, hand-picked equivalence and broken pairs, a
hardware oracle over real silicon, and artifact re-derivation.

The committed record is nine JSONL artifacts. A full build re-verifies
**114,295 artifact events with 0 failures**, passes **485 claims-audit
checks**, and confirms all **45 bibliography entries** in the paper. The
defect ledger has 34 rows; every row records what caught the defect and
why the other layers could not.

See `docs/VERIFICATION.md` for the layers, the complete ledger, and the
per-artifact recheck totals.

## Repository layout

```
artifacts/              nine JSONL artifacts of record
docs/
  adr/                  architecture decision records (001-011)
  VERIFICATION.md       verification layers and the defect ledger
fixtures/
  compiler-core/        committed gcc -S output for the core batch
  compiler-full/        committed gcc -S output for the full batch
paper/                  IEEEtran paper and its traceability build
src/
  semantics/            AST, parser, concrete and symbolic state, cost
  verifier/             miter and z3 process
  enumeration/          exhaustive search and certificates
  search/               MCMC, RNG, fuzz generators
  artifact/             JSONL artifact of record
  benchmarks/           hand-written suite and compiler batch registries
  bin/                  isuperopt, recheck, stress
tests/                  acceptance, enumeration, search, division/memory,
                        compiler core, compiler full
tools/                  audit_claims.py, hw_oracle.c
```

## Build and verify

Dependencies: Rust (stable), the `z3` binary, and `cc` for the hardware
oracle. The crate has zero Rust dependencies by design (`docs/adr/ADR-005-smt-backend.md`).
z3 is located through `ISUPEROPT_Z3` or `PATH`.

```sh
cargo build --release
cargo test                                    # 66 tests

# re-derive every concretely checkable claim from one artifact:
./target/release/recheck --from-log artifacts/compiler-comparison-full.jsonl

# re-derive the headline numbers from all artifacts:
python3 tools/audit_claims.py

# regenerate and gate the paper (recheck, audit, bibliography,
# extraction, compile, house-style):
cd paper && ./build.sh
```

## Scope and honesty

- "Optimal" means *within a declared universe* (opcode alphabet, operand
  sets, length bound, all logged). No global claims are made.
- Equivalence is the solver's verdict. `recheck` validates everything
  around it (witnesses, test cases, cost zero on committed test sets) and
  never pretends to concretely check an equivalence.
- Definedness is part of every claim: rewrites that introduce undefined
  behavior are rejected (`def_T → def_R`), and a target that is undefined
  everywhere verifies vacuously, as documented.
- Division and memory are covered by the full pyramid, including the
  hardware oracle; search generators stay register-only and log that
  restriction.
- One compiler, one machine: the comparison is against gcc 15.2.0 on
  x86-64 Linux and would need re-running per compiler release.

## Documentation

- `docs/adr/` — architecture decision records, indexed by component.
- `docs/VERIFICATION.md` — the seven layers, the 34-row defect ledger,
  and the per-artifact recheck totals.
- `paper/README.md` — the paper's build pipeline and typography.

Author: Imtamum Hosen Inan.
