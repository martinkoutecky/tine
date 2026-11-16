#!/usr/bin/env python3
"""s3.6 vs s3.5: unchanged old scenarios x every profile x every old mutant.

Only the two precisely named claimed-creating-flush refusals may differ.
No scenario, invariant, mutant or bound is changed to perform this comparison.
Run at repository root; raw Quint outputs stay under artifacts/sb1-model/cons36.
"""
import ast
from concurrent.futures import ProcessPoolExecutor
import os
from pathlib import Path
import re
import run


def compare(case):
    profile, mutant = case
    run.OUT = run.ROOT.parents[2] / 'artifacts/sb1-model/cons36'
    os.environ['SCEN'] = 'scenarios-s35.inc'
    results = []
    for model in ['storage-s35.qnt', 'storage-s36.qnt']:
        os.environ['QNT'] = model
        output, passed, failed = run.test(profile, mutant)
        errors = dict(re.findall(r'\n\s+\d+\) (\w+):\n\s+Error \[(QNT\d+)\]', output))
        assert set(errors) == set(failed), (model, case, failed, errors)
        outcomes = {**dict.fromkeys(passed, 'pass'), **errors}
        assert len(outcomes) == 194, (model, case, len(outcomes))
        results.append(outcomes)
    old, new = results
    diffs = {name: (old[name], new[name]) for name in old if old[name] != new[name]}
    expected = {'ownSlotClaimAfterAdmission': ('pass', 'QNT513')}
    if mutant == 'MNS':
        expected['ownSlotMutantTwoFiles'] = ('QNT508', 'QNT513')
    assert diffs == expected, (case, diffs, expected)
    return f'{profile}/{mutant}: 194 outcomes, {len(diffs)} intended refusals, 0 other differences', len(diffs)


if __name__ == '__main__':
    text = (run.ROOT / 'storage-s35.qnt').read_text()
    mutants = ast.literal_eval(re.search(r'// SWEEP-MUTS: (\{.*\})', text).group(1))
    cases = [(profile, mutant) for mutant in ['none', *mutants] for profile in run.PROFILES]
    assert len(cases) == 156
    with ProcessPoolExecutor(max_workers=4) as pool:
        total = 0
        for message, differences in pool.map(compare, cases):
            total += differences
            print(message, flush=True)
    assert total == 160
    print(f'PASS: {len(cases)} cases x 194 scenarios = {len(cases)*194} comparisons; '
          f'{total} intended new refusals; 0 unexpected differences')
