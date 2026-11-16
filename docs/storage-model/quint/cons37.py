#!/usr/bin/env python3
"""s3.7-gitpull vs s3.6: unchanged s3.6 scenarios x every profile x every s3.6 mutant.

The git layer must be conservative: every outcome (pass, or the exact Quint error code) is identical.
No scenario, invariant, mutant or bound is changed to perform this comparison.
Raw Quint outputs stay under artifacts/gs-model/cons37 (repository root). WORKERS (default 3) sets parallelism.
"""
import ast
from concurrent.futures import ProcessPoolExecutor
import os
import re
import run


def compare(case):
    profile, mutant = case
    run.OUT = run.ROOT.parents[2] / 'artifacts/gs-model/cons37'
    os.environ['SCEN'] = 'scenarios-s36.inc'
    results = []
    for model in ['storage-s36.qnt', 'storage-s37-gitpull.qnt']:
        os.environ['QNT'] = model
        output, passed, failed = run.test(profile, mutant)
        errors = dict(re.findall(r'\n\s+\d+\) (\w+):\n\s+Error \[(QNT\d+)\]', output))
        assert set(errors) == set(failed), (model, case, failed, errors)
        results.append({**dict.fromkeys(passed, 'pass'), **errors})
    old, new = results
    assert set(old) == set(new) and old, (case, len(old), len(new))
    diffs = {name: (old[name], new[name]) for name in old if old[name] != new[name]}
    assert not diffs, (case, diffs)
    return f'{profile}/{mutant}: {len(old)} outcomes, 0 differences', len(old)


if __name__ == '__main__':
    text = (run.ROOT / 'storage-s36.qnt').read_text()
    mutants = ast.literal_eval(re.search(r'// SWEEP-MUTS: (\{.*\})', text).group(1))
    cases = [(profile, mutant) for mutant in ['none', *mutants] for profile in run.PROFILES]
    total = 0
    with ProcessPoolExecutor(max_workers=int(os.environ.get('WORKERS', '3'))) as pool:
        for message, n in pool.map(compare, cases):
            total += n
            print(message, flush=True)
    print(f'PASS: {len(cases)} cases, {total} scenario outcomes compared; 0 differences')
