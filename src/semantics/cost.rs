//! Testcases and the STOKE improved equality metric (ASPLOS'13 §4.6, Eq. 15).
//!
//! eq'(R;T,τ) = Σ_t Σ_{r∈ρ} min_{r'∈bw(r)} [ POP(val(T,r) ⊕ val(R,r')) + w_m·1{r≠r'} ]
//!
//! with w_m = 3 (the paper's Figure 10). All registers are 64-bit in v0, so
//! bw(r) is every register. The `mem` term is structurally zero (no memory
//! model, ADR-004) and the `err` term is structurally zero (no sandboxed
//! faults in the v0 subset); both slots are documented, not faked.

use crate::instr::{Gpr, Program};
use crate::rng::Rng;
use crate::state::eval_program;

/// Wrong-location penalty, straight from the paper's Figure 10.
pub const W_M: u64 = 3;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestCase {
    pub inputs: [u64; 16],
    /// The target's final register state on `inputs`.
    pub outputs: [u64; 16],
    /// The target's live-out registers (what `outputs` is compared on).
    pub live_outs: Vec<Gpr>,
}

impl TestCase {
    pub fn from_spec_inputs(spec: &crate::verify::Spec, inputs: [u64; 16]) -> TestCase {
        TestCase {
            inputs,
            outputs: eval_program(&spec.program, inputs).regs,
            live_outs: spec.live_outs.clone(),
        }
    }

    /// Random testcase: uniform-random inputs (the STOKE default) with a
    /// sprinkle of bit-pattern-heavy values.
    pub fn random(spec: &crate::verify::Spec, rng: &mut Rng) -> TestCase {
        let mut inputs = [0u64; 16];
        for i in 0..16 {
            inputs[i] = match rng.below(4) {
                0 => rng.interesting_u64(),
                _ => rng.next(),
            };
        }
        TestCase::from_spec_inputs(spec, inputs)
    }
}

/// Cost of a *concrete final state* against one testcase: the STOKE improved
/// equality metric — for each live-out register, the Hamming distance to the
/// closest candidate register, plus the wrong-location penalty w_m when the
/// match is not at the same register. (Extracted for the MCMC sampler, which
/// accumulates this per testcase with early-exit acceptance.)
pub fn case_cost(state_regs: &[u64; 16], t: &TestCase) -> u64 {
    let mut total = 0u64;
    for &g in &t.live_outs {
        let target = t.outputs[g.idx()];
        let mut best = u64::MAX;
        for cand in 0..16usize {
            let d = (target ^ state_regs[cand]).count_ones() as u64;
            let penalty = if cand == g.idx() { 0 } else { W_M };
            let c = d + penalty;
            if c < best {
                best = c;
            }
        }
        total += best;
    }
    total
}

/// Cost of `prog` against the testcases: lower is closer to correct.
/// Zero means "output matches the target on every live-out, every testcase"
/// — the precondition for spending a solver call (STOKE's eq* discipline).
pub fn cost_on_testcases(prog: &Program, cases: &[TestCase]) -> u64 {
    let mut total: u64 = 0;
    for t in cases {
        let s = eval_program(prog, t.inputs);
        total += case_cost(&s.regs, t);
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instr::parse_program;
    use crate::verify::Spec;

    fn spec() -> Spec {
        Spec {
            program: parse_program("movq %rax, %rcx").unwrap(),
            live_outs: vec![Gpr::Rcx],
        }
    }

    #[test]
    fn exact_match_costs_zero() {
        let s = spec();
        let mut rng = Rng::new(1);
        let cases: Vec<TestCase> = (0..32).map(|_| TestCase::random(&s, &mut rng)).collect();
        // identity rewrite
        assert_eq!(cost_on_testcases(&s.program.clone(), &cases), 0);
    }

    #[test]
    fn wrong_location_beats_wrong_value() {
        let s = spec();
        let mut rng = Rng::new(2);
        let cases: Vec<TestCase> = (0..32).map(|_| TestCase::random(&s, &mut rng)).collect();
        // right value, wrong register: penalty is exactly w_m per testcase.
        // NB: the improved metric scans *all* candidate registers, and the
        // untouched rax in the rewrite still holds the target value — so this
        // rewrite is *also* w_m-per-case (that is the metric working as designed).
        let cand = parse_program("movq %rax, %rdx").unwrap();
        assert_eq!(cost_on_testcases(&cand, &cases), 32 * W_M);
        // garbage everywhere: clobber the value in every register it appears.
        let cand = parse_program("movq $0, %rcx\nmovq $0, %rax").unwrap();
        assert!(cost_on_testcases(&cand, &cases) > 32 * W_M);
    }
}
