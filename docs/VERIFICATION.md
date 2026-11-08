# Verification architecture and defect ledger

This document defines the verification layers, records the defects they
caught, and states the volume at which each layer runs. It is a reference:
the ledger IDs are stable and are cited by the ADRs and the paper.

## Layers

| layer | what it does | typical volume |
|---|---|---|
| **L1 unit/corner tests** | hand-written exact cases (`cargo test`) | 66 tests |
| **L2 bulk differential** | concrete evaluator vs constant-folded symbolic over random programs and states, including memory | 2,000,000 + 4,000,000 program-evals |
| **L3 z3-sampled differential** | full SMT-LIB2 emission, evaluated by z3 (pinned arrays, final-memory windows) | 8,000-10,000 queries |
| **L4 self-verification grid** | `verify(p,p)` must be UNSAT for every grid program | 4,832 + 5,344 programs |
| **L5 hand pairs** | hand-verified equivalences plus deliberately broken pairs with re-checked counterexamples | 15 + 18 pair verdicts |
| **L6 hardware oracle** | concrete semantics against real silicon (flags, #DE pre-check protocol, MEMBASE handshake) | 68,000 + 72,000 cases |
| **L7 recheck + claims audit** | artifacts re-derived from the log alone | every event |

No layer is trusted alone. The concrete evaluator is checked against
silicon (L6); the symbolic evaluator against the concrete (L2, L3); the
miter against itself (L4); hand-verified mathematics against all of the
above (L5); and the reporting against the artifacts (L7).

## Defect ledger

Every row records the defect, the layer that caught it, and why the other
layers could not. Rows are numbered once and keep their IDs.

| # | bug | caught by | why the others missed it |
|---|---|---|---|
| 1 | Concrete IMUL zero-extended operands into i128 — wrong CF/OF for negative operands | corner unit test | random states hit negative×negative rarely in the flag bits that differ; symbolic construction was *right*, so differential testing would eventually catch it but the corner test did first |
| 2 | Symbolic IMUL dropped the carry out of t1+t2 (can reach 2⁶⁵−2) | corner unit test (`a=b=−1`) | folding agrees with concrete almost everywhere; only the exact 65-bit-carry input diverges |
| 3 | `msb()`/`bit()` compared a 1-bit extract against a 64-bit constant — z3 sort error | miter self-checks (grid) | all *folding* was correct; only the printed SMT was wrong, so layers 2/6 saw nothing |
| 4 | get-value pair scanner consumed only the first pair (depth never decremented) | z3-sampled differential | outputs still compared "successfully" on the first register; partial parses looked like passes |
| 5 | z3 answers unknown commands with `unsupported` + `;` comment lines — protocol desync | empirical probe during the search phase | not a semantics bug at all; only long-running interactive use exposes it |
| 6 | Sample harness declared `r15` after asserting expressions that referenced it | z3-sampled differential | ordering bug in harness construction, invisible to layers that emit declares first |
| 7 | C oracle CMOV macro had out-of-range `%2` operand → garbage flags | hardware-oracle mode | the *oracle* was wrong, not the semantics — every cmov case "failed" instantly |
| 8 | Enumerator optimized the program *as written* and exposed a desc/spec mismatch (`shl $63` extracts bit 0, not 63) | the enumerator itself (enumeration layer) | the spec was wrong; no test layer can catch an author's intent error — the certificate is about the program text, which is why the tool was right to optimize the text |
| 9 | Optimization chains froze at the spec: every noise mutation cost a solver call (whole budget on dead code) | chain statistics during the search phase | not a correctness bug; a throughput collapse visible only in proposals/s |
| 10 | z3 `unknown` under per-query timeout crashed the chain ("refusing to claim" treated as fatal) | search smoke run | correct strictness in the wrong place — chains must *skip*, final claims must *re-verify* |
| 11 | Unsigned #DE bound printed as the 128-bit all-ones literal instead of zero-extended u64::MAX — every quotient ≤ 2¹²⁸ counted as defined | z3-sampled differential (1,733 discrepancies) | the *fold* was correct; concrete/fold differential (layer 2) saw perfect agreement |
| 12 | Division value nodes returned real quotients on the undefined path instead of the canonical 0 | z3-sampled differential | concrete canonicalizes; symbolic folded correctly; only the SMT text differed |
| 13 | Load64 printer byte-rotated the concat chain (s6..s0,s7) | z3-sampled differential (384 cases) | folds were right AND the miter stayed sound (both sides rotate identically) — only cross-checking z3's evaluated memory against concrete bytes exposed it |
| 14 | OOB loads read unpinned array slots — z3 free to pick values where concrete canonicalizes | z3-sampled differential (28 cases) | the def predicate was correct; the *value* expression wasn't gated |
| 15 | Signed #DE lower bound used `bvsgt` — the exact INT64_MIN quotient boundary was misclassified | z3-sampled differential (1 case in 8,000) | a one-in-2⁶⁴-ish boundary; random sampling hit it once |
| 16 | OOB store at base+250 leaked ghost bytes into tail windows (symbolic stores unconditional) | bulk differential (memory) | whole-qword range rule vs per-byte store overlap — a pure concrete/symbolic fold disagreement |
| 17 | Signed #DE **upper** bound used `bvslt` — quotient exactly i64::MAX (defined) rejected as undefined; the miter "counterexampled" a pair whose outputs were equal. Caught by the **first run** of the hand-picked broken idiv pair: z3 returned the boundary input `rax = 0xfffffffffffffffe` as a "counterexample" whose recomputed outputs were identical — a signature no correct predicate can produce. **The discovery witness was real but its artifact was later destroyed — see row 23.** The permanent record is the committed `boundary_case` event (the boundary input logged as a replayable non-witness: both programs defined, outputs equal) plus `tests/division_memory.rs::idiv_quotient_i64_max_boundary_is_defined`, which pins the exact input through the fold, the miter's fails-formula, z3 — and the historical buggy predicate text itself, which z3 confirms was false at the boundary. The *committed* `broken_pair_3` witness (`rax = 0x8000000000000001`) demonstrates the pair's intended subject: the missing sign-extension before `idiv` (target remainder −1 vs candidate +1). Two different defects, one pair: the spurious counterexample exposed the predicate bug; the genuine one exercises the sign-extension trap. | **hand-picked broken pair, first run** (division/memory review-gap closure) | survived 8,000 random z3 samples *and* the earlier boundary fix's symmetric twin: random inputs essentially never make the quotient exactly i64::MAX. The showcase row for why hand-picked pairs exist — with the caveat that its discovery trail was initially destroyed (row 23) |
| 18 | `verify_in` returned TestCases with outputs computed under zero memory even for pinned queries | audit pass (API-contract review) | no committed artifact was affected (the pairs logger recomputes under the pin); a latent contract violation only a code audit finds |
| 19 | Unpinned memory SAT crashed the model parser on z3's array-lambda terms — and such counterexamples are unreproducible by construction | audit pass, via a scope test that legitimately went SAT | every committed memory query was either UNSAT or pinned; the crash path had never been exercised |
| 20 | `enumeration_universe` omitted `include_cl_shifts` and the lea forms — the declared universe had silent dimensions | audit pass | additive declaration gap; results unaffected |
| 21 | ADR-006 referenced from `enumerator.rs` since the enumeration layer **did not exist** | audit pass | documentation debt; code↔doc cross-references must be checked like code |
| 22 | An early prose report mis-transcribed a counterexample input | **external hand-verification** | the machine outputs were right; the *report* was the bug — this created the artifact-of-record pipeline (ADR-009) |
| 23 | The i64::MAX boundary bug's discovery witness (row 17) was **destroyed**: the failing run's artifact was overwritten by the truncating re-run and then `rm`'d before the commit. The finding was real, but from the repository's perspective it was indistinguishable from an invented detail — the exact failure ADR-009 exists to prevent, committed by ADR-009's own author. Found by **external audit** demanding "show me the logged event." | external audit of the *audit machinery* | nothing in the pyramid watches the debugging workflow itself; failure evidence produced outside the artifact pipeline has no protection. Fixed three ways: a permanent `boundary_case` event (replayable non-witness, recheck-validated), a regression test committing the historical buggy predicate text, and the ADR-009 rule below |
| 24 | Model: entry flags treated as defined-false — candidates could read RFLAGS at function entry and get fake constants (`sete %rax` at entry → constant 0). Sound only while every spec also never read entry flags; real gcc output never does, so search exploited the phantom state. Found when a beat-run candidate optimum began with `sete`. Fix: entry flags undefined (SysV ABI: caller-clobbered); the optimize artifact was regenerated; pinned by committed test (setb-form rejected, leaq/xorq/cmovl form stands) | **compiler-output integration** (compiler-baseline phase) | every earlier layer used hand-written specs that masked the issue — exactly why "beat the compiler" was a required phase, not a nice-to-have |
| 25 | Model: the enumeration universe did not contain the spec's own instruction forms — gcc's `leaq -1(%rdi), %rax` fell outside the declared lea forms, so the identity program was not enumerable and "no equivalent of length ≤ 2" was an artifact of the universe | **first gcc-targeted enumeration** (compiler-baseline phase) | universe declarations were built for hand-written specs whose forms were all in the base set; no random or hand pair can see a universe-scoping bug — only the real baseline tripped it |
| 26 | Symbolic flag-WRITING arms spread `SFlags::fresh()` (entry state) instead of an all-defined spread — after the entry-flags fix, every logic-op's ZF/SF/CF/OF came out "undefined" symbolically while concrete said defined | **bulk differential** (immediate) | the differential harness caught the fix's own bug on the first run — the pyramid working as designed |
| 27 | `i128::MIN / -1` panics real Rust division even in RELEASE — a ~12%-per-run latent crash in the idivq evaluator, fold, and `DivDefined` predicate; fixed with `wrapping_div` (MIN, correctly rejected as #DE by the range check). Unsigned twin pinned by test as DEFINED. | **bulk differential** (release-mode run, compiler-baseline phase) | the exact-boundary input is one value in 2¹²⁸; corner tests and extensive random sampling had missed it — only sustained millions-scale random evaluation in release mode reached the panic |
| 28 | ADR-009's event table said `bench_start` was "echoed" but `recheck` counted it as validated (the logged spec *parsing* is a real check) — a 22-event gap between the reported count and an auditor's four-kind tally that had to be reverse-engineered from behavior. Found by **external audit** refusing to round away 0.07% | external audit | a count that doesn't decompose by construction invites exactly this question. Fixed both directions: the table now marks `bench_start` validated (what the code really does), and `recheck` prints a per-kind tally with every total (`bench_start=22 optimum=14 rejection_witness=7220 …`) so the number traces to source. Also caught: the `pair_verdict` site was missing from the first tally patch — the breakdown itself found the omission (stress-log showed `()` for 3 validated pairs) |
| 29 | ADR-010 claimed "`movabsq` = `movq` with a full-width immediate" as accepted syntax, but **the opcode was never in the parser** — and the core-batch test meant to pin it used `parse_program(...).unwrap_or_else(\|_\| …)`, a fallback that made the test pass exactly when the claim was false. The core batch never noticed because no survivor's assembly contained movabsq (the excluded div7's parse error cited it, confirming the exclusion for the wrong-but-safe reason). The full batch's p23_popcnt_bytes emits movabsq and the new reconciliation test failed on the real parse error. Found by the **full batch itself** before any artifact was written | the full-batch survivor test | the fourth instance of the project's signature failure shape — a test that cannot fail asserting a doc claim nothing checks. The self-neutralizing `unwrap_or_else` fallback is the mechanism worth remembering: a test passes, the ADR says it passes, and the opcode still rejects every real instance. Fixed: movabsq parses (imm64→reg only, routed to Mov — the first cut accidentally routed it to Cmp, caught by the un-neutered test asserting program equality, not just parse success), and the test now pins it without any fallback |
| 30 | `split_asm_functions` included the `.note.gnu.property` trailer in the LAST function's chunk, so any batch whose last function is a *survivor* would fail reconciliation with a bogus "label after ret" control-flow error — a **false exclusion by file-position accident**. The core batch never hit it (its last function, avg2, is excluded and its real parse error fires before the trailer). Found when the full batch's sat_add (last in file, a survivor) needed to parse | the full-batch reconciliation test | the failure shape is positional, not semantic: the same assembly parses or fails depending on function order in the file. Fixed structurally, not by reordering the C file: chunks now close at gcc's `.LFE` function-end marker (no instruction of any function ever follows its own `.LFE`), and only identifier-shaped labels can open chunks (the trailer's numeric `0:`/`1:` labels belong to no function). A regression test pins both fixes against the committed fixtures |
| 31 | The full-provenance seeded-synthesis pass reused `run_chain`'s non-synthesis pre-seed of `best` with the start program — valid for the paper's optimization phases (start = the spec, or a chain-verified synthesis result) but **false for seeded chains, whose starts are deliberate structural mutations of gcc's form**. Every seeded chain therefore reported its non-equivalent start variant as `best`; the pass-level selection picked it, and the final long-timeout z3 verification correctly REFUTED it, erroring the pass and skipping the `straggler_summary` events. Found by the **final-verification gate doing its job** on the first straggler run — three FAIL lines, zero false claims in the artifact | the straggler FAIL lines + reading `run_chain`'s pre-seed | the failure shape is the project's fifth instance of a story-vs-artifact gap, this time caught *before* anything was committed: the gate refused the claim, the run was stopped, and the whole batch was regenerated with the fix. Fixed at the root: `best` pre-seeds only for `optimization`/`optimization_from_synthesis` (known-correct starts); seeded chains with no verified improvement now return `best = None`. Also hardened: a refuted candidate is a logged outcome (`candidate_refuted_on_final_verification` summary + non-claiming search_result), never a silent Err. Regression-pinned with a zero-budget chain test asserting seeded `best` is `None` while optimization's pre-seed stands |
| 32 | The first full-provenance run claimed **p23_popcnt_bytes 16 → 14** — but three of the 14 forms were `andq $imm64` (`$0x5555555555555555`, `$0xcccccccccccccccd`, `$0x3333333333333333`), which real x86 cannot encode: AND takes sign-extended imm32 only, which is exactly why gcc materialized those constants with movabsq. The v0 parser's doc comment said "sign-extended imm32 or movabs imm64" but the implementation accepted any u64 on every ALU op — so the *search* had instruction forms gcc's output structurally cannot contain, and one headline win rested on them (as real instructions the program is 17, worse than gcc's 16). Found by **reading the claimed winner against the ISA** before reporting it | inspection of the claimed program against x86 encoding rules | the fifth instance of model-vs-world: a verifier-perfect equivalence proof over a universe that quietly contains forms the hardware doesn't have. The fix is deliberately two-layered: the *searched subset* (enumeration alphabet, MCMC generators and every mutation path that can move an immediate onto an ALU slot) is now restricted to imm32-encodable values, while the *model/parser stays permissive* — committed rejection witnesses replay such candidates and must keep re-checking (pinned by test). Wide constants remain available via movq/movabsq, exactly as gcc uses them; the MCMC policy deviation from STOKE's unrestricted generator is logged in every mcmc_config event. The enumeration artifact was regenerated once more (279,088 → 249,223 candidates, optima byte-identical) and the whole beat run re-adjudicated: the certificate's meaning changed from "no shorter in-model program" to "no shorter *encodable* program" — the claim the compiler comparison actually wants |
| 33 | The second full-provenance run claimed **p13_sign 5 → 4** — a genuine four-instruction sign function (`negq; setl; movq $-1, %rsp; cmovgq`) except that it materialized its −1 **in %rsp**: in-model perfectly correct (v0 treats all 16 GPRs as equal and the live-out was RAX only, and z3 proved the equivalence) but a real function body that sets RSP=−1 jumps through the return address on `ret` — it is not a drop-in replacement for gcc's function at all. The v0 search space had licensed callee-saved and stack writes since the search phase without a claimed winner ever using one. Found by **reading the claimed winner against the SysV ABI** immediately after the encodability pass | inspection of the new claimed program against the ABI (the same discipline as row 32) | the sixth instance, and the second in one run: the model's register file is an abstraction, and "equivalent, live-out RAX" quietly diverges from "equivalent function" whenever the winner touches RSP/RBP/RBX/R12-R15. Fixed to make the two claims coincide by construction: every register the MCMC can pick is now caller-clobbered only (RAX,RCX,RDX,RSI,RDI,R8-R11), the enumeration destination set drops RBX, and the identity floor now unions the spec's write-registers too (p19's gcc form writes %r8). `for_benchmark` keeps the historical RBX-including pin: the enumeration/optimize artifacts certify in-model benchmark optima, not drop-ins — their committed artifacts reproduce unchanged. Run re-adjudicated a third time; the p13 *algorithm* (4-instruction sign) remains reachable with a scratch register, and if the chains find it again the win stands clean |
| 34 | `audit_claims.py` printed its **AUDIT PASSED verdict mid-file** — right after the optimize section — and `sys.exit(0)`-by-default meant every check added after that point (all of the compiler-batch sections) could print `FAIL` lines without changing the exit code. The core batch's headline numbers were therefore never actually gated by the audit's verdict. Found while extending the audit for the full batch: a run against the in-progress full-batch artifact printed four FAIL lines and still exited 0 | extending the audit for the full batch (its own probe output) | the same failure family as row 28, one layer up: the *verifier of the numbers* had a number of its own that didn't decompose to what it checked. Fixed structurally: the verdict block moved to the end of the file so it runs over every section; the exit code now fails on any mismatch anywhere |

