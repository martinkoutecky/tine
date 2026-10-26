#!/usr/bin/env python3
"""Quint <-> Lean outcome replay: the evidence that the Lean transcription behaves like the Quint model.

    (cd ../lean && lake build)          # the replays import the Lean transcriptions
    python3 compare.py s2|s3|s31|s32|s33|s34|s35

For one model version, every scenario is run in Quint (../quint/run.py, same variants as sweep.py) and in Lean
(a program generated from the SAME scenario file by replay*.py, executed with `lake env lean --run`), under the
four race profiles and under every mutant of the model's SWEEP-MUTS line. Each scenario's outcome (pass,
assertion failure, disabled step, test returned false, refusal) must be identical, and every mutant's own
scenario must fail by assertion in both. This is finite evidence, not a proof of transcription fidelity.
Recorded results (internal verifier reruns): s2 2,024, s3 3,720, s3.1 4,433, s3.2 5,724, s3.3 7,080,
s3.4 7,831, s3.5 8,148 outcomes, zero differences. (s3.1's MUL heldness mutant was replayed separately; it is not covered here.)
"""
import ast, json, os, re, subprocess, sys, time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / 'quint'))
VERSIONS = {  # version: (generator module, model, scenarios, scenario count, generated Lean file)
    's2':  ('replay',   'storage-s2.qnt',  'scenarios-s2.inc',  88,  'Replay.lean'),
    's3':  ('replay3',  'storage-s3.qnt',  'scenarios-s3.inc',  120, 'Replay3.lean'),
    's31': ('replay7',  'storage-s31.qnt', 'scenarios-s31.inc', 143, 'Replay7.lean'),
    's32': ('replay8',  'storage-s32.qnt', 'scenarios-s32.inc', 159, 'Replay8.lean'),
    's33': ('replay33', 'storage-s33.qnt', 'scenarios-s33.inc', 177, 'Replay33.lean'),
    's34': ('replay34', 'storage-s34.qnt', 'scenarios-s34.inc', 191, 'Replay34.lean'),
    's35': ('replay35', 'storage-s35.qnt', 'scenarios-s35.inc', 194, 'Replay35.lean'),
}
if len(sys.argv) != 2 or sys.argv[1] not in VERSIONS:
    sys.exit(__doc__)
gen, qnt, scen, count, leanfile = VERSIONS[sys.argv[1]]
os.environ.update(QNT=qnt, SCEN=scen)
import run                                   # ../quint/run.py
replay = __import__(gen)
base = __import__('replay')


def classify_error(code, message):
    if code == 'QNT511': return 'returned-false'
    if code == 'QNT513': return 'disabled'
    if code == 'QNT508':
        if 'Expected action to fail' in message: return 'expected-disabled'
        if 'Expect condition does not hold true' in message: return 'assertion'
        if 'Cannot continue to' in message and 'expect' in message: return 'disabled'
    return 'runtime:' + code


def main():
    replay.generate()
    lean_src = base.OUT / leanfile
    logs = base.OUT / 'logs'
    logs.mkdir(parents=True, exist_ok=True)
    muts = ast.literal_eval(re.search(r'// SWEEP-MUTS: (\{.*\})', (run.ROOT / qnt).read_text()).group(1))
    jobs = [(p, 'none') for p in ['base', 'R1', 'weak', 'all']] + [(p, m) for m, p in muts.items()]
    results, total = [], 0
    for profile, mutant in jobs:
        start = time.monotonic()
        out, passed, _ = run.test(profile, mutant)
        qerr = {n: classify_error(c, m) for n, c, m in
                re.findall(r'\n\s+\d+\) (\w+):\n\s+Error \[(QNT\d+)\]: ([^\n]*)', out)}
        assert len(passed) + len(qerr) == count, (profile, mutant, len(passed), qerr)
        lr = subprocess.run(['lake', 'env', 'lean', '--run', str(lean_src), profile, mutant],
                            cwd=HERE.parent / 'lean', capture_output=True, text=True)
        leanout = lr.stdout + lr.stderr
        (logs / f'lean-{sys.argv[1]}-{profile}-{mutant}.log').write_text(leanout)
        assert lr.returncode == 0, leanout[-3000:]
        ls = {}
        for line in leanout.splitlines():
            if ':' in line:
                n, status, *_ = line.split(':')
                ls[n] = status
        assert len(ls) == count, len(ls)
        qs = {n: 'pass' for n in passed} | qerr
        mismatch = {n: {'quint': qs[n], 'lean': ls.get(n)} for n in qs if qs[n] != ls.get(n)}
        caught = mutant == 'none' or (qs.get('m' + mutant) == 'assertion' and ls.get('m' + mutant) == 'assertion')
        total += len(qs)
        results.append(dict(profile=profile, mutant=mutant, mismatches=mismatch, caught=caught,
                            passes=list(ls.values()).count('pass'), seconds=round(time.monotonic() - start, 1)))
        print(profile, mutant, 'passes', results[-1]['passes'], 'mismatches', mismatch, 'caught', caught, flush=True)
    bad = sum(len(r['mismatches']) for r in results)
    (base.OUT / f'comparison-{sys.argv[1]}.json').write_text(json.dumps(results, indent=2) + '\n')
    ok = bad == 0 and all(r['caught'] for r in results)
    print(f'REPLAY {"PASS" if ok else "FAIL"}: {sys.argv[1]}, {total} outcomes, {bad} mismatches, '
          f'{sum(r["caught"] for r in results if r["mutant"] != "none")}/{len(muts)} mutants caught in both')
    sys.exit(0 if ok else 1)


if __name__ == '__main__':
    main()
