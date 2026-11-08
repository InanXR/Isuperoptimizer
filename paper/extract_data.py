#!/usr/bin/env python3
"""extract_data.py — machine-extract every number quoted in the paper.

The paper has one rule: no number is hand-typed. This script reads the
committed JSONL artifacts, the committed markdown record, and the Rust
source, and emits paper/generated/*.tex so that the PDF is regenerated
from the live artifacts. Anything it cannot find is a hard error, never
a silent zero.

Inputs (all committed, all read-only):
  ../artifacts/compiler-comparison-full.jsonl  compiler baselines, optima,
                                    search results, straggler outcomes,
                                    enumeration summaries
  ../artifacts/stress-compiler-full.jsonl      the verification-pyramid mode
                                    summaries, pair verdicts, compiler-oracle
                                    records
  ../artifacts/enumeration-certificates.jsonl  optimality certificates
  ../docs/VERIFICATION.md           the 34-row defect ledger + pyramid layers
  ../fixtures/compiler-full/candidates.c       P-number provenance comments
  recheck output (produced by build.sh before this script runs)
  Rust sources                      test-count extraction

Outputs: paper/generated/*.tex + *.asm
"""
import json
import os
import re
import sys
from collections import OrderedDict

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
ART = os.path.join(REPO, "artifacts")
GEN = os.path.join(HERE, "generated")
os.makedirs(GEN, exist_ok=True)

def out(name, text):
    with open(os.path.join(GEN, name), "w") as f:
        f.write(text)

def load(path):
    with open(os.path.join(ART, path)) as f:
        for line in f:
            yield json.loads(line)

# --------------------------------------------------------------- LaTeX escape
UNICODE_MAP = [
    # no em dashes in the document (house style): the spaced dash becomes a
    # semicolon, any remaining occurrence a comma
    ("\u2014", "; "),
    ("\u2212", r"\ensuremath{-}"),  # Unicode minus sign (VERIFICATION.md row 33)
    ("\u00d7", r"\ensuremath{\times}"),
    ("\u2264", r"\ensuremath{\leq}"),
    ("\u2265", r"\ensuremath{\geq}"),
    ("\u2192", r"\ensuremath{\rightarrow}"),
    ("\u2013", "--"),
    ("\u2018", "`"),
    ("\u2019", "'"),
    ("\u201c", "``"),
    ("\u201d", "''"),
    ("\u2026", r"\ldots{}"),
    ("\u00a7", r"\S{}"),
    ("\u2260", r"\ensuremath{\neq}"),
    ("\u03b2", r"\ensuremath{\beta}"),
    ("\u03c4", r"\ensuremath{\tau}"),
]
SUPERS = {"\u2070": "0", "\u00b9": "1", "\u00b2": "2", "\u00b3": "3",
          "\u2074": "4", "\u2075": "5", "\u2076": "6", "\u2077": "7",
          "\u2078": "8", "\u2079": "9"}

def _superscript_pass(s):
    out, i, in_sup = [], 0, False
    while i < len(s):
        c = s[i]
        if c in SUPERS:
            if not in_sup:
                out.append(r"\textsuperscript{")
                in_sup = True
            out.append(SUPERS[c])
        else:
            if in_sup:
                out.append("}")
                in_sup = False
            out.append(c)
        i += 1
    if in_sup:
        out.append("}")
    return "".join(out)

def tex_escape(s):
    for a, b in [("\\", "\\textbackslash{}"), ("&", "\\&"), ("%", "\\%"),
                 ("$", "\\$"), ("#", "\\#"), ("_", "\\_"), ("{", "\\{"),
                 ("}", "\\}"), ("~", "\\textasciitilde{}"), ("^", "\\textasciicircum{}"),
                 ("<", "\\textless{}"), (">", "\\textgreater{}"),
                 ("|", "\\textbar{}")]:
        s = s.replace(a, b)
    for a, b in UNICODE_MAP:
        s = s.replace(a, b)
    # markdown in the source document: **bold**, *italic*, `code`
    s = re.sub(r"\*\*(.+?)\*\*", r"\\textbf{\1}", s)
    s = re.sub(r"(?<!\*)\*([^*]+)\*(?!\*)", r"\\emph{\1}", s)
    s = re.sub(r"`([^`]+)`", r"\\texttt{\1}", s)
    return _superscript_pass(s)

