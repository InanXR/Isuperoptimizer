# ADR-005: Raw SMT-LIB2 text over a persistent z3 subprocess; zero crate dependencies

## Status
Accepted

## Context
Two viable ways to drive Z3 from Rust: (a) the `z3` crate (v0.22.0, actively
maintained, FFI bindings via `z3-sys`), (b) emitting SMT-LIB2 text and
speaking to a `z3 -in -smt2` subprocess over pipes. The design plan called
for (b); Phase 0 was required to confirm or challenge it with findings.

## Findings
- `z3` crate: healthy (published 2026-10-28, 2.3M total downloads), but the
  `bundled` feature compiles Z3 from source (cmake + python toolchain tax on
  every clean build), and non-bundled still requires a system libz3 at
  runtime. Either way, a Z3 installation is not avoided.
- The z3 binary speaks a deterministic pipe protocol with
  `(set-option :print-success true)`: every command answers exactly one
  response (`success`, `sat`/`unsat`/`unknown` for check-sat, or an
  s-expression for get-value/get-model, or `(error "...")`). The Online Z3
  Guide documents push/pop scopes for exactly this style of control.
- The query shape is fixed and small: ≤16 declare-consts, one assert,
  check-sat, optional get-value. Manual serialization is bounded work.

## Decision
Emit SMT-LIB2 text; drive one long-lived `z3 -in -smt2` process per worker
with push/pop scopes per query; parse responses with a small balanced-paren
reader. The whole crate has **zero external dependencies** — the build is
hermetic (std only), the solver binary is located via `ISUPEROPT_Z3`, then
`PATH`.

Rationale:
1. **Auditability**: the emitted query is the certificate artifact — it can
   be saved, re-run under any SMT-LIB2 solver (cvc5 cross-check), and diffed.
2. No build-time coupling to a Z3 version; swapping solvers is a string.
3. The 100k+/s *testcase* throughput does not involve z3 at all (that is the
   Rust interpreter's job); z3 round trips happen only in `verify()` and the
   sampled differential, so binding-level speed is not the bottleneck.

## Alternatives considered
- `z3` crate bindings: kept as a documented fallback if query-shape diversity
  ever makes manual serialization expensive. The performance argument does
  not apply to this access pattern.

## Consequences
Response parsing is ours to get right; it is tested by the sampled z3
differential harness (thousands of queries compared against the concrete
evaluator). A `(timeout 60000)` session option bounds pathological queries.
z3 errors surface as hard failures with the offending script in the message —
a verifier must never swallow a solver error.