## Failure shapes

The ledger is more useful read by shape than by date. The recurring
shapes are:

### Reporting discipline (rows 22, 23)

The machine outputs were correct; the prose was not. Row 22 mis-transcribed
a counterexample input, and row 23 lost the evidence for a real finding
because the debugging workflow overwrote it. Both were found by external
hand-verification of claims the pipeline had already blessed. They produced
the artifact-of-record rule (ADR-009): failure evidence is evidence, and a
witness cited in prose must exist in a committed artifact.

### Model versus world (rows 24, 25, 32, 33)

A verifier-perfect equivalence proof is only as good as the universe and
the model it quantifies over. Entry flags that were defined-false, a
universe missing the spec's own lea form, ALU immediates that real x86
cannot encode, and a winner that wrote `%rsp` all passed the miter and
failed the hardware or the ABI. Each was found by confronting the model
with something outside it: real compiler output, the ISA encoding rules,
or the calling convention.

### The fold is right, the printer is wrong (rows 3, 13, 14)

The constant-folded semantics agreed with concrete evaluation everywhere,
while the emitted solver text was wrong. Concrete-vs-symbolic testing
cannot see this because it never round-trips through the printed text.
The z3-sampled differential (L3) exists for this shape.

### Tests that cannot fail (row 29)

A documented opcode acceptance was pinned by a test with an
`unwrap_or_else` fallback that made it pass exactly when the claim was
false. The ADR said it worked, the test said it worked, and the parser
rejected every real instance. The un-neutered test found the defect on
the first batch that emitted the opcode.

