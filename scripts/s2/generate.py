#!/usr/bin/env python3
"""Freeze s2 scenarios/oracles and full-state random Quint traces into Rust fixtures.

Parser adapted from the read-only lean1/replay.py (same frozen syntax).
All generated models, logs and ITF files stay in this worktree's scratch/.
"""
import argparse, ast, hashlib, json, os, re, subprocess
from pathlib import Path

def lex(s):
    return re.findall(r'"[^"]*"|\d+|[A-Za-z_][\w\x27]*|==|!=|>=|<=|->|[^\s]', s)

class Parser:
    def __init__(self, s):
        self.ts = lex(s); self.i = 0
    def pop(self, t=None):
        x = self.ts[self.i]; self.i += 1
        if t is not None: assert x == t, (x,t,self.ts[self.i-8:self.i+8])
        return x
    def peek(self): return self.ts[self.i] if self.i < len(self.ts) else None
    def expr(self, minprec=0):
        t = self.pop()
        if t == '(':
            x = self.expr(); self.pop(')')
        elif t == 'if':
            self.pop('('); c = self.expr(); self.pop(')')
            a = self.expr(1); self.pop('else'); b = self.expr(1)
            x = ('if', c, a, b)
        elif t == '-': x = ('neg', self.expr(9))
        else: x = ('id',t)
        while True:
            t = self.peek()
            if t == '(':
                self.pop(); args=[]
                if self.peek() != ')':
                    args.append(self.expr())
                    while self.peek() == ',': self.pop(); args.append(self.expr())
                self.pop(')'); x = ('call',x,args)
            elif t == '.':
                self.pop(); x=('dot',x,self.pop())
            elif t in {'or':2,'and':3,'==':4,'!=':4,'>':4,'<':4,'>=':4,'<=':4,'+':5,'-':5}:
                prec={'or':2,'and':3,'==':4,'!=':4,'>':4,'<':4,'>=':4,'<=':4,'+':5,'-':5}[t]
                if prec < minprec: break
                self.pop(); x=('bin',t,x,self.expr(prec+1))
            else: break
        return x

def parse(s):
    p=Parser(s); e=p.expr(); assert p.i == len(p.ts), p.ts[p.i:]; return e

def name(e):
    return e[1] if e[0]=='id' else None


SHA = "1c3194293856a11630536b74a8221c76c2a8725e77bc366b9172641681a08cef"
PROFILES = {
    "base": 'Set("crash", "power")',
    "R1": 'Set("crash", "power", "R1")',
    "weak": 'Set("crash", "power", "weak")',
    "all": 'Set("crash", "power", "R1", "weak")',
}

def scenarios(source):
    source = re.sub(r"//[^\n]*", "", source)
    matches = list(re.finditer(r"\b(action|val|run)\s+(\w+)\s*(\([^\n]*?\))?\s*(?::\s*bool)?\s*=", source))
    definitions, runs = {}, []
    for i, m in enumerate(matches):
        kind, n, params = m.groups()
        if n in ["noop", "ok"]: continue
        body = source[m.end():matches[i+1].start() if i+1 < len(matches) else len(source)].strip()
        definitions[n] = (re.findall(r"(\w+)\s*:", params or ""), parse(body))
        if kind == "run": runs.append(n)
    assert len(runs) == 88
    def expand(e, stack=()):
        if e[0] == "call" and e[1][0] == "dot" and e[1][2] in ["then", "expect"]:
            pre = expand(e[1][1], stack)
            return pre + (expand(e[2][0], stack) if e[1][2] == "then" else [["expect", e[2][0]]])
        if e[0] == "if":
            return [["if", e[1], expand(e[2], stack), expand(e[3], stack)]]
        if e[0] == "id" and e[1] == "init": return [["init"]]
        if e[0] == "id" and e[1] == "noop": return []
        n = name(e) if e[0] == "id" else name(e[1]) if e[0] == "call" else None
        if n in definitions:
            params, body = definitions[n]
            mapping = dict(zip(params, e[2] if e[0] == "call" else []))
            def subst(t):
                if isinstance(t, list): return [subst(a) for a in t]
                if not isinstance(t, tuple): return t
                if t[0] == "id" and t[1] in mapping: return mapping[t[1]]
                return tuple(subst(a) for a in t)
            assert n not in stack
            return expand(subst(body), stack+(n,))
        assert n is not None, e
        return [["action", n, e[2] if e[0] == "call" else []]]
    return {n: expand(("id", n)) for n in runs}

def command(args, log, env):
    r = subprocess.run(["rtk", "proxy", *map(str, args)], text=True, capture_output=True, env=env)
    log.write_text(r.stdout+r.stderr)
    return r

