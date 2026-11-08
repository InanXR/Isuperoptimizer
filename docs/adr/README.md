# Architecture decision records

The ADRs record the decisions behind the current system, not the order in
which they were made. Each entry states the context, the decision, the
alternatives considered, and the consequences.

## Semantics and verifier

| ADR | decision |
|---|---|
| [ADR-001](ADR-001-x86-subset.md) | x86-64 subset, GAS AT&T syntax, partial-register writes rejected |
| [ADR-002](ADR-002-flag-definedness.md) | flags are pure values with per-flag definedness |
| [ADR-003](ADR-003-miter-contract.md) | total semantics, definedness predicate, and the miter contract |
| [ADR-004](ADR-004-memory-deferral.md) | memory deferred; QF_AUFBV reserved for the array model |
| [ADR-005](ADR-005-smt-backend.md) | raw SMT-LIB2 over a persistent z3 process; zero crate dependencies |
| [ADR-008](ADR-008-division-and-memory.md) | division and memory semantics |

## Search

| ADR | decision |
|---|---|
| [ADR-006](ADR-006-optimality-certificates.md) | enumeration universe, liveness pruning, optimality certificates |
| [ADR-007](ADR-007-mcmc-search.md) | MCMC follows STOKE's published move set and hyperparameters |

## Artifact pipeline

| ADR | decision |
|---|---|
| [ADR-009](ADR-009-artifact-of-record.md) | the JSONL artifact of record and the recheck contract |

## Compiler comparison

| ADR | decision |
|---|---|
| [ADR-010](ADR-010-compiler-baseline.md) | compiler baseline, universe reconciliation, model corrections |
| [ADR-011](ADR-011-benchmark-scale.md) | scaling the comparison to the full Gulwani/STOKE provenance |
