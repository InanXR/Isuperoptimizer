# ADR-008: Division and memory semantics

## Status
Accepted

## Context
This extension adds the two semantics areas the initial ADRs deferred: division (the first non-flag use of the definedness predicate) and
memory (the promised `Array(BV64, BV8)` model).

## Division
- `divq %r` / `idivq %r` — the x86 one-operand forms: unsigned/signed
  division of RDX:RAX by r; quotient → RAX, remainder → RDX; remainder
  takes the dividend's sign (idiv).
- #DE is a **definedness violation**, not a crash: divisor 0, or quotient
  overflow (unsigned: > 2^64−1; signed: outside i64 — i.e. INT64_MIN / −1).
  Canonical values on the undefined path are 0 for both outputs, matching the
  symbolic folds exactly; the `def` predicate gates every read.
- All four flags are undefined after div/idiv (SDM); definedness threads per
  ADR-002 — reading them afterwards makes the program undefined.
- Symbolic encoding: dedicated AST nodes (`UDiv64`/`IDiv64`,
  `Bool::DivDefined`) printing the 128-bit `bvudiv`/`bvsdiv` expansions
  inline (the paper's uninterpreted-function approach is unnecessary with
  modern z3, and weaker). The quotient-overflow bound uses the 128-bit
  literal `#xffffffffffffffffffffffffffffffff` — a 64-bit literal there is a
  sort error (caught by the extended grid sweep).

## Memory
- Sandbox model: a fixed 256-byte byte-addressable memory at a fixed base;
  the effective address (disp + base + index·scale, mod 2^64) must satisfy
  `addr ∈ [base, base + 248]` for the whole qword, or the access is a
  definedness violation.
- Canonical values: OOB loads read 0, OOB stores are dropped, `def` false.
  Symbolic stores on the undefined path redirect their bytes to a scratch
  address just below the sandbox (never readable — loads only observe
  in-range addresses), which makes folds agree with the concrete drop and
  keeps the def predicate as the only UB signal. This closed a real bug:
  an OOB store at base+250 partially overlaps the tail; unconditional
  symbolic stores leaked ghost bytes into tail windows.
- Symbolic memory: `Array(BV64, BV8)` + byte-granular store chains;
  qword loads print as 8 selects assembled with nested binary concats
  (portable SMT-LIB2).
- Miter scope (deliberate, documented): memory equivalence is asserted at
  **the target's store addresses** — the candidate must reproduce the
  target's writes; writes outside the target's write set are unobservable
  in the v0 sandbox model and therefore unconstrained. `verify_in` accepts
  an optional `PinnedMem` (concrete bytes + base) so counterexamples are
  concretely re-runnable; without a pin the initial array is free and only
  register-only programs (which never declare it) are sound to claim.
- Search generators do not propose Load/Store (MCMC counterexamples must
  be concretely re-runnable; the enumerator's universe stays register-only
  and is logged as such); the differential generators do.

## Hardware oracle
`hw_oracle.c` gained: divq/idivq with #DE pre-checks (the oracle prints
`def` rather than faulting — the model's predicate must agree exactly on which
inputs fault), and load/store against the oracle's own static buffer at its
runtime address, discovered via a `MEMBASE` handshake (ASLR means the
address changes per run — the Rust side builds its sandbox at the same
absolute address with the same byte formula).

## Consequences
The full test pyramid runs on the extended subset: bulk/z3
differentials now include memory state and compare final memories, the grid
sweep covers div/idiv/load/store, and `recheck --from-log` validates
everything concretely.
