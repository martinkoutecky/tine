#!/usr/bin/env python3
"""Hash-linked s3.6 bounded policy/creation endpoints, directly from Quint ITF.

Run at repository root: rtk proxy python3 docs/storage-model/replay/replay36.py
Use --check to regenerate and compare without rewriting the committed fixture.
Full ITF/logs stay in artifacts/sb1-model/. This supplement does not replace the
s3.5 transcription, full-state conformance corpus or its Lean proof.
"""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'docs/storage-model/quint'))
import run

NAMES = [
    'ownPlainSlotRetained', 'ownPlainSlotLeaves', 'ownPlainSlotUnrelated',
    'ownPlainSlotLate', 'ownProgressSave', 'ownProgressConflict',
    'ownProgressFailure', 'ownProgressRefresh', 'ownProgressThree',
]


def decode(v):
    if isinstance(v, list):
        return [decode(x) for x in v]
    if not isinstance(v, dict):
        return v
    if '#bigint' in v:
        return int(v['#bigint'])
    if '#map' in v:
        pairs = sorted(v['#map'], key=lambda kv: decode(kv[0]))
        assert [decode(k) for k, _ in pairs] == list(range(len(pairs)))
        return [decode(x) for _, x in pairs]
    if '#set' in v:
        return sorted(decode(x) for x in v['#set'])
    if '#tup' in v:
        return decode(v['#tup'])
    return {k: decode(x) for k, x in v.items() if k != '#meta'}


def main():
    os.environ.update(QNT='storage-s36.qnt', SCEN='scenarios-s36.inc')
    run.OUT = ROOT / 'artifacts/sb1-model/quint-replay'
    fixture = {key: hashlib.sha256((run.ROOT / file).read_bytes()).hexdigest()
               for key, file in [('model_sha256', 'storage-s36.qnt'),
                                 ('scenarios_sha256', 'scenarios-s36.inc')]}
    fixture['scope'] = 'bounded semantic endpoints; default driver supplies scheduling; not full-state refinement'
    cases = []
    for profile in run.PROFILES:
        variant = run.variant(profile, 'none')
        pattern = run.OUT / f'{profile}-{{test}}-{{seq}}.itf.json'
        result = subprocess.run([
            'rtk', 'proxy', run.Q, 'test', str(variant), '--backend', 'typescript',
            '--max-samples', '1', '--match', '^(' + '|'.join(NAMES) + ')$',
            '--out-itf', str(pattern)], capture_output=True, text=True, env=run._env())
        (run.OUT / f'{profile}.log').write_text(result.stdout + result.stderr)
        assert result.returncode == 0, result.stdout + result.stderr
        for name in NAMES:
            paths = list(run.OUT.glob(f'{profile}-{name}-*.itf.json'))
            assert len(paths) == 1, paths
            trace = json.loads(paths[0].read_text())
            assert trace['#meta']['status'] == 'passed'
            states = decode(trace['states'])
            # Quint appends an empty state for a successful terminal .fail().
            if name == 'ownPlainSlotRetained':
                assert states[-1] == {}
                states.pop()
            assert all('s' in state and 'g' in state for state in states)
            s = states[-1]['s']
            assert not states[-1]['g']['bad']
            cases.append(dict(name=name, profile=profile, model_steps=len(states)-1,
                              endpoint={key: s[key] for key in
                                        ['disk', 'stable', 'claimed', 'pages', 'drafts']}))
    fixture['cases'] = cases
    target = ROOT / 'crates/tine-store/tests/fixtures/s2/s36-policy.json'
    content = json.dumps(fixture, indent=2) + '\n'
    if '--check' in sys.argv:
        assert target.read_text() == content, 's3.6 fixture drift; regenerate and review'
    else:
        target.write_text(content)
    print(f's3.6 Quint endpoints: {len(cases)} cases, {sum(c["model_steps"] for c in cases)} model steps; {fixture["model_sha256"]}')


if __name__ == '__main__':
    main()
