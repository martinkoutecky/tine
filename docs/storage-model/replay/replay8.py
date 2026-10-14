#!/usr/bin/env python3
"""Translate the frozen s3 scenario syntax, without hand-maintained expected outcomes."""
import re
from pathlib import Path
import replay
ROOT=Path(__file__).resolve().parent
LEAN=replay.LEAN
QUINT=replay.QUINT

def lex(s):
    return re.findall(r'"[^"]*"|\d+|[A-Za-z_][\w\x27]*|==|!=|>=|<=|->|=>|[^\s]',s)
class Parser(replay.Parser):
    def __init__(self,s): self.ts=lex(s);self.i=0
    def expr(self,minprec=0):
        t=self.pop()
        if t=='(':
            x=self.expr();self.pop(')')
        elif t=='if':
            self.pop('(');c=self.expr();self.pop(')');a=self.expr(1);self.pop('else');b=self.expr(1);x=('if',c,a,b)
        elif t=='-': x=('neg',self.expr(9))
        else: x=('id',t)
        precs={'and':3,'or':2,'==':4,'!=':4,'>':4,'<':4,'>=':4,'<=':4,'+':5,'-':5,'->':1,'=>':0}
        while True:
            t=self.peek()
            if t=='(':
                self.pop();args=[]
                if self.peek()!=')':
                    args.append(self.expr())
                    while self.peek()==',': self.pop();args.append(self.expr())
                self.pop(')');x=('call',x,args)
            elif t=='.': self.pop();x=('dot',x,self.pop())
            elif t in precs:
                prec=precs[t]
                if prec<minprec:break
                self.pop();x=('bin',t,x,self.expr(prec+1))
            else:break
        return x

def parse(s):
    p=Parser(s);e=p.expr();assert p.i==len(p.ts),p.ts[p.i:];return e
name=replay.name

def setexpr(e):
    return e[0]=='call' and (name(e[1])=='Set' or e[1][0]=='dot' and e[1][2]=='get' and e[1][1][0]=='dot' and e[1][1][2] in {'trash','trashStable','opRead','removed','delDurable'})
def pred(e):
    k=e[0]
    if k=='id':
        if e[1] in definitions and not definitions[e[1]][0]: return pred(definitions[e[1]][1])
        return {'s':'x.s','g':'x.g','ok':'(Storage32.guarantee x && Storage32.orderHolds x && Storage32.renameResolves x)','guarantee':'Storage32.guarantee x','orderHolds':'Storage32.orderHolds x','renameResolves':'Storage32.renameResolves x',
          'canSwitch':'Storage21.canSwitch c (Storage3.project (Storage32.project x))','ABSENT':'absent','NONE':'noneBytes','UNKNOWN':'unknown',
          'PAGES':'paths','MUTANT':'c.mutant'}.get(e[1],e[1])
    if k=='neg':return f'(-{pred(e[1])})'
    if k=='dot':return f'{pred(e[1])}.{e[2]}'
    if k=='bin':
        op=e[1]
        if op=='->': return f'({pred(e[2])}, {pred(e[3])})'
        if op=='=>':return f'(fun {pred(e[2])} => {pred(e[3])})'
        if op in ['==','!='] and e[3][0]=='call' and name(e[3][1]) in ['List','Set'] and not e[3][2]:
            r=f'({pred(e[2])}.isEmpty)';return r if op=='==' else f'(!{r})'
        if op in ['==','!='] and (setexpr(e[2]) or setexpr(e[3])):
            r=f'(setEq {pred(e[2])} {pred(e[3])})';return r if op=='==' else f'(!{r})'
        return f'({pred(e[2])} '+{'and':'&&','or':'||'}.get(op,op)+f' {pred(e[3])})'
    if k=='if':return f'(if {pred(e[1])} then {pred(e[2])} else {pred(e[3])})'
    if k=='call':
        f,args=e[1],e[2]
        if f[0]=='dot' and f[2]=='get':return f'({pred(f[1])} (pg {pred(args[0])}))'
        if f[0]=='dot' and f[2]=='contains' and name(f[1])=='RACES':
            return {'"R1"':'c.r1','"weak"':'c.weak','"crash"':'c.crashes','"power"':'c.powers'}[args[0][1]]
        if f[0]=='dot' and f[2]=='forall':return f'({pred(f[1])}.all {pred(args[0])})'
        n=name(f)
        if n=='not':return f'(!{pred(args[0])})'
        if n in ['Set','Map','List']:return '['+', '.join(pred(a) for a in args)+']'
        if n=='opClean':return f'(Storage3.opClean {pred(args[0])}.toBase (pg {pred(args[1])}))'
        if n=='gated':return f'(Storage32.gated c {pred(args[0])} (pg {pred(args[1])}))'
        return '('+pred(f)+' '+' '.join(pred(a) for a in args)+')'
    raise ValueError(e)