def decode(v):
    if isinstance(v, list): return [decode(x) for x in v]
    if not isinstance(v, dict): return v
    if "#bigint" in v: return int(v["#bigint"])
    if "#map" in v:
        pairs = v["#map"]
        assert sorted(int(decode(k)) for k, _ in pairs) == [0, 1]
        return [decode(x) for _, x in sorted(pairs, key=lambda kv: int(decode(kv[0])))]
    if "#set" in v: return sorted((decode(x) for x in v["#set"]), key=lambda x: x)
    if "#tup" in v: return [decode(x) for x in v["#tup"]]
    return {k: decode(x) for k, x in v.items() if k != "#meta"}

def traced_model(model):
    # Instrument only the next-state driver; no original rule is changed.
    begin = model.index("  action step = any {")
    end = model.index("  // ================================================================ THE GUARANTEE", begin)
    groups = []
    # Bias toward protocol progress while retaining every original choice.
    # Unbiased simulation mostly crashes before windows can send or saves run.
    weights = {"wOpen": 8, "wSend": 20, "wRecv": 20, "wEdit": 8,
               "wResolve": 8, "wOp": 12, "deliverUp": 20, "flush": 8,
               "check": 20, "rename": 20, "dirSync": 20, "launch": 20,
               "observe": 4, "draftSync": 4, "wDiscard": 2, "wClose": 2,
               "switchReq": 2}
    def emit(n, args=(), bind=""):
        encoded = [f"if ({a}) 1 else 0" if a in ["ok", "dur", "k0", "k1"] else a for a in args]
        call = n + ("(" + ", ".join(args) + ")" if args else "")
        group = bind + ' all { ' + call + ', traceAction\' = { name: "' + n + '", args: List(' + ", ".join(encoded) + ') } }'
        groups.extend([group] * weights.get(n, 1))
    for n in ["wOpen", "wSend", "wDiscard", "wClose", "wRecv", "flush", "observe", "draftSync"]:
        emit(n, ["p"], "nondet p = PAGES.oneOf()")
    for n in ["wEdit", "wResolve"]:
        emit(n, ["p", "v"], "nondet p = PAGES.oneOf() nondet v = TEXTS.oneOf()")
    emit("wOp", ["p", "dS", "dD"], "nondet p = PAGES.oneOf() nondet dS = TEXTS.oneOf() nondet dD = TEXTS.oneOf()")
    for n in ["check", "rename", "saveFail", "switchReq", "switchFin", "crash", "windowCrash", "launch"]:
        emit(n)
    for n in ["dirSync", "deliverUp"]: emit(n, ["ok"], "nondet ok = BOOLS.oneOf()")
    emit("extWriteD", ["p", "v", "dur"], "nondet p = PAGES.oneOf() nondet v = TEXTS.union(Set(ABSENT)).oneOf() nondet dur = BOOLS.oneOf()")
    emit("power", ["k0", "k1"], "nondet k0 = BOOLS.oneOf() nondet k1 = BOOLS.oneOf()")
    driver = """  var traceAction: { name: str, args: List[int] }
  action traceInit = all { init, traceAction' = { name: "init", args: List() } }
  action traceStep = any {
""" + ",\n".join(groups) + "\n  }\n"
    return model[:begin] + driver + model[end:]

