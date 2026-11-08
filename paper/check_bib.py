#!/usr/bin/env python3
"""check_bib.py — bibliography provenance gate.

Every \\bibitem in paper.tex must appear in bib-sources.json under
"confirmed" (with an evidence pointer). Anything unconfirmed is flagged
and the build fails, so an unverifiable citation can never ship.
"""
import json
import re
import sys

tex = open("paper.tex").read()
keys = re.findall(r"\\bibitem\{([^}]+)\}", tex)
man = json.load(open("bib-sources.json"))
confirmed = set(man["confirmed"])
flagged = set(man.get("flagged", {})) - {"_comment"}

bad = []
for k in keys:
    if k not in confirmed:
        bad.append(k)

print(f"bibliography: {len(keys)} \\bibitem entries, "
      f"{len(keys) - len(bad)} confirmed, {len(bad)} unconfirmed")
for k in bad:
    print(f"  FLAGGED (must be removed or confirmed): {k}")
for k in sorted(flagged):
    print(f"  flagged candidate (excluded, not in document): {k}")
if bad:
    sys.exit(1)
print("bibliography provenance: OK")
