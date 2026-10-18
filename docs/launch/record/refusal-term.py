"""Builds WORK/term.html from WORK/refusal.log (written by refusal.sh). Then: node term.mjs WORK
LABEL overrides the title bar (e.g. LABEL="first pass injected, retries by claude-haiku-5-5")."""
import json, os, sys

W = sys.argv[1]
steps, cur = [], None
for l in open(f"{W}/refusal.log").read().rstrip("\n").split("\n"):
    if l.startswith("$ "):
        cur = {"cmd": l[2:], "out": []}
        steps.append(cur)
        continue
    k = "del" if ("✗" in l or "↳" in l) else "ok" if "✓" in l else "bold" if l.startswith("Plan:") else ""
    cur["out"].append([l, k])

label = os.environ.get("LABEL", "scripted LLM, wrong on purpose on the first pass; glot is the real binary")
tpl = open(__file__.replace("refusal-term.py", "term.html")).read()
tpl = tpl.replace("white-space:pre}", "white-space:pre-wrap}").replace("~/acme-invoices — glot (mock provider)", "~/demo — " + label)
open(f"{W}/term.html", "w").write(tpl.replace("__DATA__", json.dumps(steps)))
