#!/usr/bin/env python3
"""Generate Lean replay directly from frozen Quint scenario syntax; compare results."""
import ast, hashlib, json, os, re, subprocess, sys, time
from pathlib import Path
ROOT = Path(__file__).resolve().parent
SRC = ROOT.parent / "quint"                     # published scenario files (read only)
OUT = Path(os.environ.get("OUT", ROOT / ".out")).resolve()   # generated Replay*.lean
LEAN = "lean"                                   # run under `lake env` from ../lean
QUINT = None                                    # unused here; compare.py uses ../quint/run.py

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

def pred(e):
    k=e[0]
    if k=='id':
        return {'s':'x.s','g':'x.g','ok':'guarantee x','guarantee':'guarantee x',
                'canSwitch':'canSwitch c x','ABSENT':'absent','NONE':'noneBytes','UNKNOWN':'unknown'}.get(e[1], e[1])
    if k=='neg': return f'(-{pred(e[1])})'
    if k=='dot': return f'{pred(e[1])}.{e[2]}'
    if k=='bin':
        op={'and':'&&','or':'||'}.get(e[1],e[1])
        return f'({pred(e[2])} {op} {pred(e[3])})'
    if k=='if': return f'(if {pred(e[1])} then {pred(e[2])} else {pred(e[3])})'
    if k=='call':
        f,args=e[1],e[2]
        if f[0]=='dot' and f[2]=='get':
            return f'({pred(f[1])} (pg {pred(args[0])}))'
        if f[0]=='dot' and f[2]=='contains' and name(f[1])=='RACES':
            return {'"R1"':'c.r1','"weak"':'c.weak','"crash"':'c.crashes','"power"':'c.powers'}[args[0][1]]
        n=name(f)
        if n=='not': return f'(!{pred(args[0])})'
        return '('+pred(f)+' '+' '.join(pred(a) for a in args)+')'
    raise ValueError(e)

def action_expr(e):
    if e[0]=='id': n=e[1]; args=[]
    elif e[0]=='call' and e[1][0]=='id': n=e[1][1];args=e[2]
    else: raise ValueError(e)
    a=[pred(x) for x in args]
    page_actions={'wOpen','wEdit','wSend','wResolve','wDiscard','wClose','wRecv','observe','flush','draftSync','extWrite','extWriteD'}
    if n=='wOp': a=[f'(pg {a[0]})',f'(pg (1 - {a[0]}))',*a[1:]]
    elif n in page_actions: a[0]=f'(pg {a[0]})'
    if n=='extWrite': n='extWriteD';a.append('true')
    if n=='power': a=[f'(fun p => if p.val == 0 then {a[0]} else {a[1]})']
    return f'(.{n}'+(' '+' '.join(a) if a else '')+')'

definitions={}
def program(e, stack=()):
    if e[0]=='call' and e[1][0]=='dot' and e[1][2] in ['then','expect']:
        _,f,args=e
        pre=program(f[1],stack)
        if f[2]=='then': return pre+program(args[0],stack)
        return pre+[f'  let x ← expect x (fun x => {pred(args[0])})']
    if e[0]=='if':
        aa=program(e[2],stack);bb=program(e[3],stack)
        return [f'  let x ← (if {pred(e[1])} then do', *['  '+s for s in aa], '    pure x',
                '  else do', *['  '+s for s in bb], '    pure x)']
    if e[0]=='id' and e[1]=='init': return ['  let x := init c']
    if e[0]=='id' and e[1]=='noop': return []
    n=name(e) if e[0]=='id' else name(e[1]) if e[0]=='call' else None
    if n in definitions:
        params,body=definitions[n]
        args=e[2] if e[0]=='call' else []
        mapping=dict(zip(params,args))
        def subst(t):
            if isinstance(t,list):return [subst(a) for a in t]
            if not isinstance(t,tuple):return t
            if t[0]=='id' and t[1] in mapping:return mapping[t[1]]
            return tuple(subst(a) for a in t)
        assert n not in stack
        return program(subst(body),stack+(n,))
    return [f'  let x ← act c x {action_expr(e)}']

def generate():
    s=re.sub(r'//[^\n]*','', (SRC/'scenarios-s2.inc').read_text())
    matches=list(re.finditer(r'\b(action|val|run)\s+(\w+)\s*(\([^\n]*?\))?\s*(?::\s*bool)?\s*=',s))
    runs=[]
    for i,m in enumerate(matches):
        kind,n,params=m.groups()
        body=s[m.end():matches[i+1].start() if i+1<len(matches) else len(s)].strip()
        if n in ['noop','ok']: continue
        pp=re.findall(r'(\w+)\s*:',params or '')
        definitions[n]=(pp,parse(body))
        if kind=='run':runs.append(n)
    assert len(runs)==88,len(runs)
    header='''import Storage
open Storage
def pg (i : Int) : Fin 2 := ⟨i.toNat % 2, Nat.mod_lt _ (by decide)⟩
def act (c : Config 2) (x : State 2) (a : Action 2) : Except String (State 2) :=
  match step c x a with
  | some y => .ok y
  | none => .error "disabled"
def expect (x : State 2) (p : State 2 → Bool) : Except String (State 2) :=
  if p x then .ok x else .error "assertion"
'''
    out=[header]
    for n in runs:
        out.append(f'def {n} (c : Config 2) : Except String (State 2) := do\n'+'\n'.join(program(('id',n)))+'\n  pure x\n')
    out.append('def scenarios : List (String × (Config 2 → Except String (State 2))) := [\n'+',\n'.join(f'  ("{n}", {n})' for n in runs)+']\n')
    out.append('''def main (args : List String) : IO Unit := do
  let profile := args[0]!
  let mutant := args[1]!
  let c : Config 2 := {positive := by decide, texts := [1,2,3], nonempty := by decide, r1 := profile == "R1" || profile == "all", weak := profile == "weak" || profile == "all", mutant}
  for (n,f) in scenarios do
    match f c with
    | .ok x => IO.println s!"{n}:pass:{x.g.bad}"
    | .error e => IO.println s!"{n}:{e}"
''')
    OUT.mkdir(parents=True,exist_ok=True); (OUT/'Replay.lean').write_text('\n'.join(out))
    return runs

if __name__=='__main__':
    runs=generate()
    print(f"Generated {len(runs)} original scenarios in Replay.lean")
