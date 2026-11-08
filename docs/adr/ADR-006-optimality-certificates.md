# ADR-006: Enumeration universe, liveness pruning, and the optimality certificate

## Status
Accepted (the write-up was added during the audit pass; the module had
referenced this decision before the file existed)

## Context
The enumeration layer must produce *provably optimal* programs — not merely improved ones.
"Optimal" is only meaningful against a declared universe, and the search
space must be pruned hard enough to enumerate exhaustively.

## Decision

### The universe is the certificate's scope
Every enumeration declares, in an `enumeration_universe` event: the opcode
alphabet (with its size), destination registers, source registers,
immediates, shift counts (including whether `%cl` counts are proposed), the
lea addressing forms, the maximum length, and the witness-logging
threshold. **Optimal means: no equivalent program of shorter length exists
within this universe** — nothing stronger is claimed, and the universe is
part of the certificate so the claim is checkable.

Two universe restrictions and why they are sound *for existence*:
- Source registers are restricted to the spec's read set: the miter shares
  all 16 input registers, so reading a register the spec never reads
  creates input-dependence the equivalence check rejects (the only
  exceptions cancel exactly, and a cancellation is never length-minimal —
  the same value is available as an immediate).
- Destination registers are restricted to a 4-register working set (the
  benchmarks need no more; the universe is declared).

### Backward-liveness pruning, with definedness
An instruction is enumerated only if it writes something live afterwards:
a register in the live-out set or read by a later instruction, or flags
read by a later flag-consumer. Memory writes are never pruned (ADR-008).

**Soundness argument for the optimality claim, including definedness.**
Deleting a dead-write instruction D from program P:
1. *Registers/flags:* "dead" means no later instruction reads the value D
   wrote before it is overwritten — and the liveness analysis counts ALL
   reads, including reads that feed divisors, addresses, and definedness
   predicates. So no surviving instruction's inputs change; hence no
   surviving instruction's outputs, and no surviving UB condition
   (they depend only on the inputs of surviving instructions), changes.
2. *Definedness:* deleting D can only *remove* UB sources, never add one
   (point 1). So P-without-D is defined at least wherever P is.
3. *The claim:* if P (length L, containing D) is equivalent to the spec,
   then on every input where the spec is defined, P is defined and outputs
   match; by (1)+(2) P-minus-D is also defined there with the same
   outputs — so P-minus-D is equivalent too, and by induction some
   liveness-clean program of length < L is enumerated and checked.

Hence: if every *enumerated* program of length < K is rejected with a
concrete witness, no equivalent program of length < K exists in the
universe, pruned or not. This is the exact statement the certificate makes.

### Certificate structure
Per benchmark the artifact records: the universe, every testcase (seeded
and z3-refined, CEGIS), per-length candidate counts, and for every
rejected candidate a witness — z3 rejections with the solver's model
(concretely re-checkable), testcase-screen rejections with the first
diverging testcase (logged one-by-one per length class while the class is
small, by count otherwise; count-summarized witnesses stay re-derivable
from the committed testcases, since any screened candidate diverges on
some logged testcase). The first equivalent program at the shortest
length is the optimum; its `optimum` event carries the program and the
z3 equivalence verdict, and `recheck --from-log` re-runs it against every
logged testcase.

## Alternatives considered
- Observational-equivalence (fingerprint) dedup of candidates: sound for
  *rejection* only when the fingerprint set is closed under later witness
  additions; the bookkeeping was not worth it at these sizes.
- Enumerating suffixes with prefix memoization: unsound (two prefixes
  agreeing on testcases can diverge off them).

## Audit findings folded in
- The universe event originally omitted `include_cl_shifts` and the lea
  forms; a universe declaration with silent dimensions is exactly the
  kind of gap an auditor flags. Both are now logged (the deterministic
  enumeration artifact was regenerated under the full schema; the optimize
  artifact's enumeration events predate the two additive fields — a strict subset
  declaration, results unaffected and recheck-clean).
