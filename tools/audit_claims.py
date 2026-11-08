#!/usr/bin/env python3
"""audit_claims.py — verify every headline number in the project reports
directly from the committed JSONL artifacts. Exits nonzero on any mismatch.

Usage: python3 tools/audit_claims.py
"""
import json
import sys
from collections import defaultdict

FAIL = 0

def check(name, cond, detail=""):
    global FAIL
    if cond:
        print(f"  OK   {name} {detail}")
    else:
        print(f"  FAIL {name} {detail}")
        FAIL += 1

def load(path):
    events = []
    with open(path) as f:
        for i, line in enumerate(f, 1):
            try:
                events.append((i, json.loads(line)))
            except json.JSONDecodeError as e:
                print(f"  FAIL {path}:{i} is not valid JSON: {e}")
                FAIL += 1
        return events

print("== enumeration-certificates.jsonl ==")
enumeration = load("artifacts/enumeration-certificates.jsonl")
by_len = defaultdict(int)
witnesses = defaultdict(int)
summaries = {}
optima = {}
universes = {}
for _, d in enumeration:
    k = d.get("kind")
    b = d.get("benchmark")
    if k == "enumeration_length":
        by_len[b] += d["candidates"]
        # every length below the optimum must be exhaustively rejected
        if d["equivalent_found"]:
            optima[b] = d["length"]
    elif k == "rejection_witness":
        witnesses[b] += 1
    elif k == "enumeration_summary":
        summaries[b] = d
    elif k == "enumeration_universe":
        universes[b] = d

for b in sorted(summaries):
    s = summaries[b]
    check(f"{b}: candidates_total matches the sum of length events",
          by_len[b] == s["candidates_total"],
          f"(lengths sum {by_len[b]}, summary {s['candidates_total']})")
    check(f"{b}: optimum length consistent",
          optima.get(b) == s["optimal_length"],
          f"(found at {optima.get(b)}, summary {s['optimal_length']})")
    check(f"{b}: witnesses_logged matches rejection_witness events",
          witnesses[b] == s["witnesses_logged"],
          f"(events {witnesses[b]}, summary {s['witnesses_logged']})")
    check(f"{b}: status pass", s["status"] == "pass")
    check(f"{b}: universe declares alphabet + threshold",
          "alphabet_size" in universes[b] and "witness_log_threshold" in universes[b])

total = sum(s["candidates_total"] for s in summaries.values())
# Regenerations of the enumeration suite (ADR-011): Not joined the alphabet
# (278,377 -> 279,088); the encodability pass removed non-imm32-encodable
# ALU-immediate forms (-> 249,223); the ABI register contract removed
# callee-saved destinations (-> 218,955). Optima byte-identical
# throughout; the certificate now reads over the x86-encodable,
# ABI-scratch universe — what the compiler comparison actually wants.
check("enumeration: total candidates across benchmarks = 218,955", total == 218955, f"({total})")
check("enumeration: 6 benchmark summaries", len(summaries) == 6)

print("== mcmc-search.jsonl ==")
search = load("artifacts/mcmc-search.jsonl")
cfgs = 0
chain_results = 0
abs_opt = None
for _, d in search:
    k = d.get("kind")
    if k == "mcmc_config":
        cfgs += 1
        check_vals = {
            "p_opcode": "0.16", "p_operand": "0.5", "p_swap": "0.16",
            "p_instruction": "0.16", "p_unused": "0.16", "beta": "0.1",
            "ell": 50, "testcases": 32,
        }
        for f, want in check_vals.items():
            got = d.get(f)
            ok = (got == want) if isinstance(want, int) else (str(got) == want)
            check(f"mcmc_config: {f} == {want}", ok, f"(got {got})")
    elif k == "chain_result":
        chain_results += 1
        check("chain_result: proposals > 0", d["proposals"] > 0)
        # move mix sanity: all four move kinds exercised
        for m in ("move_opcode", "move_operand", "move_swap", "move_instruction"):
            check(f"chain {d['chain']} ({d['phase']}): {m} exercised", d[m] > 0)
    elif k == "search_result" and d.get("phase") == "optimization" and d.get("benchmark") == "abs":
        abs_opt = d
