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
caught = 0
scenario = os.environ.get('SCEN', 'scenarios-s35.inc')
vehicle = 'scenarios-vehicle-recovery.inc'
vehicle_source = (run.ROOT / vehicle).read_text()
suites = [(scenario, os.environ.get('SCENARIO_PAGES', '3'))]
if scenario != vehicle and any(re.search(rf'\brun m{m}\b', vehicle_source) for m in MUTS):
    suites.append((vehicle, '5'))
selected = set()
for source, pages in suites:
    os.environ.update(SCEN=source, SCENARIO_PAGES=pages)
    cases = (run.ROOT / source).read_text()
    for prof in run.PROFILES:
        _, p, f = run.test(prof, 'none')
        ok &= not f and bool(p)
        print(f'{source}[{prof}]: {len(p)} pass, failed={f}', flush=True)
    for mut, prof in MUTS.items():
        if not re.search(rf'\brun m{mut}\b', cases):
            continue
        selected.add(mut)
        out, _, _ = run.test(prof, mut)
        hit = errors(out).get('m' + mut) == 'QNT508'
        caught += hit
        ok &= hit
        print(f'{mut:4} [{prof}] m{mut}: {"CAUGHT" if hit else "MISSED " + str(errors(out).get("m" + mut))}', flush=True)
    for mut, prof in CONTROL.items():
        if re.search(rf'\brun m{mut}\b', cases):
            out, _, _ = run.test(prof, mut)
            print(f'{mut:4} [{prof}] m{mut} (control): {"caught" if errors(out).get("m" + mut) == "QNT508" else "missed"}')
ok &= selected == MUTS.keys()
print(f'SWEEP {"PASS" if ok else "FAIL"}: {run.model()}; {caught}/{len(MUTS)} mutants caught; missing={MUTS.keys() - selected}', flush=True)
sys.exit(0 if ok else 1)
