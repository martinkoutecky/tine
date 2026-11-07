#!/usr/bin/env python3
"""Axiom audit (round 7: theorem names may contain dots and apostrophes; earlier
rounds matched only \\w+, so dotted and primed names were collapsed or dropped) of every named theorem in src/ (run after `lake build`).

1. Source scan: no sorry/admit/native_decide/axiom/unsafe/implemented_by/extern/ofReduceBool/sorryAx in code.
2. Generates .lake/AuditAll.lean that imports every module and `#print axioms` for every named theorem
   (namespaces tracked from `namespace`/`end` lines), runs it with `lake env lean`, and requires each
   theorem's axioms to be a subset of {propext, Classical.choice, Quot.sound}.
Headlines.lean additionally checks the raw dependency cone of the headline theorems during `lake build`.
"""
import os, re, resource, subprocess, sys
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
    # Track namespaces in code, not prose lines inside block comments.
    for line in code.splitlines():
        if m := re.match(r'^namespace (\w+)', line):
            stack.append(m.group(1))
        if (m := re.match(r'^end (\w+)', line)) and stack and stack[-1] == m.group(1):
            stack.pop()
        if m := re.match(r"^\s*(?:@\[[^\n]*\]\s*)?(?:private\s+)?(?:theorem|lemma)\s+([\w.']+)", line):
            if 'private' not in line.split(m.group(1))[0]:
                names.append('.'.join(stack + [m.group(1)]))
names = sorted(set(names))
# Every public headline and both current witness suites must be roots of the
# checked cone, not merely covered by the separate theorem-axiom scan.
cone_src = '\n'.join((ROOT / 'src' / f).read_text()
                     for f in ('Headlines.lean', 'Fold5.lean', 'OwnerWitness5.lean', 'QueryWitness6.lean'))
cone_roots = set(re.findall(r'``(CM\.(?:Headlines|Witnesses3|Witnesses4|Policy5|Witnesses5|Fold5|Owner5|OwnerWitness5|QueryRename6|QueryWitness6)\.\w+)', cone_src))
required_roots = {n for n in names if n.startswith(('CM.Headlines.', 'CM.Witnesses3.', 'CM.Witnesses4.',
                                                   'CM.Policy5.', 'CM.Witnesses5.', 'CM.Fold5.',
                                                   'CM.Owner5.', 'CM.OwnerWitness5.', 'CM.QueryRename6.', 'CM.QueryWitness6.'))}
if missing_roots := required_roots - cone_roots:
    sys.exit(f'missing headline cone roots: {sorted(missing_roots)}')
print(f'CONE COVERAGE PASS: {len(required_roots)} headline/witness roots')
# Shared-host resource bound: dependency width bounds the number of local module
# compilers that Lake can have active, even though this Lake has no build -j flag.
deps = {m: set(re.findall(r'^import (\w+)', (ROOT / 'src' / f'{m}.lean').read_text(), re.M)) & set(mods)
        for m in mods}
closure = {}
for m in mods:
    reached, pending = set(deps[m]), list(deps[m])
    while pending:
        for d in deps[pending.pop()]:
            if d not in reached:
                reached.add(d)
                pending.append(d)
    if m in reached:
        sys.exit(f'import cycle at {m}')
    closure[m] = reached
matched = {}
def augment(m, seen):
    for d in closure[m]:
        if d in seen:
            continue
        seen.add(d)
        if d not in matched or augment(matched[d], seen):
            matched[d] = m
            return True
    return False
for m in mods:
    augment(m, set())
width = len(mods) - len(matched)
if width > 2:
    sys.exit(f'module dependency width {width} exceeds the shared-host limit of 2')
print(f'BUILD WIDTH PASS: {width} concurrent local module compilers at most')
audit = ROOT / '.lake' / 'AuditAll.lean'
audit.parent.mkdir(exist_ok=True)
audit.write_text(''.join(f'import {m}\n' for m in mods) + ''.join(f'#print axioms {n}\n' for n in names))
# The round-5 task caps non-Lean checks at 2 GiB but explicitly requires Lean
# uncapped. Run this Python process with `ulimit -S -v 2097152`; restore only
# the Lean child's soft address-space limit, leaving Python capped.
def uncap_lean():
    _, hard = resource.getrlimit(resource.RLIMIT_AS)
    if hard != resource.RLIM_INFINITY:
        raise RuntimeError('Lean requires an unlimited hard address-space limit')
    resource.setrlimit(resource.RLIMIT_AS, (resource.RLIM_INFINITY, hard))

p = subprocess.run(['lake', 'env', 'lean', '-j2', '-s8192', str(audit)], cwd=ROOT,
                   env={**os.environ, 'LEAN_NUM_THREADS': '2'}, capture_output=True, text=True,
                   preexec_fn=uncap_lean)
out = p.stdout + p.stderr
if p.returncode:
    sys.exit(out[-4000:])
printed = {n: [a.strip() for a in axs.split(',') if a.strip()]
           for n, axs in re.findall(r"^'(.+)' depends on axioms: \[([^\]]*)\]", out, re.M)}
printed.update({n: [] for n in re.findall(r"^'(.+)' does not depend on any axioms", out, re.M)})
missing = set(names) - set(printed)
extra = {n: a for n, a in printed.items() if not set(a) <= ALLOWED}
if missing or extra:
    sys.exit(f'missing={sorted(missing)[:20]} nonstandard={extra}')
print(f'AXIOM AUDIT PASS: {len(mods)} modules, {len(names)} named theorems, axioms within {sorted(ALLOWED)}')
