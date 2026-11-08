# ADR-011: Scaling the compiler benchmark to the full Gulwani/STOKE provenance

## Status
Accepted

## Context
The core batch's "beats the compiler" rested on 1 win out of 11 survivors,
a real result but an anecdote. This phase scales the comparison to the full
benchmark provenance ADR-010 cites: the 25 programs of Gulwani/Jha/Tiwari/Venkatesan
(PLDI 2011, Figure 4 — the Hacker's Delight set STOKE ran; ASPLOS'13 §6.1
confirms the same p01–p25 and reports its own wins on p18/p21/p22/p25 and
synthesis timeouts on p19/p20/p24), and gives the three core-batch stragglers
(avg, select_bits, next_pow2) a documented second attempt.

## Program provenance

The 25 programs were transcribed from the PLDI 2011 paper's Figure 4.
Porting decisions, each documented in
`fixtures/compiler-full/candidates.c`:

- **Word-size scaling.** The four 32-bit-constant programs are ported by
  doubling the word size exactly as the core batch's `next_pow2` already did:
  P22's parity multiply trick uses `0x1111111111111111` and `>> 60`
  (verified: computes 64-bit popcount-parity), P23 uses the doubled
  SWAR constants (verified: per-byte popcounts of all 64 bits), P25
  uses 32-bit parts of 64-bit inputs (verified against `(x*y)>>64`
  including `x=y=2^64-1`), P13/P9 shift by 63.
- **Two mis-transcriptions in the paper, fixed per HD and documented.**
  P20's line 4 prints `x ^ lowest-set-bit` where HD's snoob uses
  `x ^ ripple`; the printed form returns 26 for x=22 where snoob(22)=25
  (verified against brute force). P23's line 6 duplicates its line 4
  (`o3 & 0x33333333`) where HD's method uses `(o3 >> 2) & 0x33333333`.
  Both follow HD — the stated source of the whole benchmark — with the
  deviation written into the committed C and the registry.
- Every port was verified in Phase 0 against an independent reference
  implementation before compilation (25 identities over random +
  corner inputs; P13 checked at INT64_MIN, P25 at 2^64-1 products).

## Universe reconciliation

All 31 functions (25 P-programs + the 6 core-batch extras; the 5 P-programs
already in the core batch keep their committed sources byte-identical) compiled at
-O2 and -O3 with the ADR-010 flags. **gcc 15.2.0 emits identical code at
both levels for the entire batch** (diff-verified; both levels still get
separate logged baselines and separate challenges).

**23 survivors, 8 exclusions.** The exclusions, each confirmed by the
actual parse error per level (`exclusion_confirmed` events):
p10/p11/p12 (setcc-on-%al + movzbl), p18 (testq + xorl + sete %dl),
p20 (xorl %edx before divq), p21 (xorl %esi inside otherwise-all-cmov
code — the same idiom that excluded ult_flag/avg2 in the core batch), p22
(andl $1 on %eax for the final `& 1`), p25 (movl-narrowed 32-bit parts).

### The `notq` extension (full pyramid, no exceptions)

