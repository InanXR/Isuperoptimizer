# ADR-009: The artifact of record — JSONL event pipeline and the recheck contract

## Status
Accepted (the mechanism shipped in commit a990d00; this write-up was added
during the audit pass)

## Context
A prose report once mis-transcribed a counterexample input while quoting
the true model's outputs — an internally inconsistent artifact caught by
hand-verification. The lesson: **"quote the machine log verbatim" must be
enforced by the pipeline, not by discipline.** Reports paste from the log;
the log is machine-checkable; nothing is retyped by anyone, including the
person writing the reports.

## Decision

### Pipeline shape
1. Every tool (`stress`, `isuperopt enumerate|search|optimize`) writes one
   JSONL **artifact of record** — one flat JSON object per line, fields
   per event kind, written as the run produces them.
2. `recheck --from-log <path>` is the auditor: it re-derives every
   concretely-checkable claim from the log alone (no prose, no z3):
   - `testcase` — the logged `spec_out` must reproduce from the logged
     spec program and inputs.
   - `rejection_witness` / `pair_verdict`-counterexample — logged
     `target_out`/`cand_out` must reproduce, and screen-origin witnesses
     must diverge on a live-out (z3-origin witnesses may be def-based
     with coinciding concrete outputs).
   - `optimum` / `search_result` — the claimed program must match the
     spec on every logged testcase for that benchmark. Equivalence itself
     is *not* concretely checkable: those events are labeled with the
     solver verdict they are, and recheck validates everything around it.
   - Pinned-memory witnesses carry their 256-byte pin in the event
     (`mem_base` + `mem_pin`); recheck replays them with
     `eval_program_in` so they reproduce exactly.
3. `tools/audit_claims.py` re-derives the *headline numbers* (candidate
   totals, witness counts, grid sizes, hyperparameters, ≥8 certificates,
   instruction counts of every claimed optimum) from the artifacts and
   fails loudly on mismatch — reports quote its output, not memory.

### Event schema (the emitted subset)
`ReportLog`/`parse_jsonl_line` accept exactly: flat objects with string /
u64 / bool / string-array values. Anything else fails loudly — log-format
drift can never be silent.

| kind | producer | recheck action |
|---|---|---|
| `bench_start` | enumerate/search/optimize | **validated** (logged spec must parse; registers live-outs; per benchmark) |
| `enumeration_universe` | enumerator | echoed raw (scope of the claim) |
| `testcase` | all | **validated** (spec_out reproduces) |
| `rejection_witness` | enumerator, pairs | **validated** (outputs + divergence) |
| `enumeration_length` | enumerator | echoed raw (counts) |
| `optimum` | enumerator | **validated** (cost 0 on all logged testcases) |
| `mcmc_config` | search | echoed raw (published hyperparameters) |
| `chain_result` | search | echoed raw (stats) |
| `search_result` | search/optimize | **validated** like `optimum` |
| `pair_verdict` | stress pairs | counterexamples **validated** (with pin replay when present) |
| `boundary_case` | stress pairs | **validated** (a historical non-witness: outputs reproduce, both programs defined, live-outs equal — the input must NOT distinguish the pair) |
| `compiler_baseline` | beat | echoed raw (the auditable baseline: compiler, flags, source, asm) |
| `compiler_oracle_func` / `exclusion_confirmed` | stress compiler-oracle | echoed raw (ABI-case counts; exclusion reasons confirmed against the committed assembly) |
| `straggler_plan` / `seeded_start` | beat (straggler pass) | echoed raw (the documented second attempt: approach, budgets, rationale; each chain's start program and its derivation) |
| `straggler_summary` | beat (straggler pass) | echoed raw (outcome of the second attempt; audit_claims.py cross-checks it against the pass's `search_result` events) |
| `enumeration_summary` / `mcmc_summary` / `mode_summary` / `run_summary` | all | echoed raw (numbers of record) |

`recheck` prints a **per-kind tally** with its total (`bench_start=22 optimum=14 …`),
so the reported count traces to source without reverse-engineering (an external-audit
finding: the count included 22 `bench_start` validations the
original table described as merely "echoed" — the logged spec *parsing* is a
real check — producing a 22-event gap an auditor had to reverse-engineer).

### Contract rules
- Every counterexample event is self-contained: inputs (+ memory pin when
  memory is involved) and both programs' concrete outputs are in the
  event, so a third party re-derives the witness with the committed
  concrete evaluator alone.
- Solver claims are always labeled as solver claims
  (`"equivalent (z3, …; solver claim)"`); `recheck` skips them explicitly
  rather than pretending to check them.
- Count-summarized rejection classes (over the witness threshold) remain
  re-derivable: any rejected candidate diverges on some committed
  testcase, so re-running the candidate over the committed test set
  reproduces its rejection.
- **Failure evidence is evidence** (added after the i64::MAX boundary
  incident, `docs/VERIFICATION.md` row 23): an artifact that records a
  failure under investigation must never be overwritten or deleted — the
  `ReportLog` truncates on open, so reproduction attempts go to a
  *distinct* log path, and failing-run artifacts are committed (or at
  minimum preserved) before the fix is applied. A witness cited in any
  report must exist in a committed artifact; if it does not, the report
  is corrected, not defended.

## Consequences
- Trust lives in the pipeline: the artifact is the claim; the report is
  commentary. `recheck` + `audit_claims.py` are the two commands a third
  party runs.
- z3's response protocol is ours to keep correct; it is tested by the
  sampled z3 differential (thousands of parsed models per run).