macros = {}          # name -> integer/string for the numbers macro file
def M(name, value):
    macros[name] = value

# ------------------------------------------------- compiler-comparison-full
baselines = {}       # (fn, level) -> event
optima = {}          # bench -> event
enums = {}           # bench -> summary event
searches = {}        # (bench, phase) -> event
stragglers = {}      # bench -> summary
mcmc_summaries = {}  # bench -> summary
mcmc_cfgs = []
testcases = 0
witnesses = 0
for d in load("compiler-comparison-full.jsonl"):
    k = d.get("kind")
    if k == "compiler_baseline":
        baselines[(d["function"], d["level"])] = d
    elif k == "optimum":
        optima[d["benchmark"]] = d
    elif k == "enumeration_summary":
        enums[d["benchmark"]] = d
    elif k == "search_result":
        searches[(d["benchmark"], d.get("phase"))] = d
    elif k == "straggler_summary":
        stragglers[d["benchmark"]] = d
    elif k == "mcmc_summary":
        mcmc_summaries[d["benchmark"]] = d
    elif k == "mcmc_config":
        mcmc_cfgs.append(d)
    elif k == "testcase":
        testcases += 1
    elif k == "rejection_witness":
        witnesses += 1

fns = sorted({fn for (fn, _) in baselines})
levels = ("O2", "O3")
assert len(baselines) == 46, f"expected 46 baselines, got {len(baselines)}"
assert len(fns) == 23, f"expected 23 functions, got {len(fns)}"

# gcc emitted identical assembly at both levels: verified here, not assumed
identical = all(baselines[(f, "O2")]["asm"] == baselines[(f, "O3")]["asm"] for f in fns)
assert identical, "O2/O3 assembly differs — the paper's claim would be false"
M("IdenticalLevels", "true")
M("CompilerVersion", tex_escape(baselines[(fns[0], "O2")]["compiler"]))
M("NumFunctions", len(fns))
M("NumBaselines", len(baselines))
M("NumTestcases", testcases)
M("NumWitnesses", witnesses)

# the ratio, derived exactly as audit_claims.py does
beaten, matched, honest = set(), set(), set()
for f in fns:
    beaten_here = False
    for lv in levels:
        bench = f"{f}_{lv}"
        if bench in optima and optima[bench]["length"] < optima[bench]["spec_length"]:
            beaten_here = True
        for ph in ("optimization", "seeded_optimization"):
            if (bench, ph) in searches:
                d = searches[(bench, ph)]
                if d.get("verdict", "").startswith("equivalent") and d["perf"] < d["spec_perf"]:
                    beaten_here = True
    if beaten_here:
        beaten.add(f)
    elif any(f"{f}_{lv}" in optima for lv in levels):
        matched.add(f)
    else:
        honest.add(f)
matched -= beaten
honest -= beaten | matched
assert len(beaten | matched | honest) == 23
M("NumBeaten", len(beaten))
M("NumMatched", len(matched))
M("NumHonest", len(honest))

# P-number provenance from the committed fixtures/compiler-full/candidates.c comments
pnum = {}
with open(os.path.join(REPO, "fixtures", "compiler-full", "candidates.c")) as f:
    for line in f:
        m = re.match(r"/\*\s*(P(\d+))[^:]*:.*?\*/", line)
        if m:
            pnum[m.group(1)] = None  # placeholder
# function -> Pn by scanning the comment directly above each definition
cur_p = None
with open(os.path.join(REPO, "fixtures", "compiler-full", "candidates.c")) as f:
    for line in f:
        mc = re.match(r"/\*\s*P(\d+)", line)
        if mc:
            cur_p = "P" + mc.group(1)
        mf = re.match(r"(?:u|)int64_t (\w+)\(", line)
        if mf and cur_p:
            pnum[mf.group(1)] = cur_p
            cur_p = None