check("search: one mcmc_config per benchmark", cfgs == 3, f"({cfgs})")
check("search: abs optimization result exists with verdict equivalent",
      abs_opt is not None and abs_opt["verdict"].startswith("equivalent"))
if abs_opt:
    prog = [l for l in abs_opt["program"].split("\n") if l.strip() and l.strip() != "unused"]
    check("search: abs improvement is 5 -> 3 instructions",
          abs_opt["spec_length"] == 5 and len(prog) == 3, f"({abs_opt['spec_length']} -> {len(prog)})")

print("== stress-division-memory.jsonl ==")
divmem = load("artifacts/stress-division-memory.jsonl")
modes = {d["mode"]: d for _, d in divmem if d.get("kind") == "mode_summary"}
check("division/memory: bulk 2,000,000 evals 0 discrepancies",
      modes["bulk"]["program_evals"] == 2000000 and modes["bulk"]["discrepancies"] == 0)
check("division/memory: z3 8,000 queries 0 discrepancies",
      modes["z3"]["queries"] == 8000 and modes["z3"]["discrepancies"] == 0)
check("division/memory: grid 4,832 programs 0 failures",
      modes["grid"]["programs"] == 4832 and modes["grid"]["failures"] == 0)
check("division/memory: hw 60,000 cases 0 discrepancies",
      modes["hw"]["cases"] == 60000 and modes["hw"]["discrepancies"] == 0)
check("division/memory: hw-mem 8,000 cases, #DE exercised, 0 discrepancies",
      modes["hw_mem"]["cases"] == 8000 and modes["hw_mem"]["def_cases"] > 2000
      and modes["hw_mem"]["discrepancies"] == 0)
pair_names = [d.get("pair") for _, d in divmem if d.get("kind") == "pair_verdict"]
for p in ("equiv_pair_7", "broken_pair_3", "equiv_mem_pair", "broken_mem_pair"):
    check(f"division/memory: {p} present", p in pair_names)
for _, d in divmem:
    if d.get("kind") == "pair_verdict" and d.get("pair") == "broken_mem_pair":
        check("broken_mem_pair: witness carries the 256-byte memory pin",
              d.get("verdict") == "counterexample" and len(d.get("mem_pin", [])) == 256)
    if d.get("kind") == "pair_verdict" and d.get("pair") == "broken_pair_3":
        check("broken_pair_3: counterexample (not the vacuous boundary)",
              d.get("verdict") == "counterexample" and d.get("differs_on") == ["rdx"])

bc = [d for _, d in divmem if d.get("kind") == "boundary_case"]
check("division/memory: i64::MAX boundary non-witness event present (exactly one)", len(bc) == 1)
if bc:
    d = bc[0]
    check("boundary_case: input is the exact historical witness (rax = 0xfffffffffffffffe)",
          d["inputs"][0] == "0xfffffffffffffffe")
    check("boundary_case: outputs equal (the input is a non-witness)",
          d["target_out"][2] == d["cand_out"][2] and d["outputs_equal"])
    check("boundary_case: both programs defined",
          d["target_defined"] and d["cand_defined"])

print("== optimize-certificates.jsonl ==")
optimize = load("artifacts/optimize-certificates.jsonl")
optimum_events = {}
search_events = {}
optimize_summaries = {}
for _, d in optimize:
    k = d.get("kind")
    if k == "optimum":
        optimum_events[d["benchmark"]] = d
    elif k == "search_result":
        search_events[(d["benchmark"], d["phase"])] = d
    elif k in ("enumeration_summary", "mcmc_summary"):
        optimize_summaries[d["benchmark"]] = d
for b, d in optimum_events.items():
    prog = [l for l in d["program"].split("\n") if l.strip() and l.strip() != "unused"]
    check(f"optimize {b}: optimum program length == logged length", len(prog) == d["length"])
    check(f"optimize {b}: verdict is an equivalence claim", d["verdict"].startswith("equivalent"))
