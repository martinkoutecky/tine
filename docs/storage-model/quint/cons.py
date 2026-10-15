#!/usr/bin/env python3
"""Conservativity check by scenario outcomes (evidence, not a proof; the Lean conservativity theorems are the proof).

    python3 cons.py OLD.qnt NEW.qnt OLD-SCENARIOS.inc [--two-pages] [--full]

Runs the OLD model's scenarios on OLD and on NEW and requires the same per-scenario outcome (pass, or the same
Quint error code) in every race profile, unmutated and under every mutant of OLD's `SWEEP-MUTS` line (each
under its own profile; with --full, every mutant under every profile). --two-pages restricts NEW's PAGES to
{0, 1}, as needed when OLD is an s2-family model (two pages) and NEW an s3-family model (three). Examples:

    python3 cons.py storage-s2x.qnt storage-s3.qnt  scenarios-s2.inc --two-pages    # s3   vs s2.1
    python3 cons.py storage-s33.qnt storage-s34.qnt scenarios-s33.inc --full        # s3.4 vs s3.3
    python3 cons.py storage-s34.qnt storage-s35.qnt scenarios-s34.inc --full        # s3.5 vs s3.4
"""
import ast, os, re, sys
from pathlib import Path
import run

args = [a for a in sys.argv[1:] if not a.startswith('--')]
if len(args) != 3:
    sys.exit(__doc__)
old, new, scen = args
gen = run.OUT / 'cons'
gen.mkdir(parents=True, exist_ok=True)
new_text = (run.ROOT / new).read_text()
if '--two-pages' in sys.argv:
    assert 'pure val PAGES = Set(0, 1, 2)' in new_text, 'NEW model does not declare PAGES = Set(0, 1, 2)'
    new_text = new_text.replace('pure val PAGES = Set(0, 1, 2)', 'pure val PAGES = Set(0, 1)', 1)
restricted = gen / Path(new).name.replace('.qnt', '-cons.qnt')
restricted.write_text(new_text)
os.environ['SCEN'] = str((run.ROOT / scen).resolve())
muts = ast.literal_eval(re.search(r'// SWEEP-MUTS: (\{.*\})', (run.ROOT / old).read_text()).group(1))
PROFS = ['base', 'R1', 'weak', 'all']
cases = [(p, 'none') for p in PROFS] + ([(p, m) for m in muts for p in PROFS] if '--full' in sys.argv
                                        else [(p, m) for m, p in muts.items()])
expected = len(re.findall(r'^\s*run (?:max|own|q|m)[A-Z]\w*', (run.ROOT / scen).read_text(), re.M))


def outcome(qnt, prof, mut):
    os.environ['QNT'] = str(qnt)
    out, passed, failed = run.test(prof, mut)
    errs = dict(re.findall(r'\n\s+\d+\) (\w+):\n\s+Error \[(QNT\d+)\]', out))
    return {**{n: 'pass' for n in passed}, **{n: errs.get(n, 'fail') for n in failed}}


diffs = 0
for prof, mut in cases:
    a = outcome((run.ROOT / old).resolve(), prof, mut)
    b = outcome(restricted.resolve(), prof, mut)
    assert len(a) == expected and len(b) == expected, (prof, mut, len(a), len(b), expected)
    d = {k: (a.get(k), b.get(k)) for k in set(a) | set(b) if a.get(k) != b.get(k)}
    diffs += len(d)
    print(f'{prof:5} {mut:5} scenarios={len(a)} same={not d} {d if d else ""}', flush=True)
print(f'TOTAL DIFFS {diffs} ({len(cases)} cases x {expected} scenarios)')
sys.exit(1 if diffs else 0)
