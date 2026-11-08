# ADR-002: Flags are pure values with per-flag definedness; never mutable hidden state

## Status
Accepted

## Context
x86 flag semantics are the classic minefield. The SDM says IMUL's two-operand
form leaves ZF/SF "undefined (along with AF and PF)", and for shifts "OF [is]
affected only for 1-bit shifts (otherwise undefined)" while "if the count is
0, the flags are not affected." A verifier that stores flags as plain state
values must silently pick *something* for undefined flags — and whatever it
picks changes which equivalences it "proves." That is unsound.

## Decision
1. Flag values (ZF, SF, CF, OF) are **pure values threaded through the state**,
   produced by the instructions that set them. "Unaffected" means the previous
   value (and definedness) is threaded through unchanged.
2. Each flag carries a **definedness bit** alongside its value. Instructions
   that leave a flag undefined (IMUL: ZF/SF; SHL/SHR/SAR with count ∈ [2,63]:
   OF) set that flag's definedness to false. Undefined flags take the
   canonical value `false`; the canonical value is *never* semantically
   meaningful because:
3. A program-level **definedness predicate** (`def`) accumulates only when a
   flag is **read** (by `set<cc>`/`cmov<cc>`, per the SDM condition table).
   Reading an undefined flag makes `def` false on that path. Thus no
   counterexample or equivalence ever depends on the value of an undefined flag.
4. Flags are never live outputs in v0. `Spec.live_outs` contains GPRs only;
   flag-setting differences between target and rewrite are therefore
   unconstrained (matching how the search cost function treats them).

## Alternatives considered
- STOKE's approach (flags as symbolic state cells, UB handled by producing
  "counterexamples [that] might appear wrong in the sandbox because of the
  undefined behavior"): rejected — it lets an undefined value influence the
  verdict; this model is strictly more conservative.
- Making undefined flags a hard error: rejected — `imulq` followed by nothing
  that reads ZF is a perfectly well-defined program; only *reads* matter.

## Consequences
`def` is nontrivial from the start (the initial subset already has undefined-flag cases),
which exercises the miter's `def_T → def_R` contract immediately. A program
that is UB on all inputs verifies as "equivalent" to anything (vacuous
contract) — intended, documented, and matches STOKE's stance that
"optimization is undefined for ill-formed programs."