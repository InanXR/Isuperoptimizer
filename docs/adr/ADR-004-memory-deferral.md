# ADR-004: No memory model in v0; QF_AUFBV reserved for the Array(BV64,BV8) milestone

## Status
Accepted

## Context
Memory is the largest semantics surface after flags (aliasing, partial
overlaps, ordering). STOKE modeled memory as a (64-bit address, 8-bit value)
pair per access with an alias axiom `addr1 = addr2 ⇒ val1 = val2` — a hack
that existed because their solver, STP, had weak array support. Z3's array
theory is not weak.

## Decision
1. v0 has **no memory operations**. `leaq` computes an address into a
   register and touches no memory. Any load/store opcode is an explicit
   parse-time rejection, not a no-op.
2. The z3 session sets `(set-logic QF_AUFBV)` **from day one** so that adding
   `Array(BV64, BV8)` + store chains later requires no protocol or session
   changes (set-logic must precede all assertions and cannot change
   mid-session).
3. When memory lands: flat byte-addressed `Array(BV64, BV8)`, symbolic
   stores as store-chains, loads as selects; definedness extended with an
   out-of-range-access predicate (STOKE's sandbox defines the legal address
   set per testcase; the model does the same via the spec's memory map).

## Alternatives considered
- A single 64-bit "memory cell" toy to exercise the machinery early: rejected
  — it validates nothing that the register miter doesn't already validate,
  and it is not on the critical path.
- STOKE's pair+alias-axiom encoding: rejected as obsolete given Z3 arrays.

## Consequences
Initial verification claims are about register-only straight-line code and must
be labeled as such. The `err` term of the cost function (segfaults, OOB
reads) is structurally zero in v0 and its slot is documented, not faked.