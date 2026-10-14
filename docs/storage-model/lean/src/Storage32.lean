import Storage31
namespace Storage32
open Storage21
structure Gate (P : Nat) where
  dst : Fin P
  src : Fin P
  vers : Fin P → Int
  pending : List (Fin P)
abbrev Obl (P : Nat) := Fin P × Fin P × Int × Fin P
structure Sys (P : Nat) extends toBase : Storage3.Sys P where
  gates : List (Gate P) := []
structure Ghost (P : Nat) extends toBase : Storage3.Ghost P where
  obl : List (Obl P) := []
  renamed : List (Fin P × Fin P) := []
  orderOk : Bool := true
structure State (P : Nat) where
  s : Sys P
  g : Ghost P
def project (x : State P) : Storage31.State P := ⟨x.s.toBase,x.g.toBase⟩
def embed (x : Storage31.State P) : State P := ⟨⟨x.s,[]⟩,⟨x.g,[],[],true⟩⟩
def lift (x : State P) (y : Storage31.State P) : State P :=
  ⟨{x.s with toBase :=  y.s},{x.g with toBase :=  y.g}⟩
def commit (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String) : State P :=
  let y := Storage3.commit c (project x) s.toBase g.toBase n
  ⟨{s with toBase :=  y.s},{g with toBase :=  y.g}⟩
def initWith (c : Config P) (disk : Fin P → Int) : State P := embed (Storage31.initWith c disk)
def init (c : Config P) : State P := initWith c (Storage31.initDisk c)
def gates1 (c : Config P) (gt : Gate P) (k : Fin P) : Bool :=
  gt.pending.contains k && k != gt.dst && !(c.mutant == "MRF" && k != gt.src) &&
    (gt.pending.contains gt.dst || (k == gt.src && gt.pending.length > 1 && c.mutant != "MRD"))
def gated (c : Config P) (sys : Sys P) (k : Fin P) : Bool := sys.gates.any (fun gt => gates1 c gt k)
def touches (sys : Sys P) (ks : List (Fin P)) : Bool :=
  sys.gates.any (fun gt => ks.any (fun k => gt.vers k != noneBytes))
def partner (sys : Sys P) (k : Fin P) : Option (Fin P) :=
  sys.gates.foldl (fun acc gt => if acc.isNone && gt.dst == k && gt.pending.contains k then some gt.src else acc) none
def discharge (sys : Sys P) (q : Fin P) (ver : Int) : Sys P :=
  {sys with gates := sys.gates.filterMap fun gt =>
    let ng := if gt.vers q != noneBytes && ver >= gt.vers q then {gt with pending := exclude gt.pending [q]} else gt
    if ng.pending.isEmpty then none else some ng}
def discarded (sys : Sys P) (k : Fin P) : Sys P :=
  {sys with gates := sys.gates.filterMap fun gt =>
    let ng := if gt.dst == k then gt else {gt with pending := exclude gt.pending [k]}
    if (gt.dst == k && gt.pending.contains k) || ng.pending.isEmpty then none else some ng}
def oblPublished (obl : List (Obl P)) (q : Fin P) (ver : Int) : List (Obl P) :=
  obl.filter fun o => !(o.2.1 == q && ver >= o.2.2.1)
def oblDiscarded (obl : List (Obl P)) (k : Fin P) : List (Obl P) :=
  let cancels := obl.any fun o => o.2.1 == k && o.2.2.2 == k
  obl.filter fun o => !(o.1 == k || o.2.1 == k || (cancels && o.2.2.2 == k))
def unrename (r : List (Fin P × Fin P)) (ks : List (Fin P)) : List (Fin P × Fin P) :=
  r.filter fun x => !(ks.contains x.1 || ks.contains x.2)
def makeGate (x : State P) (dst src : Fin P) (ps : List (Fin P)) : Gate P :=
  {dst,src,vers := fun p => if ps.contains p then x.g.vc + 1 + p.val else noneBytes, pending := ps}