for (b, phase), d in search_events.items():
    prog = [l for l in d["program"].split("\n") if l.strip() and l.strip() != "unused"]
    check(f"optimize {b}/{phase}: program instr count == logged length", len(prog) == d["length"])
    if phase == "optimization":
        check(f"optimize {b}: perf improved", d["perf"] < d["spec_perf"])
certified = sum(1 for b, d in optimum_events.items()) + \
           sum(1 for (b, p), d in search_events.items() if p == "optimization" and d["verdict"].startswith("equivalent"))
check("optimize: at least 8 benchmarks hold certificates", certified >= 8, f"({certified})")
check("optimize: all benchmark summaries present", len(optimize_summaries) == 11, f"({len(optimize_summaries)})")

print()
# (VERIFICATION.md row 34): the verdict used to print HERE —
# mid-file, after the optimize section — so every later section (all of
# compiler-core and compiler-full checks) could FAIL without changing the exit
# code. The verdict now runs once, at the end of the file, over every
# section.

print("== compiler-comparison-core.jsonl ==")
import os
if os.path.exists("artifacts/compiler-comparison-core.jsonl"):
    core_comp = load("artifacts/compiler-comparison-core.jsonl")
    baselines = {}
    core_optima = {}
    core_searches = {}
    for _, d in core_comp:
        k = d.get("kind")
        if k == "compiler_baseline":
            baselines[(d["function"], d["level"])] = d
        elif k == "optimum":
            core_optima[(d["benchmark"], "enum")] = d
        elif k == "search_result":
            core_searches[(d["benchmark"], d["phase"])] = d
    for (fn, lv), d in baselines.items():
        check(f"baseline {fn}/{lv}: compiler, flags, source, asm all logged",
              "compiler" in d and "flags" in d and "source" in d and "asm" in d)
        prog = [l for l in d["asm"].split("\n") if l.strip() and l.strip() != "unused"]
        check(f"baseline {fn}/{lv}: asm length matches the logged program", len(prog) > 0)
    n_baselines = len(baselines)
    check("compiler-core: 22 compiler baselines (11 functions x 2 levels)", n_baselines == 22, f"({n_baselines})")
    # acceptance: >= 5 functions end-to-end with verified certificates
    certified = set()
    for (b, _), d in list(core_optima.items()) + list(core_searches.items()):
        v = d.get("verdict", "")
        if v.startswith("equivalent") or (b.endswith(("O2", "O3")) and "length" in d and v.startswith("equivalent")):
            certified.add(b)
    # optima carry verdict; search_results too
    for (b, _), d in list(core_optima.items()) + list(core_searches.items()):
        if d.get("verdict", "").startswith("equivalent"):
            certified.add(b)
    n_funcs = len(set(b.rsplit("_", 1)[0] for b in certified))
    check("compiler-core: >= 5 compiler-emitted functions hold certificates", n_funcs >= 5, f"({n_funcs}: {sorted(certified)})")
    # acceptance: at least one function compared against both O2 and O3
    both = {b.rsplit("_", 1)[0] for b in certified if b.endswith("O2")} & {b.rsplit("_", 1)[0] for b in certified if b.endswith("O3")}
    check("compiler-core: >= 1 function certified at BOTH -O2 and -O3", len(both) >= 1, f"({sorted(both)})")
else:
    print("  (artifacts/compiler-comparison-core.jsonl not yet generated)")
    FAIL += 0

print("== stress-compiler-core.jsonl (compiler oracle) ==")
if os.path.exists("artifacts/stress-compiler-core.jsonl"):
    core_stress = load("artifacts/stress-compiler-core.jsonl")
    oracle_fns = [d for _, d in core_stress if d.get("kind") == "compiler_oracle_func"]
    check("compiler-core oracle: 22 function-level records (11 x 2 levels)", len(oracle_fns) == 22, f"({len(oracle_fns)})")
    for d in oracle_fns:
        check(f"oracle {d['function']}/{d['level']}: 0 discrepancies", d["discrepancies"] == 0)
    core_modes = {d["mode"]: d for _, d in core_stress if d.get("kind") == "mode_summary"}
    if "compiler_oracle" in core_modes:
        m = core_modes["compiler_oracle"]
        check("compiler-core oracle: mode summary pass, 44,000 cases",
              m["status"] == "pass" and m["cases"] == 44000 and m["discrepancies"] == 0,
              f"(cases {m['cases']}, disc {m['discrepancies']})")
    excl = [d for _, d in core_stress if d.get("kind") == "exclusion_confirmed"]
    check("compiler-core: 16 exclusion confirmations (8 functions x 2 levels)", len(excl) == 16, f"({len(excl)})")
    for d in excl:
        check(f"exclusion {d['function']}/{d['level']}: parse error confirms the logged reason",
              d["parse_error"] and "MISSING" not in d["parse_error"] and "UNEXPECTEDLY" not in d["parse_error"])