### The verifier needs verifying (rows 28, 34)

An event counter whose totals did not decompose to what it validated, and
an audit script that printed its verdict before running the sections that
mattered. Both are failures of the observability layer itself: the number
that describes the check had not been checked.

### Boundary coverage (rows 15, 17)

Random sampling catches large formula errors (row 11 recorded 1,733
discrepancies in one run) but has essentially zero probability at exact
boundaries. Row 15 required one hit in 8,000 samples, and row 17 was
caught by a hand-picked pair on its first run.

### Oracle and harness defects (rows 4, 6, 7)

A wrong oracle or harness masquerades as a semantics defect. The pyramid's
structure localizes these: a wrong oracle fails every case loudly, while
the differentials isolate the actual divergence.

### Audit-machinery defects (rows 18-21)

API-contract violations, silent schema dimensions, and dangling
documentation references are their own layer. They were found after every
program-level test was green.

## Verification volume

The paper's title-page claim is the sum of the nine per-artifact tallies
printed by `paper/build.sh` (live `recheck --from-log`, 0 FAILED):

| artifact | validated events | composition |
|---|---|---|
| `enumeration-certificates` | 1,790 | bench_start=6 optimum=6 rejection_witness=1,730 testcase=48 |
| `mcmc-search` | 2,289 | bench_start=3 search_result=2 testcase=2,284 |
| `stress-division-memory` | 6 | boundary_case=1 pair_verdict=5 |
| `optimize-certificates` | 13,434 | bench_start=11 optimum=6 rejection_witness=1,829 search_result=4 testcase=11,584 |
| `stress-compiler-core` | 7 | boundary_case=1 pair_verdict=6 |
| `compiler-comparison-core` | 32,639 | bench_start=22 optimum=14 rejection_witness=7,220 search_result=6 testcase=25,377 |
| `compiler-comparison-full` | 64,120 | bench_start=52 optimum=26 rejection_witness=20,538 search_result=10 testcase=43,494 |
| `stress-compiler-full` | 7 | boundary_case=1 pair_verdict=6 |
| `stress-log` | 3 | pair_verdict=3 |
| **total** | **114,295** | |

