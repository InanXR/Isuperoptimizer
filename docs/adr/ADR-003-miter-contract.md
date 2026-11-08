# ADR-003: Total semantics, definedness predicate, and the miter contract

## Status
Accepted

## Context
A verifier is only as sound as its notion of "undefined." The miter contract
is:

```
claim: ∀s0. def_T(s0) → def_R(s0) ∧ out_T(s0) = out_R(s0)
query: ∃s0. def_T(s0) ∧ (¬def_R(s0) ∨ out_T ≠ out_R)   [check SAT]
```

UNSAT ⇒ equivalent. SAT ⇒ the model is a counterexample, which becomes a
TestCase for the search loop (STOKE's τ-refinement, ASPLOS'13 §4.1).

## Decision
1. Every instruction in the v0 subset is a **total function** on the machine
   state: no panics, no host-language UB, no silent truncation. Shift counts
   are masked `& 63` per the SDM ("The count is masked to 5 bits (or 6 bits
   with a 64-bit operand)") — explicit in code, not incidental. The only
   "undefinedness" is flag definedness (ADR-002), carried as data.
2. The definedness predicate lives in the state type from day one
   (`CState.def` / `SState.def`), even though the initial subset has no division
   or memory. Nothing about `def` is retrofitted later: division (divisor≠0)
   and memory (out-of-sandbox accesses) will extend it.
3. Miter encoding: all 16 input registers are **shared** free constants for
   both target and rewrite. This is semantically identical to STOKE's
   "states that agree on the live inputs": a register the target never reads
   cannot influence the target's outputs, so sharing it merely constrains
   the rewrite on a register it has no right to depend on — any real
   dependence is exactly what the miter must catch. Live *outputs* are
   declared explicitly in `Spec.live_outs`.
4. Counterexamples must be independently re-checkable: `verify()` returns
   the model as a `TestCase` carrying concrete inputs **and** the target's
   concrete outputs, so any consumer can confirm the mismatch by running the
   concrete evaluator by hand. No verdict ever depends on trusting z3's
   model extraction for *soundness* — only for *witness generation*.

## Alternatives considered
- Restricting the miter to the target's actual read set as live inputs
  (requires read-set analysis): deferred — same verdicts, extra machinery.
- Treating candidate UB as immediate rejection without checking `def_T`:
  rejected — the contract's implication direction (`def_T → def_R`) is what
  lets ill-formed targets be optimized at all; it is the STOKE contract.

## Consequences
A target that reads undefined flags everywhere has `def_T ≡ false` and
verifies as equivalent to any rewrite. This vacuity is the price of the
STOKE contract and is *documented behavior*, tested explicitly.