CORE_EXTRAS = {"smin", "umin", "umax", "sat_add", "select_bits", "sign_mask"}

def prov(fn):
    if fn in pnum:
        return pnum[fn]
    if fn in CORE_EXTRAS:
        return "n/a"  # core-batch extras, provenance in fixtures/compiler-core/candidates.c
    raise AssertionError(f"no provenance for {fn}")

# ------------------------------------------------------- results table body
def clean_len(prog):
    return len([l for l in prog.split("\n") if l.strip() and l.strip() != "unused"])

rows = []
for f in fns:
    b2 = baselines[(f, "O2")]["asm"]
    gcc_len = len([l for l in b2.split("\n") if l.strip()])
    outc = None
    if f in beaten:
        outc = "beaten"
    elif f in matched:
        outc = "matched"
    else:
        outc = "no-improve"
    scale = ""
    if f"{f}_O2" in enums:
        e2, e3 = enums[f"{f}_O2"], enums[f"{f}_O3"]
        scale = f"{e2['candidates_total']:,} cand., {e2['z3_queries']} z3"
        if e2["candidates_total"] != e3["candidates_total"]:
            scale += f" / {e3['candidates_total']:,} at -O3"
    elif outc == "beaten":
        w = searches[(f"{f}_O2", "optimization")]
        scale = f"perf {w['spec_perf']} \u2192 {w['perf']} via MCMC (z3-equivalent)"
    else:
        s = mcmc_summaries.get(f"{f}_O2", {})
        scale = "MCMC budget, no verified improvement"
    rows.append((f, prov(f), gcc_len, outc, scale))
rows.sort(key=lambda r: (r[3] != "beaten", r[3] != "matched", r[0]))

# beaten rows carry the winner length
def winner_len(f):
    for lv in levels:
        d = searches.get((f"{f}_{lv}", "optimization"))
        if d and d.get("verdict", "").startswith("equivalent") and d["perf"] < d["spec_perf"]:
            return d["length"]
    raise AssertionError(f"no winner for beaten {f}")

tbl = []
for f, p, gl, outc, scale in rows:
    reslen = (f"{winner_len(f)}" if outc == "beaten" else
              (str(enums[f"{f}_O2"]["optimal_length"]) if outc == "matched" else str(gl)))
    o = {"beaten": "\\textcolor{beatencolor}{beaten}",
         "matched": "\\textcolor{matchedcolor}{matched}",
         "no-improve": "\\textcolor{honestcolor}{no improve}"}[outc]
    fn_t = tex_escape(f) + ("" if p == "---" else "")
    tbl.append(f"{fn_t} & {p} & {gl} & {o} & {reslen} & {tex_escape(scale)} \\\\")
def out_body(name, macro, text):
    """Emit a table body as a macro definition — \\input inside tabular
    is unreliable in modern TeX Live, macro invocation is bulletproof."""
    out(name,
        f"% generated by extract_data.py — invoke \\{macro} inside the tabular\n"
        f"\\newcommand{{\\{macro}}}{{%\n" + text + "}\n")

out_body("results-table.tex", "ResultsTableBody",
         "\n".join(tbl) + "\n")

# candidates-rejected chart data (enumerate-scoped, -O2), log scale
cand_rows = []
for f in sorted(f for f in fns if f"{f}_O2" in enums):
    cand_rows.append((f, enums[f"{f}_O2"]["candidates_total"]))
total_cands = sum(e["candidates_total"] for e in enums.values())
M("TotalCandidates", total_cands)
M("TotalOptima", len(optima))
mn = min(cand_rows, key=lambda t: t[1])
mx = max(cand_rows, key=lambda t: t[1])
M("MinCandidates", mn[1])
M("MinCandFn", tex_escape(mn[0]))
M("MaxCandidates", mx[1])
M("MaxCandFn", tex_escape(mx[0]))

