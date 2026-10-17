#!/usr/bin/env python3
"""Run one Quint storage model under a race profile and a mutant.

    QNT=storage-s35.qnt SCEN=scenarios-s35.inc INV='guarantee and orderHolds and renameResolves' \
        python3 run.py PROFILE MUTANT SAMPLES STEPS [SEED]      # random simulation

PROFILE is base | R1 | weak | all; MUTANT is none or a name from the model's `SWEEP-MUTS` line.
The model is instantiated by rewriting its RACES and MUTANT constants and splicing the scenario file in at
`// @@SCENARIOS@@`; the variant is written under OUT (default ./.out), never over the model.
Quint: $QUINT, else ./node_modules/.bin/quint (`npm ci` here), else `quint` on PATH. Tested with Quint 0.33.0.
"""
import json, os, re, shutil, subprocess, sys, time
from pathlib import Path

ROOT = Path(__file__).resolve().parent
OUT = Path(os.environ.get('OUT', ROOT / '.out')).resolve()
Q = os.environ.get('QUINT') or (str(ROOT / 'node_modules/.bin/quint') if (ROOT / 'node_modules/.bin/quint').exists()
                                else shutil.which('quint') or 'quint')
PROFILES = {
    'base': 'Set("crash", "power")',
    'R1':   'Set("crash", "power", "R1")',
    'weak': 'Set("crash", "power", "weak")',
    'all':  'Set("crash", "power", "R1", "weak")',
}
SCENARIO_MATCH = '^(max|own|q|m)[A-Z]'   # every scenario name in the published scenario files


def model():
    return os.environ.get('QNT', 'storage-s35.qnt')


def variant(profile, mutant):
    t = (ROOT / model()).read_text()
    t = re.sub(r'pure val RACES: Set\[str\] = .*', f'pure val RACES: Set[str] = {PROFILES[profile]}', t, count=1)
    t = re.sub(r'pure val MUTANT: str = ".*"', f'pure val MUTANT: str = "{mutant}"', t, count=1)
    t = t.replace('  // @@SCENARIOS@@', (ROOT / os.environ.get('SCEN', 'scenarios-s35.inc')).read_text())
    d = OUT / 'gen'
    d.mkdir(parents=True, exist_ok=True)
    f = d / f'{Path(model()).stem}-{profile}-{mutant}.qnt'
    f.write_text(t)
    return f


def _env():
    (OUT / 'tmp').mkdir(parents=True, exist_ok=True)
    return {**os.environ, 'TMPDIR': str(OUT / 'tmp')}


def run(profile, mutant, samples, steps, seed=None, inv=None):
    """Random simulation with `quint run`, checking invariant INV (default `guarantee`)."""
    inv = inv or os.environ.get('INV', 'guarantee')
    f = variant(profile, mutant)
    logs = OUT / 'logs'
    logs.mkdir(parents=True, exist_ok=True)
    stem = f'{Path(model()).stem}-{profile}-{mutant}-{samples}x{steps}'
    cmd = [Q, 'run', str(f), '--invariant', inv, '--max-samples', str(samples), '--max-steps', str(steps),
           '--backend', 'typescript', '--verbosity', '1', '--out-itf', str(logs / f'{stem}.itf.json')]
    if seed:
        cmd += ['--seed', seed]
    t = time.time()
    r = subprocess.run(cmd, capture_output=True, text=True, env=_env())
    out = r.stdout + r.stderr
    (logs / f'{stem}.log').write_text(out)
    m = re.search(r'Use --seed=(\S+)', out)
    return dict(model=model(), profile=profile, mutant=mutant, samples=samples, steps=steps,
                violated='[violation]' in out, exit=r.returncode, seconds=round(time.time() - t, 1),
                seed=m.group(1) if m else None, log=str(logs / f'{stem}.log'))


def test(profile, mutant):
    """Run every scenario (`quint test`) on one profile/mutant variant. Returns (output, passed, failed)."""
    f = variant(profile, mutant)
    r = subprocess.run([Q, 'test', str(f), '--backend', 'typescript', '--max-samples', '1', '--verbosity', '2',
                        '--match', SCENARIO_MATCH], capture_output=True, text=True, env=_env())
    out = r.stdout + r.stderr
    (OUT / 'logs').mkdir(parents=True, exist_ok=True)
    (OUT / 'logs' / f'test-{Path(model()).stem}-{profile}-{mutant}.log').write_text(out)
    passed = re.findall(r'ok (\w+) passed', out)
    failed = re.findall(r'\d+\) (\w+):? failed', out)
    return out, passed, failed


if __name__ == '__main__':
    a = sys.argv[1:]
    if len(a) < 4:
        sys.exit(__doc__)
    print(json.dumps(run(a[0], a[1], int(a[2]), int(a[3]), a[4] if len(a) > 4 else None)))
