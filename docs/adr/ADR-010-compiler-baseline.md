# ADR-010: The compiler baseline, universe reconciliation, and the model corrections it forced

## Status
Accepted

## Context
Earlier phases validated everything against hand-written naive specs. This
phase takes real `gcc -O2/-O3` output as the baseline: prove equivalence, try to
beat it, log honestly. That exposed three things this ADR records: the
opcode-reconciliation decision, the gcc-assembly transcription layer, and
**two soundness corrections to the model itself** that only became visible
against compiler output.

## Baseline decisions

- **Compiler**: gcc 15.2.0 (Ubuntu), flags `-O2`/`-O3` separately, always
  `-m64 -fno-tree-vectorize -fno-tree-slp-vectorize` (SLP vectorization is
  enabled at -O2 in GCC 15 — verified with `-Q --help=optimizers`; the
  flags keep output inside the 64-bit-GPR straight-line subset). clang was
  not part of the recorded toolchain.
- **ABI**: System V AMD64 — integer args in RDI, RSI, RDX, RCX, R8, R9;
  return in RAX (multiple sources, incl. the Oracle/SysV ABI docs).
  All core-batch candidates use `uint64_t`/`int64_t` exclusively.
- **Benchmark provenance**: Hacker's Delight functions via Gulwani's
  25-program benchmark (the set STOKE used) and STOKE's own reported wins
  (cycle-through-3-values, compute-parity, next-power-of-2).

## Universe reconciliation

Compiled 18 candidate functions at -O2 and -O3; vocabulary diff against
ADR-001 found: **`negq` in-subset-extendable** (adopted, with the full treatment
below), and **gaps** — `andl/xorl/movzbl/setb/cltq` (partial-
register writes: excluded, ADR-001), `bswap`, `mulq` (one-operand RDX:RAX),
`rep bsf`, `je` (control flow): excluded with logged reasons.
**8 functions excluded, 11 survived.** No silent exclusions: every
exclusion is (a) written into `compiler_core::CORE_EXCLUDED`, (b) confirmed
*empirically* per level — the committed assembly for each excluded
function must fail to parse, and the observed parse error is logged in an
`exclusion_confirmed` event.

### The `negq` extension (full treatment, no shortcuts)
Concrete + symbolic + effects/liveness + enumeration alphabet + MCMC move
set (latency 1, singleton class) + fuzz generators + hardware oracle
(`neg` op, flags compared) + hand pairs (`neg ≡ 0 - x` via sub-from-zero;
broken: `neg` vs `xor $-1`, the NOT/NEG confusion) + grid entry +
`written_regs`/`strip_dead` plumbing. SDM semantics: result = 0 - dst,
CF = (dst ≠ 0), OF = (dst = INT64_MIN), ZF/SF from result.

### Syntax acceptance (not semantics changes)
- Unsuffixed `cmovcc` with 64-bit register operands is the 64-bit form
  (gas infers size from operands); condition-code aliases (z/nz/nb/nc/na/
  nbe/nle/nl/nge/ng/c) normalize to the canonical codes — same
  instructions, already hardware-oracle-validated.
- One-operand `shlq/shrq/sarq %r` is shift-by-1 (opcode D1).
- `movabsq` = `movq` with a full-width immediate.

## The compiled-binary oracle

New stress mode `compiler_oracle`: for each survivor function and each
level, a harness binary is built from the *committed* `fixtures/compiler-core/candidates.c`
with the exact logged flags; the harness calls the real function through
the real ABI on corner + random inputs; the result is compared against
this project's concrete evaluator running the *parsed* assembly
(`parse_gcc_asm` transcription). This is the layer that makes "this is
what gcc emits" a checked claim: the recurring failure shape "the fold is
right, the printer/parser is wrong" (VERIFICATION.md rows 3, 13, 14)
applies to the new transcription step too. 44,000 ABI calls
(11 functions × 2 levels × 2,000), 0 discrepancies.

`parse_gcc_asm` rules (nothing silently dropped): dot-directives skipped;
`endbr64` skipped (CET landing pad, no architectural state effect — and
the oracle validates the resulting behavior empirically); `.LFB/.LFE`
frame markers skipped (unconditional, not jump targets); any other `.L`
label is a hard error (control flow); exactly one function label; a
single trailing `ret`/`rep ret` ends the function (the call/return
protocol is outside the straight-line model — dropping it is the defined
function→program transformation); everything else must parse as a subset
instruction.

## The two model corrections (found by the compiler-baseline phase, fixed with the full pyramid)

### 1. Entry flags are UNDEFINED (supersedes the earlier convention)
Running gcc output through the pipeline surfaced a candidate optimum that
*began* with `sete %rax` — reading RFLAGS at function entry. Per the SysV
ABI, incoming flags are caller-clobbered; gcc never reads them on entry.
The old model (entry flags defined-false) let flag-reads-at-entry produce
fake constants — sound only against hand-written specs that also never
read entry flags. **Fix:** `Flags::fresh()` is now all-undefined (values
canonical false), mirrored in `SFlags::fresh()`; `SFlags::defined()` is
the spread for flag-*writing* instructions (the audit found the symbolic
arms had been spreading `fresh()` — a differential catch). Effects:
- any candidate reading entry flags is rejected via `def_R` — a new
  `#[test]` pins that the earlier `setb`-at-entry |x| form is now
  REJECTED while the legitimate `leaq -1; xorq $-1; cmovl` form stands;
- the optimize artifact was regenerated (its `setb`-abs optimum was
  equivalent-in-old-model only);
- enumeration/search headline results were unaffected (none read entry flags;
  enumeration optima identical, re-verified).

### 2. The identity floor: the spec's own forms must be in its universe
The first enumeration against gcc output FAILED to find any equivalent of
length ≤ 2 for a 2-instruction gcc function — because gcc's
`leaq -1(%rdi), %rax` uses a lea form outside the enumeration universe.
"No program of length < L exists" must never be an artifact of the spec
falling outside its own universe. **Fix:** `EnumConfig::for_spec` unions
the spec's own immediates, shift counts, and lea addressing modes into the
universe (declared, as always, in the `enumeration_universe` event). The
enumeration artifact was regenerated once more when `Neg` joined the alphabet:
optima unchanged, counts grew (119,680 → 278,377 candidates, certificates
strictly stronger); `for_benchmark` still pins the enumeration universe shape
so those artifacts reproduce deterministically.

## Consequences
"Beats the compiler" claims are now meaningful in both directions: the
baseline is the checked, logged, oracle-validated gcc output, and the
model has no phantom entry-state for search to exploit.
