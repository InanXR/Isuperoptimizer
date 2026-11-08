# Evidence index

Every quantitative claim in the README, the paper, and the ADRs traces to
one of the nine committed JSONL artifacts. This page maps claims to
artifacts and gives the command that re-derives each one.

## Artifacts of record

| artifact | contents | validated events |
|---|---|---|
| `enumeration-certificates.jsonl` | exhaustive enumeration certificates for six benchmarks | 1,790 |
| `mcmc-search.jsonl` | STOKE-style MCMC runs, including the 3-instruction `abs` | 2,289 |
| `stress-division-memory.jsonl` | division/memory differentials, hardware oracle, hand pairs | 6 |
| `optimize-certificates.jsonl` | end-to-end optimize pipeline, eight certificates from eleven attempts | 13,434 |
| `stress-compiler-core.jsonl` | compiled-binary oracle and exclusion confirmations, core batch | 7 |
| `compiler-comparison-core.jsonl` | gcc comparison, core batch (11 survivors, 22 baselines) | 32,639 |
| `stress-compiler-full.jsonl` | full-batch pyramid, oracle, exclusions, `notq` coverage | 7 |
| `compiler-comparison-full.jsonl` | gcc comparison, full provenance (23 survivors, 46 baselines) | 64,120 |
| `stress-log.jsonl` | standalone stress-mode log (pair verdicts) | 3 |
| **total** | | **114,295** |

The validated-event counts are the per-kind tallies printed by
`recheck --from-log`. Events that are echoes of the run (mode summaries,
configurations, chain statistics) are counted separately as skipped and
are not part of the total.

## Claim map

| claim | artifact | re-derive with |
|---|---|---|
| six enumerated optima, 218,955 rejected candidates | `enumeration-certificates.jsonl` | `recheck --from-log`, `audit_claims.py` |
| STOKE hyperparameters; `abs` 5 → 3 | `mcmc-search.jsonl` | `recheck --from-log`, `audit_claims.py` |
| 2,000,000 differential evals; 8,000 z3 queries; 4,832-program grid; 60,000 silicon cases plus 8,000 memory cases (2,143 `#DE`); 15 hand pairs | `stress-division-memory.jsonl` | `recheck --from-log`; `stress` regenerates the modes |
| eight certificates from eleven attempts | `optimize-certificates.jsonl` | `recheck --from-log`, `audit_claims.py` |
| core batch: 22 baselines, 44,000 ABI oracle calls, 16 exclusions | `compiler-comparison-core.jsonl`, `stress-compiler-core.jsonl` | `recheck --from-log`, `audit_claims.py` |
| full batch: 46 baselines, 3 beaten / 13 matched / 7 honest, 26 optima, 14,216,086 rejected candidates | `compiler-comparison-full.jsonl` | `recheck --from-log`, `audit_claims.py` |
| full-batch pyramid: 2,000,000 programs × 2 states, 8,000 z3 queries, 5,344-program grid, 64,000 silicon cases plus 8,000 memory cases, 18 pairs | `stress-compiler-full.jsonl` | `recheck --from-log`, `audit_claims.py` |
| full-batch oracle: 92,000 ABI calls, 0 discrepancies | `stress-compiler-full.jsonl` (`compiler_oracle`) | `recheck --from-log`; re-run `stress --compiler-oracle` |
| eight excluded functions (16 confirmations) with empirically confirmed parse errors | `stress-compiler-core.jsonl`, `stress-compiler-full.jsonl` | `recheck --from-log`; `cargo test --test compiler_core --test compiler_full` |
| 114,295 events re-verified, 0 failures | all nine | `paper/build.sh` |
| 485 claims-audit checks | all nine | `python3 tools/audit_claims.py` |
| 45 bibliography entries confirmed | `paper/paper.tex`, `paper/bib-sources.json` | `paper/build.sh` (`check_bib.py`) |
| 34-row defect ledger | `docs/VERIFICATION.md` | read; the paper's table is extracted from it |

## Reproduce

```sh
# one artifact, every concretely checkable claim:
./target/release/recheck --from-log artifacts/<name>.jsonl

# headline numbers across all artifacts:
python3 tools/audit_claims.py

# the paper's full gate (recheck, audit, bibliography, extraction,
# compile, house style):
cd paper && ./build.sh
```

`recheck` validates test cases, rejection witnesses, optima, search
results, and pinned-memory counterexamples concretely. Solver verdicts are
labeled as solver claims and are not re-checked concretely by `recheck`;
that distinction is part of the contract (`docs/adr/ADR-009-artifact-of-record.md`).