An independent recomputation that reports 113,067 differs by exactly
1,228 events, and the difference decomposes without remainder:

| source | events |
|---|---|
| `enumeration-certificates` at 3f3ba54 (pre-compiler-baseline: 119,680 candidates, 3,249 witnesses) instead of af74118 (218,955 candidates, 1,730 witnesses) | +1,519 |
| `optimize-certificates` at ce322f7 (pre-entry-flags: 119,680 candidates, 3,249 witnesses) instead of e560d62 (278,377 candidates, 1,829 witnesses) | -2,744 |
| `stress-log.jsonl` omitted from the sum | -3 |
| **net** | **-1,228** |

The enumeration artifact's stale copy is the pre-compiler-baseline
version, and its 119,680 candidates / 3,249 witnesses are the stale
values. That staleness inflates the sum by 1,519. The shortfall comes from
a second stale artifact, `optimize-certificates` at its original ce322f7
revision, whose certificate suite embeds the same six enumeration
benchmarks under the old universe (hence the identical candidate and
witness totals). The stale optimize artifact costs 2,744 events, which
more than offsets the stale enumeration artifact and the missing
`stress-log`. Re-pulling the nine committed artifacts and re-running

    ./target/release/recheck --from-log artifacts/<name>.jsonl

on each reproduces 114,295 with 0 failures; `paper/build.sh` does this on
every build and aborts on any FAILED event.
