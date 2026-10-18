"""Builds term.html (animated terminal) from real outputs in WORK:
  WORK/demo.log     KEEP=1 examples/demo.sh > demo.log
  WORK/i18nize.txt  glot i18nize --dry-run app/page.tsx (in a fresh `glot init`ed copy of demo-saas)
Usage: python3 build-term.py WORK   then   node term.mjs WORK
ponytail: line ranges below match the 0.2.1 outputs; adjust them if the output changes."""
import json, re, sys

W = sys.argv[1]
strip = lambda t: re.sub(r"\x1b\[[0-9;]*m", "", t)
demo = strip(open(f"{W}/demo.log").read()).split("\n")
dry = open(f"{W}/i18nize.txt").read().split("\n")
ex = demo.index("$ glot extract")
iz = demo.index("$ glot i18nize --yes --switcher")
keep = ("Next.js", "Scanning", "27 files", "i18n runtime", "  created", "  edited", "✓ done")
steps = [
    ("glot extract", demo[ex + 1 : ex + 10] + ["  …"]),
    ("glot i18nize --dry-run app/page.tsx", [dry[202]] + dry[216:248] + [""] + dry[257:263] + ["  …", "", dry[276]]),
    ("glot i18nize --yes", [l for l in demo[iz + 1 : iz + 40] if l.startswith(keep)]),
    ("glot extract", ["31 files scanned", "✓ nothing to report"]),
]

def cls(l):
    if l.startswith("+") and not l.startswith("+++"): return "add"
    if l.startswith("-") and not l.startswith("---"): return "del"
    if l.startswith("@@"): return "hunk"
    if l.startswith("  + "): return "key"
    if "✓" in l: return "ok"
    if re.match(r"^(Hardcoded|\d+ files to modify)", l): return "bold"
    return ""

data = [{"cmd": c, "out": [[l, cls(l)] for l in o]} for c, o in steps]
tpl = open(__file__.replace("build-term.py", "term.html")).read()
open(f"{W}/term.html", "w").write(tpl.replace("__DATA__", json.dumps(data)))
