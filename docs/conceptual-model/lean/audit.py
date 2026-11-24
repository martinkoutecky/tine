#!/usr/bin/env python3
"""Axiom audit (round 7: theorem names may contain dots and apostrophes; earlier
rounds matched only \\w+, so dotted and primed names were collapsed or dropped) of every named theorem in the conceptual model and shared base (run after `lake build`).

1. Source scan: no sorry/admit/native_decide/axiom/unsafe/implemented_by/extern/ofReduceBool/sorryAx in code.
2. Generates .lake/AuditAll.lean that imports every module and `#print axioms` for every named theorem
   (namespaces tracked from `namespace`/`end` lines), runs it with `lake env lean`, and requires each
   theorem's axioms to be a subset of {propext, Classical.choice, Quot.sound}.
CutHeadlines.lean additionally checks the raw dependency cone of the headline theorems during `lake build`.
"""
import os, re, subprocess, sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent
BASE_SRC = ROOT.parent.parent / 'model-base' / 'lean' / 'src'
ALLOWED = {'propext', 'Classical.choice', 'Quot.sound'}
FORBIDDEN = r'\b(sorry|admit|native_decide|axiom|unsafe|implemented_by|extern|ofReduceBool|sorryAx)\b'
mods, names = [], []
local_sources = sorted((ROOT / 'src').glob('*.lean'))
sources = local_sources + sorted(BASE_SRC.glob('*.lean'))
for f in sources:
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
# Every public theorem and named witness must be a root of the checked cone.
cone_src = (ROOT / 'src' / 'CutHeadlines.lean').read_text()
cone_roots = set(re.findall(r"``((?:CM|ListedDisk)[\w.']+)", cone_src))
required_roots = set(names)
if missing_roots := required_roots - cone_roots:
    sys.exit(f'missing headline cone roots: {sorted(missing_roots)}')
print(f'CONE COVERAGE PASS: {len(required_roots)} headline/witness roots')
# Three layers: core imports neither views nor chapters. Evidence comparators
# are chapter-side roots, never a way for core to acquire an O8 dependency.
view_modules = {'Pages','Query','Links','Decided','Consolidated13Static','Presentation13','DateViews','DateComparison13','Ref10'}
chapter_modules = {'Journal14','CutSettings','CutSettingsExamples','CutRefactorProofs'}
core_roots = {'CutPatch','CutConsequences','CutAliasProofs','CutTwins','CutConservation','CutSafetyProofs','CutPublish','CutSplice','CutDecision','CutRedundancy','CutNames','CutObserve','CutTypes','ReadingEffects','Date13'}
all_imports = {p.stem: set(re.findall(r'^import ([\w.]+)',p.read_text(),re.M)) for p in sources}
def dependency_cone(roots):
    reached, todo = set(), list(roots)
    while todo:
        m = todo.pop()
        if m in reached: continue
        reached.add(m)
        todo.extend(all_imports.get(m,set()))
    return reached
reached = dependency_cone(core_roots)
if reached & (view_modules | chapter_modules):
    sys.exit(f'core imports views/chapters: {sorted(reached & (view_modules | chapter_modules))}')
if not {'Journal14','CutSettings'} <= all_imports.keys():
    sys.exit('a retained chapter is missing')
if not {'Ref10','DateViews'} <= all_imports.keys():
    sys.exit('a retained tree view is missing')
print(f'LAYER SEPARATION PASS: {len(reached)} core modules, no views or chapters; both chapters present')
# Shared-host resource bound for K's local module compilers. The shared base
# has its own Lake target and is audited above.
local_mods = {p.stem for p in local_sources}
deps = {m: all_imports[m] & local_mods for m in local_mods}
closure = {}
for m in sorted(local_mods):
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
for m in sorted(local_mods):
    augment(m, set())
width = len(local_mods) - len(matched)
if width > 2:
    sys.exit(f'module dependency width {width} exceeds the shared-host limit of 2')
print(f'BUILD WIDTH PASS: {width} concurrent local module compilers at most')
audit = ROOT / '.lake' / 'AuditAll.lean'
audit.parent.mkdir(exist_ok=True)
audit.write_text(''.join(f'import {m}\n' for m in mods) + ''.join(f'#print axioms {n}\n' for n in names))
p = subprocess.run(['lake', 'env', 'lean', '-j2', '-s8192', str(audit)], cwd=ROOT,
                   env={**os.environ, 'LEAN_NUM_THREADS': '2'}, capture_output=True, text=True)
out = p.stdout + p.stderr
(ROOT / '.lake' / 'axioms.log').write_text(out)
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