def makeObl (x : State P) (dst src : Fin P) (others : List (Fin P)) : List (Obl P) :=
  let v := fun p : Fin P => x.g.vc + 1 + p.val
  union (union (others.map fun r => (r,dst,v dst,dst))
    (others.map fun r => (src,r,v r,dst))) [(src,dst,v dst,dst)]
def opRename (c : Config P) (x : State P) (src dst : Fin P) (refs : List (Fin P)) (rt : Fin P → Int) : State P :=
  let full := Storage3.opCur x.s.toBase src != absent
  let ps := union (if full then [src,dst] else []) refs
  let y := lift x (Storage3.opRename c (project x) src dst refs rt)
  {y with s := {y.s with gates := if full then x.s.gates ++ [makeGate x dst src ps] else x.s.gates},
          g := {y.g with obl := if full then union x.g.obl (makeObl x dst src refs) else x.g.obl, renamed := union (unrename x.g.renamed [src,dst]) (if full && x.s.disk src != absent then [(src,dst)] else [])}}
def opDelete (c : Config P) (x : State P) (p : Fin P) : State P :=
  let y := lift x (Storage3.opDelete c (project x) p)
  {y with g := {y.g with renamed := unrename x.g.renamed [p]}}
def flush (c : Config P) (x : State P) (p : Fin P) : State P :=
  let y := lift x (Storage3.flushDel c (project x) p)
  {y with g := {y.g with orderOk := x.g.orderOk && !(x.g.obl.any fun o => o.1 == p)}}
def rename (c : Config P) (x : State P) : State P :=
  let y := lift x (Storage3.rename c (project x))
  {y with s := if c.mutant == "MPR" then discharge y.s x.s.job.p x.s.job.ver else y.s}
def dirSync (c : Config P) (x : State P) (ok : Bool) : State P :=
  let y := lift x (Storage3.dirSync c (project x) ok)
  {y with s := if (ok && c.mutant != "MPR") || c.mutant == "MCV" then discharge y.s x.s.job.p x.s.job.ver else y.s,
          g := {y.g with obl := if ok then oblPublished x.g.obl x.s.job.p x.s.job.ver else x.g.obl}}
def check (c : Config P) (x : State P) : State P :=
  let y := lift x (Storage3.executeOld c (project x) .check)
  {y with s := if c.mutant == "MCV" && x.s.disk x.s.job.p != x.s.job.base && c.mutant != "MC" && c.mutant != "MC1" then discharge y.s x.s.job.p x.s.job.ver else y.s}
def saveFail (c : Config P) (x : State P) : State P :=
  let y := lift x (Storage3.executeOld c (project x) .saveFail)
  {y with s := if c.mutant == "MCV" then discharge y.s x.s.job.p x.s.job.ver else y.s}
def stopped (y : State P) : State P :=
  {y with s := {y.s with gates := []},g := {y.g with obl := [],renamed := []}}
