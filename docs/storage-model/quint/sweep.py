#!/usr/bin/env python3
"""Scenario and mutant sweep for one model version.

    QNT=storage-s35.qnt SCEN=scenarios-s35.inc python3 sweep.py

1. Every scenario passes on the unmutated model in each race profile (base, R1, weak, all).
2. Each mutant listed in the model's `// SWEEP-MUTS: {...}` line (mutant -> profile) is CAUGHT: its own
   scenario `m<MUTANT>` fails with QNT508 (an assertion/invariant failure, not merely a disabled step).
Exit status 0 only if both hold.
"""
import ast, os, re, sys
import run

src = (run.ROOT / run.model()).read_text()
MUTS = ast.literal_eval(re.search(r'// SWEEP-MUTS: (\{.*\})', src).group(1))
m = re.search(r'// SWEEP-CONTROL: (\{.*\})', src)
CONTROL = ast.literal_eval(m.group(1)) if m else {}


def errors(out):
    return dict(re.findall(r'\n\s+\d+\) (\w+):\n\s+Error \[(QNT\d+)\]', out))


ok = True
for prof in ['base', 'R1', 'weak', 'all']:
    _, p, f = run.test(prof, 'none')
    ok &= not f and bool(p)
    print(f'spec[{prof}]: {len(p)} pass, failed={f}', flush=True)
caught = 0
for mut, prof in MUTS.items():
    out, p, f = run.test(prof, mut)
    hit = errors(out).get('m' + mut) == 'QNT508'
    caught += hit
    ok &= hit
    print(f'{mut:4} [{prof}] m{mut}: {"CAUGHT" if hit else "MISSED " + str(errors(out).get("m" + mut))}', flush=True)
for mut, prof in CONTROL.items():
    out, p, f = run.test(prof, mut)
    print(f'{mut:4} [{prof}] m{mut} (control): {"caught" if errors(out).get("m" + mut) == "QNT508" else "missed"}')
print(f'SWEEP {"PASS" if ok else "FAIL"}: {run.model()}; {caught}/{len(MUTS)} mutants caught', flush=True)
sys.exit(0 if ok else 1)
