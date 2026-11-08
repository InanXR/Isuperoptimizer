//! The equivalence verifier — the miter per ADR-003.
//!
//! claim: ∀s0. def_T(s0) → def_R(s0) ∧ out_T(s0) = out_R(s0)
//! query: ∃s0. def_T(s0) ∧ (¬def_R(s0) ∨ out_T ≠ out_R)      [check SAT]
//!
//! UNSAT ⇒ equivalent. SAT ⇒ the model is a counterexample, returned as a
//! `TestCase` carrying concrete inputs *and* the target's concrete outputs so
//! any consumer can independently confirm the mismatch without trusting z3.

use crate::ast::*;
use crate::cost::TestCase;
use crate::instr::{Gpr, Instr, Program, Slot};
use crate::state::{eval_program, eval_program_in};
use crate::sym::{sym_eval_program, SState};
use crate::z3proc::{Answer, Z3Session};

/// The target: its program plus the registers the caller cares about.
#[derive(Clone, Debug)]
pub struct Spec {
    pub program: Program,
    pub live_outs: Vec<Gpr>,
}

pub enum Verdict {
    Equivalent,
    Counterexample(TestCase),
}

/// An optional concrete memory pinned into the miter: the initial array is
/// asserted to equal these bytes at this base (so SAT models are concretely
/// re-runnable). `None` leaves the initial memory free — only sound for
/// register-only programs, where the array is never declared at all.
pub struct PinnedMem<'a> {
    pub bytes: &'a [u8],
    pub base: u64,
}

fn touches_memory(p: &Program) -> bool {
    p.0.iter().any(|s| {
        matches!(
            s,
            Slot::Instr(Instr::Load { .. }) | Slot::Instr(Instr::Store { .. })
        )
    })
}

/// `verify(cand, spec, z3)` — candidate first, per the search-loop contract.
/// Register-only: memory stays out of the query entirely.
pub fn verify(cand: &Program, spec: &Spec, z3: &mut Z3Session) -> Result<Verdict, String> {
    verify_in(cand, spec, z3, None)
}