def action_expr(e):
    if e[0]=='id':n=e[1];args=[]
    elif e[0]=='call' and e[1][0]=='id':n=e[1][1];args=e[2]
    else:raise ValueError(e)
    a=[pred(x) for x in args]
    pages={'wOpen','wEdit','wSend','wResolve','wDiscard','wClose','wRecv','observe','flush','draftSync','extWrite','extWriteD','opDelete','flushDel','load'}
    if n=='opRename':a=[f'(pg {a[0]})',f'(pg {a[1]})',f'({a[2]}.map pg)',f'(rewrite {a[3]})']
    elif n=='wOp':a=[f'(pg {a[0]})',f'(pg (1 - {a[0]}))',*a[1:]]
    elif n=='wOpTo':n='wOp';a=[f'(pg {a[0]})',f'(pg {a[1]})',*a[2:]]
    elif n in pages:a[0]=f'(pg {a[0]})'
    if n=='extWrite':n='extWriteD';a.append('true')
    if n=='power':a=[f'(fun p => if p.val == 0 then {a[0]} else if p.val == 1 then {a[1]} else false)']
    if n=='powerK':n='power';a=[f'(fun p => ({a[0]} : List Int).contains (p.val : Int))']
    r=f'(.{n}'+(' '+' '.join(a) if a else '')+')'
    return r if n in ['opRename','opDelete','flushDel','load'] else f'(.old {r})'

definitions={}
def program(e,stack=(),failing=False):
    if e[0]=='call' and e[1][0]=='dot' and e[1][2]=='fail':
        assert not failing;return program(e[1][1],stack,True)
    if e[0]=='call' and e[1][0]=='dot' and e[1][2] in ['then','expect']:
        assert not failing
        _,f,args=e;pre=program(f[1],stack)
        return pre+program(args[0],stack) if f[2]=='then' else pre+[f'  let x ← expect x (fun x => {pred(args[0])})']
    if e[0]=='if':
        assert not failing
        aa=program(e[2],stack);bb=program(e[3],stack)
        return [f'  let x ← (if {pred(e[1])} then do',*['  '+s for s in aa],'    pure x','  else do',*['  '+s for s in bb],'    pure x)']
    if e[0]=='id' and e[1]=='init':return ['  let x := Storage32.init c']
    if e[0]=='id' and e[1]=='noop':return []
    n=name(e) if e[0]=='id' else name(e[1]) if e[0]=='call' else None
    if n in definitions:
        params,body=definitions[n];args=e[2] if e[0]=='call' else [];mapping=dict(zip(params,args))
        def subst(t):
            if isinstance(t,list):return [subst(a) for a in t]
            if not isinstance(t,tuple):return t
            if t[0]=='id' and t[1] in mapping:return mapping[t[1]]
            return tuple(subst(a) for a in t)
        assert n not in stack
        return program(subst(body),stack+(n,),failing)
    return [f'  let x ← {"refuse" if failing else "act"} c x {action_expr(e)}']

def generate():
    replay.OUT.mkdir(parents=True,exist_ok=True)
    definitions.clear()
    s=re.sub(r'//[^\n]*','',(replay.SRC/'scenarios-s32.inc').read_text())
    matches=list(re.finditer(r'\b(action|val|run)\s+(\w+)\s*(\([^\n]*?\))?\s*(?::\s*bool)?\s*=',s));runs=[]
    for i,m in enumerate(matches):
        kind,n,params=m.groups();body=s[m.end():matches[i+1].start() if i+1<len(matches) else len(s)].strip()
        if n in ['noop','ok']:continue
        definitions[n]=(re.findall(r'(\w+)\s*:',params or ''),parse(body))
        if kind=='run':runs.append(n)
    assert len(runs)==159,len(runs)
    header='''import Storage32
open Storage21
abbrev opClean := @Storage3.opClean
def pg (i : Int) : Fin 3 := ⟨i.toNat % 3, Nat.mod_lt _ (by decide)⟩
def paths : List Int := [0,1,2]
def setEq [BEq α] (a b : List α) : Bool := a.all b.contains && b.all a.contains
def rewrite (m : List (Int × Int)) (p : Fin 3) : Int :=
  match m.find? (fun e => e.1 == (p.val : Int)) with | some e => e.2 | none => absent
def act (c : Config 3) (x : Storage32.State 3) (a : Storage32.Action 3) : Except String (Storage32.State 3) :=
  match Storage32.step c x a with | some y => .ok y | none => .error "disabled"
def refuse (c : Config 3) (x : Storage32.State 3) (a : Storage32.Action 3) : Except String (Storage32.State 3) :=
  if Storage32.enabled c x a then .error "returned-false" else .ok x
def expect (x : Storage32.State 3) (p : Storage32.State 3 → Bool) : Except String (Storage32.State 3) :=
  if p x then .ok x else .error "assertion"
'''
    out=[header]
    for n in runs:out.append(f'def {n} (c : Config 3) : Except String (Storage32.State 3) := do\n'+'\n'.join(program(('id',n)))+'\n  pure x\n')
    out.append('def scenarios : List (String × (Config 3 → Except String (Storage32.State 3))) := [\n'+',\n'.join(f'  ("{n}", {n})' for n in runs)+']\n')
    out.append('''def main (args : List String) : IO Unit := do
  let profile := args[0]!
  let mutant := args[1]!
  let c : Config 3 := {positive := by decide, texts := [1,2,3], nonempty := by decide, r1 := profile == "R1" || profile == "all", weak := profile == "weak" || profile == "all", mutant}
  for (n,f) in scenarios do
    match f c with
    | .ok x => IO.println s!"{n}:pass:{x.g.bad}"
    | .error e => IO.println s!"{n}:{e}"
''')
    (replay.OUT/'Replay8.lean').write_text('\n'.join(out));return runs
if __name__=='__main__': print('Generated',len(generate()),'s3 scenarios')