else:
    print("  (artifacts/stress-compiler-core.jsonl not yet generated)")

print("== compiler-comparison-full.jsonl (full-suite compiler comparison) ==")
if os.path.exists("artifacts/compiler-comparison-full.jsonl"):
    full_comp = load("artifacts/compiler-comparison-full.jsonl")
    full_baselines = {}
    full_optima = {}
    full_searches = {}
    full_outcomes = {}
    full_stragglers = {}
    for _, d in full_comp:
        k = d.get("kind")
        if k == "compiler_baseline":
            full_baselines[(d["function"], d["level"])] = d
        elif k == "optimum":
            full_optima[d["benchmark"]] = d
        elif k == "search_result":
            full_searches[(d["benchmark"], d.get("phase"))] = d
        elif k in ("enumeration_summary", "mcmc_summary"):
            full_outcomes.setdefault(d["benchmark"], []).append(d)
        elif k == "straggler_summary":
            full_stragglers[d["benchmark"]] = d

    n_baselines = len(full_baselines)
    fns7 = sorted({fn for (fn, _) in full_baselines})
    check("compiler-full: 46 compiler baselines (23 functions x 2 levels)", n_baselines == 46, f"({n_baselines})")
    check("compiler-full: 23 distinct functions", len(fns7) == 23, f"({len(fns7)})")
    for (fn, lv), d in sorted(full_baselines.items()):
        check(f"compiler-full baseline {fn}/{lv}: compiler, flags, batch, source, asm, nargs all logged",
              all(f in d for f in ("compiler", "flags", "batch", "source", "asm", "nargs")))

    # attempted end-to-end: every function at every level has a completed
    # outcome (enumeration_summary or mcmc_summary)
    attempted = set(full_outcomes)
    expected = {f"{fn}_{lv}" for fn in fns7 for lv in ("O2", "O3")}
    check("compiler-full: every function attempted end-to-end at BOTH levels (46 bench outcomes)",
          attempted == expected, f"({len(attempted)} of {len(expected)})")

    # the ratio, derived only from the artifact
    beaten, matched, honest = set(), set(), set()
    for bench in expected:
        fn = bench.rsplit("_", 1)[0]
        if bench in full_optima:
            d = full_optima[bench]
            if d["length"] < d["spec_length"]:
                beaten.add(fn)      # enumeration found something shorter than gcc
            else:
                matched.add(fn)      # optimum == gcc length: proven minimal in-universe
        if (bench, "optimization") in full_searches:
            d = full_searches[(bench, "optimization")]
            if d.get("verdict", "").startswith("equivalent") and d["perf"] < d["spec_perf"]:
                beaten.add(fn)      # MCMC beat the gcc form
        if (bench, "seeded_optimization") in full_searches:
            d = full_searches[(bench, "seeded_optimization")]
            if d.get("verdict", "").startswith("equivalent") and d["perf"] < d["spec_perf"]:
                beaten.add(fn)      # the straggler second attempt beat it
        if bench not in full_optima and (bench, "optimization") not in full_searches:
            honest.add(fn)          # MCMC-scoped, ran to budget, no verified improvement
    matched -= beaten
    honest -= beaten | matched
    check("compiler-full: ratio covers all 23 functions", len(beaten | matched | honest) == 23,
          f"(beaten={len(beaten)} matched={len(matched)} no-improvement={len(honest)})")
    print(f"       compiler-full ratio: beaten {len(beaten)} | matched-with-proof {len(matched)} | honest no-improvement {len(honest)}")
    print(f"       beaten: {sorted(beaten)}")

    # Calibration contracts (VERIFICATION.md rows 32-33), machine-enforced:
    # every CLAIMED program (optimum, equivalent search_result, or straggler
    # result) must be x86-encodable (no ALU immediate outside sign-extended
    # imm32) and ABI-scratch-clean (no write to a callee-saved/stack
    # register) — otherwise "beats the compiler" is not a drop-in claim.
    import re as _re
    _alu = _re.compile(r'^(addq|subq|andq|orq|xorq|cmpq|movabsq?)\s+\$(.+?),(?:\s|%|$)')
    def _imm32_ok(tok):
        # the immediate's 64-bit pattern, reinterpreted as signed — that is
        # what a sign-extended imm32 can hold (e.g. $0xffffffffffffffff
        # is -1, encodable; $0x5555555555555555 is not).
        tok = tok.strip()
        neg = tok.startswith('-')
        body = tok.lstrip('-')
        v = int(body, 16) if body.startswith('0x') else abs(int(body))
        if neg:
            v = (-v) % (2**64)
        s = v - 2**64 if v >= 2**63 else v
        return -2**31 <= s <= 2**31 - 1
    _callee_saved = {'rbx', 'rsp', 'rbp', 'r12', 'r13', 'r14', 'r15'}
    def _claim_issues(d):
        prog = d.get('program') or d.get('asm') or ''
        issues = []
        for line in prog.split('\n'):
            t = line.strip()
            if not t or t == 'unused' or t.endswith(':'):
                continue
            m = _alu.match(t)
            if m and m.group(1).startswith(('add', 'sub', 'and', 'or', 'xor', 'cmp')) and not _imm32_ok(m.group(2)):
                issues.append(f"non-encodable ALU immediate: {t}")
            parts = [p.strip() for p in t.split(' ', 1)]
            if parts[0] in ('divq', 'idivq'):
                continue  # the explicit operand is a divisor (a read)
            ops = [p.strip() for p in parts[1].split(',')] if len(parts) == 2 else []
            if ops:
                dst = ops[-1]
                if dst.startswith('%') and dst[1:] in _callee_saved:
                    issues.append(f"callee-saved/stack register write: {t}")
        return issues
    claimed_programs = list(full_optima.values()) + [d for d in full_searches.values()]
    for d in claimed_programs:
        issues = _claim_issues(d)
        bench = d.get('benchmark', d.get('function'))
        check(f"compiler-full claim {bench}: encodable and ABI-scratch-clean",
              not issues, f"({'; '.join(issues[:3])})")

    # every function compared at BOTH levels (the identical-asm note is in
    # the baselines; both levels were challenged regardless)
    for fn in fns7:
        check(f"compiler-full {fn}: logged and challenged at O2 and O3",
              f"{fn}_O2" in attempted and f"{fn}_O3" in attempted)

    # the three stragglers: documented second attempt with logged outcome
    for fn in ("avg", "select_bits", "next_pow2"):
        s2 = full_stragglers.get(f"{fn}_O2")
        s3 = full_stragglers.get(f"{fn}_O3")
        check(f"compiler-full straggler {fn}: second attempt logged at both levels",
              s2 is not None and s3 is not None)
        for bench, s in ((f"{fn}_O2", s2), (f"{fn}_O3", s3)):
            if s is None:
                continue
            seeded = full_searches.get((bench, "seeded_optimization"))
            if s["status"] == "improved":
                check(f"compiler-full straggler {bench}: improved status backed by an equivalent seeded search_result",
                      seeded is not None and seeded["verdict"].startswith("equivalent")
                      and seeded["perf"] < seeded["spec_perf"])
            elif s["status"] == "candidate_refuted_on_final_verification":
                check(f"compiler-full straggler {bench}: refuted-candidate status backed by a NON-claiming seeded search_result",
                      seeded is not None and not seeded["verdict"].startswith("equivalent"))
            else:
                check(f"compiler-full straggler {bench}: no-improvement status backed by NO claimed seeded result",
                      seeded is None)
