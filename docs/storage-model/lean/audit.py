#!/usr/bin/env python3
"""Axiom audit of every named theorem in src/ (run after `lake build`).

1. Source scan: no sorry/admit/native_decide/axiom/unsafe/implemented_by/extern/ofReduceBool/sorryAx in code.
2. Generates .lake/AuditAll.lean that imports every module and `#print axioms` for every named theorem
   (namespaces tracked from `namespace`/`end` lines), runs it with `lake env lean`, and requires each
   theorem's axioms to be a subset of {propext, Classical.choice, Quot.sound}.
Headlines.lean additionally checks the raw dependency cone of the headline theorems during `lake build`.
"""
import re, subprocess, sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent
ALLOWED = {'propext', 'Classical.choice', 'Quot.sound'}
FORBIDDEN = r'\b(sorry|admit|native_decide|axiom|unsafe|implemented_by|extern|ofReduceBool|sorryAx)\b'
mods, names = [], []
for f in sorted((ROOT / 'src').glob('*.lean')):
    src = f.read_text()
    code = re.sub(r'/-.*?-/', '', src, flags=re.S)
    code = re.sub(r'--[^\n]*', '', code)
    code = re.sub(r'"(?:[^"\\]|\\.)*"', '', code)
    code = re.sub(r'#print axioms', '', code)
    bad = re.search(FORBIDDEN, code)
    if bad:
        sys.exit(f'forbidden token {bad.group(0)!r} in {f.name}')
    mods.append(f.stem)
    stack = []
    for line in src.splitlines():
        if m := re.match(r'^namespace (\w+)', line):
            stack.append(m.group(1))
        if (m := re.match(r'^end (\w+)', line)) and stack and stack[-1] == m.group(1):
            stack.pop()
        if m := re.match(r'^\s*(?:@\[[^\n]*\]\s*)?(?:private\s+)?(?:theorem|lemma)\s+(\w+)', line):
            if 'private' not in line.split(m.group(1))[0]:
                names.append('.'.join(stack + [m.group(1)]))
names = sorted(set(names))
audit = ROOT / '.lake' / 'AuditAll.lean'
audit.parent.mkdir(exist_ok=True)
audit.write_text(''.join(f'import {m}\n' for m in mods) + ''.join(f'#print axioms {n}\n' for n in names))
p = subprocess.run(['lake', 'env', 'lean', str(audit)], cwd=ROOT, capture_output=True, text=True)
out = p.stdout + p.stderr
if p.returncode:
    sys.exit(out[-4000:])
printed = {n: [a.strip() for a in axs.split(',') if a.strip()]
           for n, axs in re.findall(r"'([^']+)' depends on axioms: \[([^\]]*)\]", out)}
printed.update({n: [] for n in re.findall(r"'([^']+)' does not depend on any axioms", out)})
missing = set(names) - set(printed)
extra = {n: a for n, a in printed.items() if not set(a) <= ALLOWED}
if missing or extra:
    sys.exit(f'missing={sorted(missing)[:20]} nonstandard={extra}')
print(f'AXIOM AUDIT PASS: {len(mods)} modules, {len(names)} named theorems, axioms within {sorted(ALLOWED)}')