# primary-pass chain budgets, from the logged wall-clock of chain_result
opt_ms, synth_ms = [], []
for d in load("compiler-comparison-full.jsonl"):
    if d.get("kind") == "chain_result":
        if d.get("phase") == "optimization" and d.get("benchmark") in {f"{f}_O2" for f in fns}:
            opt_ms.append(d["millis"])
        elif d.get("phase") == "synthesis":
            synth_ms.append(d["millis"])
def median(xs):
    xs = sorted(xs)
    return xs[len(xs) // 2]
assert opt_ms and synth_ms
M("MedianOptChainSec", round(median(opt_ms) / 1000))
M("MedianSynthChainSec", round(median(synth_ms) / 1000))

# ratio chart data: per function gcc length + outcome class, in table
# order (beaten first, then matched, then honest) with GLOBAL y indices so
# the paper's pgfplots axis reads top-down; y = N - global_index + 1
order = [(f, gl, {"beaten": "beaten", "matched": "matched",
                   "no-improve": "honest"}[o])
         for f, p, gl, o, s in rows]
names_order = [f for f, gl, cls in order]
cls_sets = {"beaten": [], "matched": [], "honest": []}
for gi, (f, gl, cls) in enumerate(order):
    y = len(order) - gi  # first row at top
    cls_sets[cls].append((gl, y))
out("ratio-chart.tex",
    "% generated by extract_data.py\n"
    "% NOTE: pgfplots assigns yticklabels bottom-up (tick 1 first), while the\n"
    "% data places the first row at the top (y = N). The label list is\n"
    "% therefore emitted REVERSED so tick 23 reads the first row.\n"
    f"\\def\\RatioNames{{{','.join(tex_escape(n) for n in reversed(names_order))}}}\n")
out("ratio-plots.tex",
    "% generated by extract_data.py: literal addplot lines, no unexpanded macros\n"
    f"\\addplot[fill=beatencolor] coordinates {{{ ' '.join(f'({x},{y})' for x, y in cls_sets['beaten'])}}};\n"
    f"\\addplot[fill=matchedcolor] coordinates {{{ ' '.join(f'({x},{y})' for x, y in cls_sets['matched'])}}};\n"
    f"\\addplot[fill=honestcolor] coordinates {{{ ' '.join(f'({x},{y})' for x, y in cls_sets['honest'])}}};\n")

# candidates chart: (global-y, name, candidates) for 13 enumerate-scoped
cand_sorted = sorted(cand_rows, key=lambda t: t[1])
cand_names = [f for f, c in cand_sorted]
cand_coords = {"matched": []}
for gi, (f, c) in enumerate(cand_sorted):
    y = len(cand_sorted) - gi
    cand_coords["matched"].append((c, y))
out("candidates-chart.tex",
    "% generated by extract_data.py: enumeration_summary.candidates_total (O2)\n"
    "% labels reversed for the same bottom-up pgfplots assignment as Fig. ratio\n"
    f"\\def\\CandNames{{{','.join(tex_escape(n) for n in reversed(cand_names))}}}\n")
out("candidates-plots.tex",
    "% generated by extract_data.py: literal addplot line\n"
    f"\\addplot[fill=matchedcolor] coordinates {{{ ' '.join(f'({c},{y})' for c, y in cand_coords['matched'])}}};\n")

# ------------------------------------------------------------- the three wins
def emit_win(fn, tag, file_tag):
    base = baselines[(fn, "O2")]["asm"]
    win = None
    for lv in levels:
        d = searches.get((f"{fn}_{lv}", "optimization"))
        if d and d.get("verdict", "").startswith("equivalent") and d["perf"] < d["spec_perf"]:
            win = d
            break
    assert win is not None, f"no winner for {fn}"
    def prog_lines(p):
        return "\n".join(l.strip() for l in p.split("\n") if l.strip() and l.strip() != "unused")
    out(f"gcc-{file_tag}.asm", prog_lines(base) + "\n")
    out(f"ours-{file_tag}.asm", prog_lines(win["program"]) + "\n")
    M(f"GccLen{tag}", clean_len(base))
    M(f"OursLen{tag}", win["length"])
    M(f"SpecPerf{tag}", win["spec_perf"])
    M(f"Perf{tag}", win["perf"])

assert beaten == {"iabs", "p13_sign", "p19_exchange_fields"}, sorted(beaten)
emit_win("iabs", "Iabs", "iabs")
emit_win("p13_sign", "Pthirteen", "p13")
emit_win("p19_exchange_fields", "Pnineteen", "p19")

# straggler outcomes
sp = sorted(stragglers.items())
assert len(sp) == 6
s0 = sp[0][1]
M("StragglerChains", s0["chains"])
M("StragglerSeconds", s0["seconds_per_chain"])
strag_tbl = []
for bench, s in sp:
    strag_tbl.append(f"{tex_escape(bench.replace('_O2',' (-O2)').replace('_O3',' (-O3)'))} & "
                     f"{s['spec_perf']} & {s['best_perf']} & {tex_escape(s['status'])} \\\\")
out_body("straggler-table.tex", "StragglerTableBody",
         "\n".join(strag_tbl) + "\n")

# synthesis-equivalent discoveries
synth = sorted({b for (b, ph) in searches if ph == "synthesis"})
M("NumSynth", len(synth))

# ---------------------------------------------------- stress-compiler-full
modes = {}
pairs = []
oracle_fns = 0
oracle_cases = 0
excl = 0
flag_comps = 0
for d in load("stress-compiler-full.jsonl"):
    k = d.get("kind")
    if k == "mode_summary":
        modes[d["mode"]] = d
    elif k == "pair_verdict":
        pairs.append(d)
    elif k == "compiler_oracle_func":
        oracle_fns += 1
    elif k == "exclusion_confirmed":
        excl += 1
M("BulkPrograms", modes["bulk"]["programs"])
M("BulkEvals", modes["bulk"]["program_evals"])
M("Z3Queries", modes["z3"]["queries"])
M("GridPrograms", modes["grid"]["programs"])
M("SelfPrograms", modes["self"]["programs"])
M("PairVerdicts", modes["pairs"]["verdicts"])
M("HwCases", modes["hw"]["cases"])
M("HwFlagComps", modes["hw"]["flag_comparisons"])
M("HwMemCases", modes["hw_mem"]["cases"])
M("HwMemDef", modes["hw_mem"]["def_cases"])
M("OracleFns", oracle_fns)
M("OracleCases", modes["compiler_oracle"]["cases"])
M("Exclusions", excl)
not_pair = next(d for d in pairs if d.get("target", "").strip() == "notq %rcx")
assert not_pair["verdict"] == "equivalent"

# ------------------------------------------------------- enumeration suite
enum_cands = 0
for d in load("enumeration-certificates.jsonl"):
    if d.get("kind") == "enumeration_summary":
        enum_cands += d["candidates_total"]
M("EnumCandidates", enum_cands)

# the enumeration regeneration chain, parsed from the committed audit script's
# own comment (the four universe generations are recorded there)
with open(os.path.join(REPO, "tools", "audit_claims.py")) as f:
    at = f.read()
chain = {c.replace(",", "") for c in re.findall(r"(278,?377|279,?088|249,?223|218,?955)", at)}
chain = {int(c) for c in chain}
assert chain == {278377, 279088, 249223, 218955}, f"regeneration chain not found: {chain}"
M("EnumChainA", 278377)
M("EnumChainB", 279088)
M("EnumChainC", 249223)
M("EnumChainD", 218955)

# ------------------------------------------------------------- recheck output
re_total = 0
re_fail = 0
for a in ("enumeration-certificates", "mcmc-search", "stress-division-memory",
          "optimize-certificates", "stress-compiler-core",
          "compiler-comparison-core", "compiler-comparison-full",
          "stress-compiler-full", "stress-log"):
    with open(os.path.join(GEN, f"recheck-{a}.txt")) as f:
        m = re.search(r"(\d+) events? re-verified exactly \(.*\), (\d+) FAILED", f.read())
        assert m, f"recheck output missing for {a} (build.sh must run recheck first)"
        re_total += int(m.group(1))
        re_fail += int(m.group(2))
M("RecheckTotal", re_total)
M("RecheckFailed", re_fail)
assert re_fail == 0, "recheck reported failures — refusing to build the paper"

# audit result
with open(os.path.join(GEN, "audit.txt")) as f:
    atxt = f.read()
m = re.search(r"AUDIT PASSED", atxt)
assert m, "audit did not pass — refusing to build the paper"
ok_count = len(re.findall(r"^  OK", atxt, re.M))
M("AuditChecks", ok_count)

# ------------------------------------------------------------- test count
n_tests = 0
for root, _, files in os.walk(os.path.join(REPO, "src")):
    for fn in files:
        if fn.endswith(".rs"):
            n_tests += open(os.path.join(root, fn)).read().count("#[test]")
for root, _, files in os.walk(os.path.join(REPO, "tests")):
    for fn in files:
        if fn.endswith(".rs"):
            n_tests += open(os.path.join(root, fn)).read().count("#[test]")
M("NumTests", n_tests)

# ------------------------------------------------------------- bug ledger
ledger = OrderedDict()
with open(os.path.join(REPO, "docs", "VERIFICATION.md")) as f:
    vt = f.read()
for mrow in re.finditer(r"^\| (\d+) \| (.+?) \| (.+?) \| (.+?) \|\s*$", vt, re.M):
    row = {}
    row["bug"] = mrow.group(2)
    row["caught"] = mrow.group(3)
    row["why"] = mrow.group(4)
    ledger[int(mrow.group(1))] = row
assert len(ledger) == 34, f"expected 34 ledger rows, got {len(ledger)}"
M("NumLedgerRows", len(ledger))

def first_sentences(s, n=1):
    parts = re.split(r"(?<=[.!?]) ", s.replace("\n", " "))
    return " ".join(parts[:n])

SELECTED = [1, 3, 8, 11, 13, 15, 17, 22, 23, 24, 32, 33, 34]
lt = []
for i in SELECTED:
    r = ledger[i]
    lt.append(
        f"{i} & {tex_escape(first_sentences(r['bug'], 1))} & "
        f"{tex_escape(first_sentences(r['caught'], 1))} & "
        f"{tex_escape(first_sentences(r['why'], 1))} \\\\")
out_body("ledger-table.tex", "LedgerTableBody",
         "\n".join(lt) + "\n")

# ------------------------------------------------------------- numbers macro file
lines = ["% generated by extract_data.py — the paper's only numeric source",
         "% every macro traces to a committed artifact or document"]
def fmt(v):
    if isinstance(v, int):
        return f"\\num[grouping-four={str(v).find(',') >= 0}]{{{v:,}}}".replace("\\num[grouping-four=False]", "")
    return v
for k, v in macros.items():
    if isinstance(v, int):
        sv = f"{v:,}".replace(",", "\\,") if v >= 10000 else str(v)
        # LaTeX macro names are letters only — sanitize digit-containing
        # keys (e.g. Z3Queries -> ZSolverQueries) before emitting.
        k = k.replace("Z3Queries", "ZSolverQueries")
        assert not any(c.isdigit() for c in k), f"macro name {k} has digits"
        lines.append(f"\\newcommand{{\\{k}}}{{{sv}}}")
    else:
        k = k.replace("Z3Queries", "ZSolverQueries")
        lines.append(f"\\newcommand{{\\{k}}}{{{v}}}")
out("numbers.tex", "\n".join(lines) + "\n")

print(f"extract OK: {len(macros)} macros, {len(tbl)} result rows, "
      f"{len(lt)} ledger rows, ratio {len(beaten)}/{len(matched)}/{len(honest)}")