/// Full miter per ADR-003, extended with memory per ADR-008:
///   claim: ∀s0. def_T(s0) → def_R(s0) ∧ regs match on live_outs
///                  ∧ memory matches at every address the target stored to
///   query: ∃s0. def_T(s0) ∧ (¬def_R(s0) ∨ regs differ ∨ stored-bytes differ)
///
/// The memory comparison uses the *target's* store addresses: the candidate
/// must reproduce the target's writes; writes outside the target's write
/// set are unobservable in the v0 sandbox model and therefore unconstrained
/// (deliberate, documented in ADR-008).
pub fn verify_in(
    cand: &Program,
    spec: &Spec,
    z3: &mut Z3Session,
    pin: Option<&PinnedMem>,
) -> Result<Verdict, String> {
    let uses_mem = touches_memory(cand) || touches_memory(&spec.program);
    let init = SState::from_vars();
    let t = sym_eval_program(&spec.program, init.clone());
    let r = sym_eval_program(cand, init);

    let mut outs = Vec::new();
    for &g in &spec.live_outs {
        outs.push(Bool::Eq(t.regs[g.idx()].clone(), r.regs[g.idx()].clone()));
    }
    let mut seen_addrs = std::collections::HashSet::new();
    if uses_mem {
        for a in &t.stores {
            if seen_addrs.insert(a.to_smt2()) {
                outs.push(Bool::Eq(
                    BV::Load64 { mem: Box::new(t.mem.clone()), addr: Box::new(a.clone()) },
                    BV::Load64 { mem: Box::new(r.mem.clone()), addr: Box::new(a.clone()) },
                ));
            }
        }
    }
    let outs_eq = bool_and_all(outs);
    let fails = bool_and(
        t.def.clone(),
        bool_or(bool_not(r.def.clone()), bool_not(outs_eq)),
    );

    let mut cmds = Vec::with_capacity(20);
    for i in 0..16 {
        cmds.push(format!("(declare-const r{} (_ BitVec 64))", i));
    }
    if uses_mem {
        cmds.push("(declare-const mem (Array (_ BitVec 64) (_ BitVec 8)))".to_string());
        if let Some(p) = pin {
            for (k, b) in p.bytes.iter().enumerate() {
                cmds.push(format!(
                    "(assert (= (select mem #x{:016x}) #x{:02x}))",
                    p.base.wrapping_add(k as u64),
                    b
                ));
            }
        }
    }
    cmds.push(format!("(assert {})", fails.to_smt2()));

    // NB: never request the array in get-value — z3 returns array-lambda
    // terms that are neither parseable nor concretely reproducible. Under a
    // pin the array is fully determined anyway; without one a SAT model's
    // memory is unknowable, which the check below rejects outright.
    let names: Vec<String> = (0..16).map(|i| format!("r{}", i)).collect();
    let model = match z3.solve(&cmds, Some(&names))? {
        Answer::Unsat => return Ok(Verdict::Equivalent),
        Answer::Unknown => return Err("z3 returned unknown: refusing to claim equivalence".into()),
        Answer::Sat(vals) => vals,
    };
    if uses_mem && pin.is_none() {
        // The counterexample depends on an arbitrary initial memory z3 chose;
        // it cannot be replayed concretely, so it must not be handed out as
        // a TestCase (ADR-003: every counterexample is independently
        // reproducible). The non-equivalence verdict itself is still true.
        return Err(
            "counterexample under free memory is not concretely reproducible; \
             supply a PinnedMem to obtain a checkable witness"
                .into(),
        );
    }
    let mut inputs = [0u64; 16];
    for (name, v) in model {
        let idx: usize = name
            .strip_prefix('r')
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| format!("unexpected model symbol `{}`", name))?;
        inputs[idx] = v;
    }
    // The counterexample TestCase must be concretely reproducible (ADR-003):
    // when the query ran under a pinned memory, the target's outputs must be
    // computed under the SAME pin — not under the zero-filled default,
    // which would silently contradict the query that produced the witness.
    let outputs = match pin {
        Some(p) => {
            let mut mem = [0u8; crate::state::MEM_BYTES];
            let n = p.bytes.len().min(mem.len());
            mem[..n].copy_from_slice(&p.bytes[..n]);
            eval_program_in(&spec.program, inputs, mem, p.base).regs
        }
        None => eval_program(&spec.program, inputs).regs,
    };
    Ok(Verdict::Counterexample(TestCase {
        inputs,
        outputs,
        live_outs: spec.live_outs.clone(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instr::parse_program;

    fn spec(src: &str, live: &[Gpr]) -> Spec {
        Spec { program: parse_program(src).unwrap(), live_outs: live.to_vec() }
    }

    #[test]
    fn identity_is_equivalent() {
        let mut z = Z3Session::new().unwrap();
        let s = spec("movq %rax, %rcx", &[Gpr::Rcx]);
        let v = verify(&s.program.clone(), &s, &mut z).unwrap();
        assert!(matches!(v, Verdict::Equivalent));
    }

    #[test]
    fn wrong_output_gives_checkable_counterexample() {
        let mut z = Z3Session::new().unwrap();
        let s = spec("movq %rax, %rcx", &[Gpr::Rcx]);
        let cand = parse_program("movq %rbx, %rcx").unwrap();
        match verify(&cand, &s, &mut z).unwrap() {
            Verdict::Counterexample(tc) => {
                // Independently confirm: run both by hand on the model inputs.
                let t = eval_program(&s.program, tc.inputs);
                let c = eval_program(&cand, tc.inputs);
                assert_ne!(t.regs[1], c.regs[1]);
                assert_eq!(t.regs[1], tc.outputs[1]);
            }
            Verdict::Equivalent => panic!("mov rbx must not be equivalent to mov rax"),
        }
    }
}