def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("model_dir", type=Path)
    ap.add_argument("--quint", type=Path)
    ap.add_argument("--traces-per-profile", type=int, default=64)
    ap.add_argument("--steps", type=int, default=40)
    ap.add_argument("--sample", type=int, default=8, help="committed traces per profile; all generated traces are also replayed")
    ap.add_argument("--reuse", action="store_true", help="rebuild fixtures from existing logs/ITF, without claiming fresh Quint execution")
    ap.add_argument("--reuse-scenarios", action="store_true", help="reuse scenario oracle logs but generate fresh random traces")
    args = ap.parse_args()
    root = Path(__file__).resolve().parents[2]
    work = root/"scratch"/"s2"; work.mkdir(parents=True, exist_ok=True)
    fixture = root/"crates/tine-store/tests/fixtures/s2"; fixture.mkdir(parents=True, exist_ok=True)
    model = (args.model_dir/"storage-s2.qnt").read_text()
    assert hashlib.sha256(model.encode()).hexdigest() == SHA
    source = (args.model_dir/"scenarios-s2.inc").read_text()
    muts = ast.literal_eval(re.search(r"// SWEEP-MUTS: (\{.*\})", model).group(1))
    quint = args.quint or args.model_dir.parent/"model/tools/node_modules/.bin/quint"
    env = {**os.environ, "TMPDIR": str(work)}
    jobs = [(p, "none") for p in PROFILES] + [(p, m) for m, p in muts.items()]
    outcomes = []
    for p, m in jobs:
        text = re.sub(r"pure val RACES: Set\[str\] = .*", f"pure val RACES: Set[str] = {PROFILES[p]}", model, count=1)
        text = re.sub(r'pure val MUTANT: str = ".*"', f'pure val MUTANT: str = "{m}"', text, count=1)
        path = work/f"{p}-{m}.qnt"; path.write_text(text.replace("  // @@SCENARIOS@@", source))
        log = work/f"scenarios-{p}-{m}.log"
        if not (args.reuse or args.reuse_scenarios):
            result = command([quint, "test", path, "--backend", "typescript", "--max-samples", "1", "--verbosity", "2", "--match", "^(max|own|q|m)[A-Z]"], log, env)
            assert result.returncode in [0, 1], log.read_text()
        out = log.read_text()
        statuses = {n: "pass" for n in re.findall(r"ok (\w+) passed", out)}
        errors = dict(re.findall(r"\n\s+\d+\) (\w+):\n\s+Error \[(QNT\d+)\]", out))
        statuses.update({n: {"QNT508": "assertion", "QNT507": "disabled", "QNT513": "disabled"}.get(e, e) for n, e in errors.items()})
        assert len(statuses) == 88, (p, m, len(statuses), out)
        assert m != "none" or set(statuses.values()) == {"pass"}
        assert m == "none" or statuses["m"+m] == "assertion"
        outcomes.append({"profile": p, "mutant": m, "outcomes": statuses})
        print(f"scenario oracle {p}/{m}: {len(statuses)}", flush=True)
    data = {"model_sha256": SHA, "scenario_sha256": hashlib.sha256(source.encode()).hexdigest(), "scenarios": scenarios(source), "oracles": outcomes}
    assert all(set(o["outcomes"]) == set(data["scenarios"]) for o in outcomes)
    (fixture/"scenarios.json").write_text(json.dumps(data, separators=(",", ":"))+"\n")
    traces = []
    seeds = {p: str(20261008 + i * 7919) for i, p in enumerate(PROFILES)}
    for p in PROFILES:
        text = re.sub(r"pure val RACES: Set\[str\] = .*", f"pure val RACES: Set[str] = {PROFILES[p]}", model, count=1)
        path = work/f"trace-{p}.qnt"; path.write_text(traced_model(text))
        pattern = work/f"{p}-{{seq}}.itf.json"
        if not args.reuse:
            result = command([quint, "run", path, "--backend", "typescript", "--init", "traceInit", "--step", "traceStep", "--invariant", "guarantee", "--max-samples", args.traces_per_profile, "--n-traces", args.traces_per_profile, "--max-steps", args.steps, "--seed", seeds[p], "--verbosity", "1", "--out-itf", pattern], work/f"traces-{p}.log", env)
            assert result.returncode == 0, result.stdout+result.stderr
        paths = sorted(work.glob(f"{p}-*.itf.json"))
        assert len(paths) == args.traces_per_profile, (p, len(paths))
        for path in paths:
            states = json.loads(path.read_text())["states"]
            traces.append({"profile": p, "states": [{"action": decode(s["traceAction"]), "state": {"s": decode(s["s"]), "g": decode(s["g"])}} for s in states]})
        print(f"traces {p}: {len(paths)}", flush=True)
    seen = {s["action"]["name"] for t in traces for s in t["states"]}
    required = {"wOpen", "wSend", "wDiscard", "wClose", "wRecv", "flush", "observe", "draftSync",
                "wEdit", "wResolve", "wOp", "check", "rename", "saveFail", "switchReq", "switchFin",
                "crash", "windowCrash", "launch", "dirSync", "deliverUp", "extWriteD", "power"}
    assert required <= seen, ("random corpus omitted actions", required-seen)
    all_data = {"model_sha256": SHA, "seeds": seeds, "max_steps": args.steps,
                "driver": "weighted-original-choices", "traces": traces}
    (work/"traces.json").write_text(json.dumps(all_data, separators=(",", ":"))+"\n")
    sample = []
    for p in PROFILES:
        candidates = [t for t in traces if t["profile"] == p]
        covered = set()
        for _ in range(args.sample):
            # Deterministic greedy sample keeps rare actions (e.g. resolve)
            # instead of simply selecting the first eight random draws.
            best = max(range(len(candidates)), key=lambda i:
                       len({s["action"]["name"] for s in candidates[i]["states"]} - covered))
            chosen = candidates.pop(best)
            covered.update(s["action"]["name"] for s in chosen["states"])
            sample.append(chosen)
    sample_data = {**all_data, "traces": sample}
    (fixture/"traces.json").write_text(json.dumps(sample_data, separators=(",", ":"))+"\n")
    print(f"generated {len(traces)} traces, {sum(len(t['states']) for t in traces)} states; committed {len(sample)} traces, {(fixture/'traces.json').stat().st_size} bytes")

if __name__ == "__main__": main()