P7/P8 (`~x`) need NOT. Adopted with the complete treatment, mirroring
negq (ADR-010): concrete + symbolic + effects/liveness (writes dst,
**touches no flags** — SDM: NOT, like MOV/LEA, performs no flag update) +
enumeration alphabet + MCMC opcode/mutator + latency 1 singleton class +
fuzz generators + grid entry + hand pairs (`notq ≡ xorq $-1`, and the
core-batch broken pair's xor stand-in replaced by the real opcode contrast)
+ **hardware oracle `not` op with a flag-preset protocol**: the handler
executes `subq` then `notq` and the model evaluates the same sub, so all
four flags are compared after the NOT — validating flags-untouched on
silicon, not just the register result.

### Syntax acceptance (no semantics changes), per the ADR-010 precedent
- `salq`/`sal` — the historical synonym of `shlq` (identical opcode;
  gcc emits it for P19's variable left shift). Normalizes to `Shl`.
- `movabsq` — **accepted for real this time** (see VERIFICATION.md
  row 29): ADR-010 claimed `movabsq = movq` with full-width immediate,
  but the opcode was never in the parser and the core-batch pinning test's
  `unwrap_or_else` fallback made it self-neutralizing. The full batch's p23
  emits movabsq and failed to parse. Fixed: imm64→reg form only, and
  the test now pins it without the fallback.

### The encodability pass (VERIFICATION.md row 32)

The first full-provenance run claimed p23 16→14 — but three of the winner's
forms were `andq $imm64`, which real x86 cannot encode (ALU immediates
are sign-extended imm32 only; that is exactly why gcc materialized those
constants via movabsq). The v0 parser's doc said "sign-extended imm32 or
movabs imm64" but accepted any u64 on every ALU op, so the search had
forms gcc's output structurally cannot contain. The fix is two-layered:

- **The searched subset is encodable-restricted.** The enumeration
  alphabet carries immediates on ALU forms only when imm32-encodable
  (`instr::imm32_encodable`); wide constants remain on Mov (movabsq).
  The MCMC generators and every mutation path that can place an
  immediate on an ALU slot (synthesis draws, instruction/operand moves,
  and the opcode-move carry from Mov into an ALU op) resample through
  `paper_imm_alu`. This is a **documented deviation from STOKE's
  unrestricted immediate generator** (ADR-007), logged in every
  `mcmc_config` event's `immediate_policy`.
- **The model/parser stays permissive.** The evaluator and parser still
  accept wide ALU immediates as a model construct, because committed
  artifacts' rejection witnesses replay exactly such candidates —
  tightening the parser would retroactively break their re-checkability.
  Pinned by test; the layer split (model vs searched subset) is the
  documented contract.

the enumeration artifact was regenerated once more for the alphabet change (279,088 →
249,223 candidates, optima byte-identical); the certificate now reads
"no shorter *encodable* program exists" — the claim the compiler
comparison actually wants. All previously committed claimed programs
(the enumeration, search, and optimize artifacts' optima and search results) were scanned and are
encodable-clean; only rejected-candidate witnesses and dead-code
synthesis padding contain wide forms, and both still replay.

### The ABI register contract (VERIFICATION.md row 33)

The second full run then claimed p13_sign 5→4 — a real four-instruction
sign function (`negq; setl; movq $-1, %rsp; cmovgq`) whose constant was
materialized **in %rsp**. In-model perfectly sound (all 16 GPRs are
equal model citizens; live-out was RAX; z3 proved the equivalence), but
a real function body that sets RSP=−1 returns through garbage — not a
drop-in replacement. "Equivalent, live-out RAX" and "equivalent
function" diverge exactly when a candidate writes a callee-saved or
stack register. The fix makes them coincide by construction:

- every register the MCMC can pick (sources, destinations, addressing)
  is caller-clobbered only: RAX, RCX, RDX, RSI, RDI, R8–R11;
- the enumeration destination set for gcc specs drops RBX (callee-saved);
- the identity floor now also unions the spec's write-registers
  (p19's gcc form writes %r8), keeping the floor total;
- `for_benchmark` keeps its historical pin (including RBX) — the enumeration/optimize artifacts
  certify in-model benchmark optima, not drop-ins, and their committed
  artifacts reproduce unchanged.

The p13 *algorithm* stays reachable (any scratch register holds the
constant); the run was re-adjudicated a third time so every claim that
survives is drop-in-clean. the enumeration artifact regenerated for the narrower universe
(249,223 → 218,955 candidates, optima byte-identical yet again).

### Parser-layer fix (found by the full batch)
Assembly files end with a `.note.gnu.property` trailer whose numeric
labels (`0:`, `1:`) previously landed inside the LAST function's chunk,
where `parse_gcc_asm` correctly rejects them ("label after ret") — a
false exclusion when the last function is a survivor (the full batch's sat_add).
`split_asm_functions` now closes each chunk at gcc's `.LFE` function-end
marker and only opens chunks on identifier-shaped labels. Core-batch
behavior is unchanged (its last chunk was the excluded avg2, whose
observed andl error precedes any trailer line).

## The full suite
Every survivor through enumerate-then-search at both levels
(`artifacts/compiler-comparison-full.jsonl`): 13 functions are enumerate-scoped to the
gcc length (optimality proven in-universe), 10 are MCMC-scoped with the
measured infeasibility reason in `src/benchmarks/compiler_full.rs` (1-arg length-4 and 2/3-arg
length-4+ classes are ~10^9+ candidates — core-batch measurements; the
identity floor keeps each spec's own forms, including notq/salq/%cl
shifts, inside its universe).

## The stragglers' second attempt
avg, select_bits, next_pow2 get **seeded synthesis**: chains start from
structural variations of the compiler's own form (identity,
single-instruction deletions, adjacent-slot swaps) under a 300s budget —
5x the core batch's optimization budget — instead of pure random restarts.
Rationale (logged in each `straggler_plan` event): synthesis spends most
of its budget crossing algorithmic regions; perturbations of gcc's form
start inside or adjacent to the right region. Variant programs are NOT
equivalent to the spec — every claim still goes through the same
long-timeout z3 re-verification, and an honest no-improvement after the
second attempt is reported as such.

## Consequences
- The win/match/no-improvement ratio is now a statistic over 23 real
  compiler-emitted functions at two optimization levels, not a curated
  anecdote (numbers and evidence in
  `artifacts/compiler-comparison-full.jsonl`).
- The alphabet grew by Not; the enumeration artifact was regenerated once more (278,377 →
  279,088 candidates, optima identical, certificates strictly stronger;
  audit constant updated), following the core-batch precedent.
- The `notq ≡ xorq $-1` identity the enumerator can now see means P7/P8
  alternatives are enumerable in principle; their enumeration scopes
  stay MCMC for feasibility reasons, as logged.