def upDiscard (c : Config P) (x : State P) (sys : Sys P) (m : Up P) (rok : Bool) : State P :=
  let pg := sys.pages m.p
  let d := x.s.disk m.p
  let v1 := x.g.vc + 1
  let pr := x.g.promise m.p
  let hold := pg.risk && (d == pg.buf || d == (x.s.drafts m.p).bytes) && c.mutant != "MDE"
  let nw := {pg with buf := d,base := d,ver := v1,obs := d,conflict := false,risk := hold,typed := false}
  let live := pr.on && !(pr.saved && x.g.ext m.p > pr.ep)
  let npr := if live && pr.bytes == d then pr else endPromise pr v1
  let src := (partner sys m.p).getD m.p
  let revert := src != m.p && (sys.pages src).buf == absent
  let sp := sys.pages src
  let sd := x.s.disk src
  let v2 := x.g.vc + 2
  let sprm := x.g.promise src
  let shold := sp.risk && (sd == sp.buf || sd == (x.s.drafts src).bytes) && c.mutant != "MDE"
  let slive := sprm.on && !(sprm.saved && x.g.ext src > sprm.ep)
  let pages1 := put sys.pages m.p nw
  let pages2 := if revert then put pages1 src {sp with buf := sd,base := sd,ver := v2,obs := sd,conflict := false,risk := shold,typed := false} else pages1
  let prom1 := put x.g.promise m.p npr
  let prom2 := if revert then put prom1 src (if slive && sprm.bytes == sd then sprm else endPromise sprm v2) else prom1
  if !pg.held then commit c x sys {x.g with bad := union x.g.bad ["request-to-unheld"]} "uDiscard"
  else if !rok then commit c x {sys with toBase := {sys.toBase with toSys := ackMail sys.toBase.toSys m.p m.cur pg.ver false}} x.g "uDiscardFail"
  else
    let ns := discarded {sys with pages := pages2} m.p
    commit c x {ns with toBase :=  {ns.toBase with toSys :=  ackMail ns.toBase.toSys m.p m.cur v1 false}}
      {x.g with vc := if revert then v2 else v1,promise := prom2,mine := if revert then [m.p,src] else [m.p],obl := oblDiscarded x.g.obl m.p} "uDiscard"
def deliverUp (c : Config P) (x : State P) (rok : Bool) : State P :=
  match x.s.up with
  | [] => x
  | m::tail => if m.kind == "discard" then upDiscard c x {x.s with up := tail} m rok
    else lift x (Storage3.executeOld c (project x) (.deliverUp rok))
abbrev Action := Storage31.Action
def enabled (c : Config P) (x : State P) (a : Action P) : Bool :=
  Storage31.enabled c (project x) a &&
    match a with
    | .old (.flush p) => !gated c x.s p
    | .flushDel p => c.mutant == "MDF" || !gated c x.s p
    | .opRename src dst refs _ => !touches x.s (union refs [src,dst])
    | .opDelete p => !touches x.s [p]
    | .old (.deliverUp _) => match x.s.up with
      | m::_ => m.kind != "discard" || !(partner x.s m.p).any (fun p => locked x.s.toBase.toSys p)
      | [] => true
    | _ => true
def execute (c : Config P) (x : State P) : Action P → State P
  | .old (.flush p) => flush c x p
  | .flushDel p => flush c x p
  | .old .rename => rename c x
  | .old (.dirSync ok) => dirSync c x ok
  | .old .check => check c x
  | .old .saveFail => saveFail c x
  | .old (.deliverUp ok) => deliverUp c x ok
  | .old .crash => stopped (lift x (Storage3.executeOld c (project x) .crash))
  | .old .switchFin => stopped (lift x (Storage3.executeOld c (project x) .switchFin))
  | .old (.power keep) => stopped (lift x (Storage3.power c (project x) keep))
  | .old (.extWriteD p v dur) =>
    let y := lift x (Storage3.executeOld c (project x) (.extWriteD p v dur))
    {y with g := {y.g with renamed := unrename x.g.renamed [p]}}
  | .opRename src dst refs rt => opRename c x src dst refs rt
  | .opDelete p => opDelete c x p
  | a => lift x (Storage31.execute c (project x) a)
def step (c : Config P) (x : State P) (a : Action P) : Option (State P) :=
  if enabled c x a then some (execute c x a) else none
abbrev guarantee (x : State P) := Storage31.guarantee (project x)
def orderHolds (x : State P) : Bool := x.g.orderOk
def renameResolves (x : State P) : Bool :=
  x.g.renamed.all fun r => x.s.disk r.1 != absent || x.s.disk r.2 != absent
inductive Reachable (c : Config P) (disk : Fin P → Int) : State P → Prop where
  | initial : Reachable c disk (initWith c disk)
  | advance {x y} : Reachable c disk x → (a : Action P) → step c x a = some y → Reachable c disk y
end Storage32
