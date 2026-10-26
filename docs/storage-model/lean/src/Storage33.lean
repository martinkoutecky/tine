import Storage32
namespace Storage33
open Storage21
abbrev State := Storage32.State
abbrev init := @Storage32.init
abbrev initWith := @Storage32.initWith
abbrev guarantee := @Storage32.guarantee
abbrev orderHolds := @Storage32.orderHolds
abbrev renameResolves := @Storage32.renameResolves
abbrev Map (P : Nat) := Fin P → Option Int
def keys (eff : Map P) : List (Fin P) := (ids P).filter fun p => (eff p).isSome
def get (eff : Map P) (p : Fin P) : Int := (eff p).getD absent
def stale (x : State P) (eff plan : Map P) : Bool :=
  (keys eff).any fun p => (plan p).isNone || Storage3.opCur x.s.toBase p != get plan p
def enabledApply (c : Config P) (x : State P) (eff plan : Map P) (dst src : Option (Fin P)) : Bool :=
  let ps := keys eff
  x.s.alive && !ps.isEmpty && Storage3.opFree x.s.toBase ps && Storage31.opHeld c (Storage32.project x) ps &&
    !Storage32.touches x.s ps && ps.all (fun p => Storage3.opClean x.s.toBase p || c.mutant == "MAC") &&
    (!stale x eff plan || c.mutant == "MEX") &&
    ps.all (fun p => get eff p == absent || c.texts.contains (get eff p)) &&
    match dst,src with
    | none,none => true
    | some d,some s => d != s && ps.contains d && ps.contains s && get eff s == absent && get eff d != absent && get plan s != absent
    | _,_ => false
def opApply (c : Config P) (x : State P) (eff plan : Map P) (dst src : Option (Fin P)) : State P :=
  let ps := keys eff
  let v := fun p : Fin P => x.g.vc + 1 + p.val
  let np := fun p => {Storage3.opEdit x.s.toBase p (if ps.contains p then get eff p else absent) (v p) with risk := true}
  let pages := ps.foldl (fun acc p => put acc p (np p)) x.s.pages
  let drafted := if c.mutant == "MAN" then x.s.drafts else ps.foldl (fun acc p => put acc p (draftEntry (np p))) x.s.drafts
  let d := dst.getD (zero c)
  let s := src.getD (zero c)
  let mv := dst.isSome
  let others := exclude ps [d,s]
  Storage32.commit c x {x.s with pages,drafts := drafted,gates := if mv && c.mutant != "MAM" then x.s.gates ++ [Storage32.makeGate x d s ps] else x.s.gates}
    {x.g with vc := x.g.vc + P,mine := ps.filter (Storage3.opClean x.s.toBase),opRead := fun p => if ps.contains p then union (x.g.opRead p) [get plan p] else x.g.opRead p, obl := if mv then union x.g.obl (Storage32.makeObl x d s others) else x.g.obl, renamed := union (Storage32.unrename x.g.renamed ps) (if mv && x.s.disk s != absent then [(s,d)] else []), wrote := ps.foldl (fun acc p => put acc p (union (acc p) [(get eff p,v p)])) x.g.wrote, promise := ps.foldl (fun acc p => put acc p (ack (acc p) (get eff p) (v p) false 0)) x.g.promise, bad := union (union x.g.bad (tags (!ps.all (Storage3.opClean x.s.toBase)) "D4-unclean-page-changed")) (tags (stale x eff plan) "op-stale-base")} "opApply"
inductive Action (P : Nat) where
  | prior (a : Storage32.Action P)
  | opApply (eff plan : Map P) (dst src : Option (Fin P))
def enabled (c : Config P) (x : State P) : Action P → Bool
  | .prior a => Storage32.enabled c x a
  | .opApply eff plan dst src => enabledApply c x eff plan dst src
def execute (c : Config P) (x : State P) : Action P → State P
  | .prior a => Storage32.execute c x a
  | .opApply eff plan dst src => opApply c x eff plan dst src
def step (c : Config P) (x : State P) (a : Action P) : Option (State P) :=
  if enabled c x a then some (execute c x a) else none
inductive Reachable (c : Config P) (disk : Fin P → Int) : State P → Prop where
  | initial : Reachable c disk (initWith c disk)
  | advance {x y} : Reachable c disk x → (a : Action P) → step c x a = some y → Reachable c disk y
end Storage33
