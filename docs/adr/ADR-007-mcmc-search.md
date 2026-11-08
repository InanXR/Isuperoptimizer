# ADR-007: MCMC search follows STOKE's published move set and hyperparameters

## Status
Accepted

## Context
The search layer implements STOKE's published MCMC move set and
hyperparameters (ASPLOS'13 §3–4, Fig. 10).

## Decision
Implemented exactly as published:
- State: fixed-ℓ programs (ℓ = 50) padded with `UNUSED`.
- Moves with the published probabilities — opcode p_c=0.16, operand
  p_o=0.5, swap p_s=0.16, instruction p_i=0.16, UNUSED-proposal p_u=0.16.
  The published figures sum to 0.98; the four kinds are *normalized* to keep
  their ratios (logged in `mcmc_config`).
- Immediates from the published set: [-16, 16] ∪ {±2^k}, **plus a documented
  extension** {0x55, 0x33, 0x0F patterns} without which SWAR-style targets
  (popcount) are unreachable — the paper's set is tuned for its x86 opcode
  universe, ours is a curated subset; the extension is logged in
  `immediate_policy` on every run.
- Acceptance: Metropolis ratio (the paper's Eq. 6) with β = 0.1, including
  the §4.5 early-exit: sample p first, bound = c_cur − ln(p)/β, stop
  evaluating testcases once the partial sum exceeds the bound.
- Cost: eq' (the improved equality metric, w_m = 3) + perf (H(f) = Σ static
  per-opcode latency) in the optimization phase; eq' only in synthesis.
- Two phases (§4.4): synthesis (random start, correctness only) and
  optimization (correct start, correctness + performance), **and the
  pipeline between them**: each synthesis result seeds a fresh optimization
  run, because crossing algorithm regions is synthesis's job and stripping
  noise inside the new region is then all downhill.
- Testcases: 32 per chain (the paper's default); z3 counterexamples refine
  them (CEGIS).
- eq* discipline: the solver is consulted only on zero-cost candidates,
  with verdict memoization. Per-chain z3 runs on a **4 s per-query
  timeout**: `unknown` means "cannot confirm within budget", never
  "inequivalent" — the candidate is skipped; every headline result is
  re-verified with a long-timeout session before being logged.

## Documented deviations
1. Latency table is static rather than learned by regression (the paper
   learned theirs from hardware counters); ours is a per-opcode table with
   imul = 3, lea-with-index = 3, div = 26, rest = 1.
2. Optimization phases normalize away provably-dead code between proposal
   and costing (`enumerator::strip_dead`, sound by the same dead-code lemma
   as the enumeration pruning). Without this, noise mutations of a 50-slot
   program each cost a solver call and the chain spends its entire budget
   verifying dead code — measured: 43→34 instructions in 90 s before the
   normalization, algorithm-crossing results after. Synthesis stays
   paper-faithful: junk slots are its raw material.
3. Chains are independent threads (no mid-run testcase broadcast; the
   paper broadcasts the *initial* set, as do we; refinements are per-chain
   and all are logged).
4. **(Full-provenance batch, ADR-011 encodability pass)** ALU-slot immediates are
   resampled until x86-encodable (sign-extended imm32) — the paper's
   generator places unrestricted 64-bit immediates on any operand slot.
   Real x86 ALU instructions have no imm64 form; a search that can write
   `andq $0x5555555555555555` competes against gcc output that
   structurally cannot contain it (found when the first full-provenance run
   "won" p23 with exactly such forms — VERIFICATION.md row 32). Full-width
   constants remain available via movq/movabsq, as real code materializes
   them; `immediate_policy` on every run logs the restriction.
5. **(Full-provenance batch, ADR-011 ABI register contract)** Register picks are
   restricted to the SysV caller-clobbered set (RAX, RCX, RDX, RSI, RDI,
   R8–R11) — the paper's operand slots span the full register file,
   including RSP. A candidate that writes RSP/RBP/RBX/R12–15 is not a
   drop-in replacement for the compiled function (found when a claimed
   p13_sign winner held its −1 in %rsp — VERIFICATION.md row 33). The
   model still evaluates callee-saved writes soundly (differential covers
   them); only the searched subset is restricted.

## Consequences
Results are logged as `search_result` events with the final long-timeout
verdict; `recheck --from-log` re-runs every claimed program against every
logged testcase. Unprovable candidates are never silently accepted.
