#!/usr/bin/env bash
# build.sh — regenerate the paper from the live artifacts.
#
#   1. re-check every committed artifact with `recheck` (the paper quotes
#      these live results; a FAILED anywhere aborts the build)
#   2. run the claims audit (must exit 0)
#   3. verify the bibliography provenance manifest (unconfirmed entries
#      are flagged and abort the build)
#   4. extract every number the paper quotes into generated/*.tex
#   5. compile with latexmk (or pdflatex twice)
#   6. run the house-style gate on the rendered PDF (no em dashes, no
#      banned stock constructions)
#
# Usage:  ./build.sh [--preview]
set -euo pipefail
cd "$(dirname "$0")"
mkdir -p generated

ARTIFACTS="enumeration-certificates mcmc-search stress-division-memory optimize-certificates stress-compiler-core compiler-comparison-core compiler-comparison-full stress-compiler-full stress-log"

echo "== 1/6 rechecking artifacts (live) =="
for a in $ARTIFACTS; do
  ../target/release/recheck --from-log "../artifacts/$a.jsonl" > "generated/recheck-$a.txt" 2>/dev/null
  tail -1 "generated/recheck-$a.txt"
done

echo "== 2/6 claims audit =="
(cd .. && python3 tools/audit_claims.py) > generated/audit.txt
tail -1 generated/audit.txt

echo "== 3/6 bibliography provenance =="
python3 check_bib.py

echo "== 4/6 extracting paper data from artifacts =="
python3 extract_data.py

echo "== 5/6 compiling =="
if command -v latexmk >/dev/null 2>&1; then
  latexmk -pdf -interaction=nonstopmode -halt-on-error paper.tex >/dev/null
else
  pdflatex -interaction=nonstopmode -halt-on-error paper.tex >/dev/null
  pdflatex -interaction=nonstopmode -halt-on-error paper.tex >/dev/null
fi
test -f paper.pdf || { echo "build.sh: no paper.pdf produced"; exit 1; }
[ paper.pdf -nt paper.tex ] || { echo "build.sh: paper.pdf is stale"; exit 1; }

echo "== 6/6 house-style gate (rendered PDF) =="
python3 check_style.py

if [ "${1:-}" = "--preview" ] && command -v pdftoppm >/dev/null 2>&1; then
  rm -f paper-*.png
  pdftoppm -png -r 110 paper.pdf paper
fi

echo "done: paper.pdf ($(pdfinfo paper.pdf | awk '/^Pages/{print $2}') pages)"