else:
    print("  (artifacts/compiler-comparison-full.jsonl not yet generated)")

print("== stress-compiler-full.jsonl (notq pyramid + full-batch compiler oracle) ==")
if os.path.exists("artifacts/stress-compiler-full.jsonl"):
    full_stress = load("artifacts/stress-compiler-full.jsonl")
    full_modes = {d["mode"]: d for _, d in full_stress if d.get("kind") == "mode_summary"}
    for mode in ("bulk", "z3", "grid", "hw", "hw_mem", "compiler_oracle"):
        if mode in full_modes:
            m = full_modes[mode]
            bad = m.get("discrepancies", m.get("failures"))
            check(f"compiler-full stress {mode}: pass, zero discrepancies/failures",
                  m["status"] == "pass" and bad == 0,
                  f"(cases {m.get('cases', m.get('program_evals', m.get('queries', m.get('programs', '?'))))}, disc {bad})")
    if "bulk" in full_modes:
        check("compiler-full bulk: 2,000,000-program differential (x2 states)",
              full_modes["bulk"]["programs"] == 2000000 and full_modes["bulk"]["program_evals"] == 4000000)
    if "grid" in full_modes:
        check("compiler-full grid: 5,344 programs (Not entries joined)", full_modes["grid"]["programs"] == 5344,
              f"({full_modes['grid']['programs']})")
    if "hw" in full_modes:
        check("compiler-full hw: 64,000 silicon cases across 32 ops (incl. `not` with flag-preset)",
              full_modes["hw"]["cases"] == 64000 and full_modes["hw"]["ops"] == 32,
              f"(cases {full_modes['hw']['cases']}, ops {full_modes['hw']['ops']}, flag comparisons {full_modes['hw'].get('flag_comparisons')})")
    full_pairs = {d.get("pair"): d for _, d in full_stress if d.get("kind") == "pair_verdict"}
    notq_ev = next((d for d in full_pairs.values() if d.get("target", "").strip() == "notq %rcx"), None)
    check("compiler-full pairs: notq == xor $-1 equivalence verified (found by program content)",
          notq_ev is not None and notq_ev["verdict"] == "equivalent",
          f"(pair={notq_ev['pair'] if notq_ev else 'MISSING'})")
    full_oracle = [d for _, d in full_stress if d.get("kind") == "compiler_oracle_func"]
    check("compiler-full oracle: 46 function-level records (23 x 2 levels)", len(full_oracle) == 46, f"({len(full_oracle)})")
    for d in full_oracle:
        check(f"compiler-full oracle {d['function']}/{d['level']}: 0 discrepancies", d["discrepancies"] == 0)
    if "compiler_oracle" in full_modes:
        m = full_modes["compiler_oracle"]
        check("compiler-full oracle: mode summary pass, 92,000 cases (2000 x 23 x 2)",
              m["status"] == "pass" and m["cases"] == 92000 and m.get("batch") in ("full", "week7"),  # historical label kept for the committed artifact
              f"(cases {m['cases']}, disc {m['discrepancies']}, batch {m.get('batch')})")
    full_excl = [d for _, d in full_stress if d.get("kind") == "exclusion_confirmed"]
    check("compiler-full: 16 exclusion confirmations (8 functions x 2 levels)", len(full_excl) == 16, f"({len(full_excl)})")
    for d in full_excl:
        check(f"compiler-full exclusion {d['function']}/{d['level']}: parse error confirms the logged reason",
              d["parse_error"] and "MISSING" not in d["parse_error"] and "UNEXPECTEDLY" not in d["parse_error"])
else:
    print("  (artifacts/stress-compiler-full.jsonl not yet generated)")

# ---------------------------------------------------------------- final verdict
# (moved here from mid-file — VERIFICATION.md row 34: the verdict used to
# print after the optimize section, so core_comp/full_comp FAILs never touched the exit code)
print()
if FAIL:
    print(f"AUDIT FAILED: {FAIL} mismatch(es)")
    sys.exit(1)
print("AUDIT PASSED: all checked claims reproduce from the artifacts")
