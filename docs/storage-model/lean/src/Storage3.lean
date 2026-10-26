import Storage21
namespace Storage3
open Storage21

-- Common s2.1 transitions are used on the projection, not reimplemented.
structure Sys (P : Nat) extends Storage21.Sys P where
  trash : Fin P → List Int
  trashStable : Fin P → List Int
structure Ghost (P : Nat) extends Storage21.Ghost P where
  opRead : Fin P → List Int
  removed : Fin P → List Int
  delDurable : Fin P → List Int
structure State (P : Nat) where
  s : Sys P
  g : Ghost P

def project (x : State P) : Storage21.State P := ⟨x.s.toSys, x.g.toGhost⟩
def embed (x : Storage21.State P) : State P :=
  ⟨⟨x.s, fun _ => [], fun _ => []⟩, ⟨x.g, fun _ => [], fun _ => [], fun _ => []⟩⟩
def lift (x : State P) (y : Storage21.State P) : State P :=
  ⟨{x.s with toSys := y.s}, {x.g with toGhost := y.g}⟩
def commit (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String) : State P :=
  let y := Storage21.commit c (project x) s.toSys g.toGhost n
  ⟨{s with toSys := y.s}, {g with toGhost := y.g}⟩

def initWith (c : Config P) (disk : Fin P → Int) : State P :=
  embed {Storage21.init c with s := {(Storage21.init c).s with disk, stable := disk}}
def initDisk (_c : Config P) (p : Fin P) : Int :=
  if p.val == 0 then 1 else if p.val == 1 then 2 else absent
def init (c : Config P) : State P := initWith c (initDisk c)

def opCur (s : Sys P) (p : Fin P) : Int := if (s.pages p).held then (s.pages p).buf else s.disk p
def opEdit (s : Sys P) (p : Fin P) (t v : Int) : Page :=
  let pg := s.pages p
  let d := s.disk p
  let base := if pg.held then pg else {held := true, buf := d, base := d, obs := d : Page}
  {base with buf := t, typed := true, ver := v}
def opClean (s : Sys P) (p : Fin P) : Bool := !(s.pages p).held || clean (s.pages p)
def opFree (s : Sys P) (ps : List (Fin P)) : Bool := !(s.job.on && ps.contains s.job.p)

def rename (c : Config P) (x : State P) : State P :=
  let j := x.s.job
  let violates := !x.g.guard || (x.s.disk j.p != j.base && !c.r1)
  let unseen := x.g.guard && j.bytes != j.base &&
    !((x.g.seen j.p).contains j.base || (x.g.wrote j.p).any (fun w => w.1 == j.base) ||
      (x.g.opRead j.p).contains j.base)
  let del := j.bytes == absent && x.s.disk j.p != absent
  let trash := if del && c.mutant != "MDT" then
    put x.s.trash j.p (union (x.s.trash j.p) [x.s.disk j.p]) else x.s.trash
  commit c x {x.s with disk := put x.s.disk j.p j.bytes, trash, job := {j with phase := 3, ep := x.g.ext j.p}}
    {x.g with guard := false, wrote := put x.g.wrote j.p (union (x.g.wrote j.p) [(j.bytes,j.ver)]), removed := if del then put x.g.removed j.p (union (x.g.removed j.p) [x.s.disk j.p]) else x.g.removed, bad := union (union x.g.bad (tags violates "C-overwrote-external")) (tags unseen "G-overwrote-unseen")} "rename"
def dirSync (c : Config P) (x : State P) (ok : Bool) : State P :=
  let y := lift x (Storage21.dirSync c (project x) ok)
  let j := x.s.job
  let durable := ok && !c.weak && x.s.disk j.p == j.bytes
  {y with s := {y.s with trashStable := if durable && c.mutant != "MTS" then put x.s.trashStable j.p (x.s.trash j.p) else x.s.trashStable}, g := {y.g with delDurable := if durable && j.bytes == absent then put x.g.delDurable j.p (x.g.removed j.p) else x.g.delDurable}}
def power (c : Config P) (x : State P) (keep : Fin P → Bool) : State P :=
  let y := lift x (Storage21.power c (project x) keep)
  let nt := fun p => if keep p then x.s.trash p else x.s.trashStable p
  let nr := fun p => if keep p then x.g.removed p else x.g.delDurable p
  {y with s := {y.s with trash := nt, trashStable := nt}, g := {y.g with removed := nr, delDurable := fun p => (x.g.delDurable p).filter (nr p).contains}}

def opRename (c : Config P) (x : State P) (src dst : Fin P)
    (refs : List (Fin P)) (rt : Fin P → Int) : State P :=
  let text := opCur x.s src
  let full := text != absent
  let named := if full then [src,dst] else []
  let all3 := union named refs
  let v := fun p : Fin P => x.g.vc + 1 + p.val
  let nd := {opEdit x.s dst text (v dst) with risk := true}
  let ns := {opEdit x.s src absent (v src) with risk := true}
  let pages0 := if full then put (put x.s.pages dst nd) src ns else x.s.pages
  let nr := fun r => {opEdit x.s r (rt r) (v r) with risk := true}
  let pages := refs.foldl (fun acc r => put acc r (nr r)) pages0
  let drafted0 := if !full then x.s.drafts
    else if c.mutant == "MRN1" then put x.s.drafts src (draftEntry ns)
    else if c.mutant == "MRN5" then put x.s.drafts dst (draftEntry nd)
    else put (put x.s.drafts dst (draftEntry nd)) src (draftEntry ns)
  let drafted := if c.mutant == "MRN4" then drafted0 else
    refs.foldl (fun acc r => put acc r (draftEntry (nr r))) drafted0
  let gotten := union (union (refs.map fun r => (r,rt r)) (if full then [(dst,text)] else []))
    (if full && c.mutant != "MRN5" then [(src,absent)] else [])
  let read := fun p => union (x.g.opRead p)
    (if p == dst && full then [opCur x.s dst].filter fun t => t == absent
     else if (p == src && full) || refs.contains p then
       [if (x.s.pages p).held then (x.s.pages p).base else x.s.disk p] else [])
  commit c x {x.s with pages, drafts := drafted}
    {x.g with vc := x.g.vc + P, mine := union named (refs.filter fun r => opClean x.s r), opRead := read, wrote := gotten.foldl (fun acc e => put acc e.1 (union (acc e.1) [(e.2,v e.1)])) x.g.wrote, promise := gotten.foldl (fun acc e => put acc e.1 (ack (acc e.1) e.2 (v e.1) false 0)) x.g.promise, bad := union (union x.g.bad (tags (!all3.all (opClean x.s)) "D4-unclean-page-changed")) (tags (full && opCur x.s dst != absent) "rename-clobbered-target")}
    (if full then "opRename" else "opRenameRefs")
def opDelete (c : Config P) (x : State P) (p : Fin P) : State P :=
  let pg := x.s.pages p
  let v1 := x.g.vc + 1
  let np := {opEdit x.s p absent v1 with risk := c.mutant != "MDD"}
  commit c x {x.s with pages := put x.s.pages p np, drafts := if c.mutant == "MDD" then x.s.drafts else put x.s.drafts p (draftEntry np)}
    {x.g with vc := v1, mine := [p], opRead := put x.g.opRead p (union (x.g.opRead p) [if pg.held then pg.base else x.s.disk p]), wrote := if c.mutant == "MDD" then x.g.wrote else put x.g.wrote p (union (x.g.wrote p) [(absent,v1)]), promise := if c.mutant == "MDD" then x.g.promise else put x.g.promise p (ack (x.g.promise p) absent v1 false 0)} "opDelete"
def flushDel (c : Config P) (x : State P) (p : Fin P) : State P :=
  lift x (Storage21.flush c (project x) p)

inductive Action (P : Nat) where
  | old (a : Storage21.Action P)
  | opRename (src dst : Fin P) (refs : List (Fin P)) (rt : Fin P → Int)
  | opDelete (p : Fin P)
  | flushDel (p : Fin P)

def enabled (c : Config P) (x : State P) (a : Action P) : Bool :=
  match a with
  | .old a => Storage21.enabled c (project x) a
  | .opRename src dst refs rt =>
    let full := opCur x.s src != absent
    let named := if full then [src,dst] else []
    let all3 := union named refs
    x.s.alive && src != dst && opFree x.s all3 &&
      refs.all (fun r => r != src && r != dst && c.texts.contains (rt r)) &&
      (!full || opClean x.s src) &&
      (!full || ((opCur x.s dst == absent && opClean x.s dst) || c.mutant == "MRN3")) &&
      refs.all (fun r => opClean x.s r || c.mutant == "MRN2") && !all3.isEmpty
  | .opDelete p => x.s.alive && opFree x.s [p] && opClean x.s p && opCur x.s p != absent
  | .flushDel p => x.s.alive && !x.s.job.on && (x.s.pages p).held &&
    !(x.s.pages p).conflict && (x.s.pages p).buf == absent && dirty (x.s.pages p)
-- s3 generalizes MDO to dropping every other page, whereas the inherited
-- generalized s2 executable drops the cyclic next page. This mutant is explicit.
def upSubmitMDO (c : Config P) (x : Storage21.State P) (sys : Storage21.Sys P) (m : Up P) : Storage21.State P :=
  let pg := sys.pages m.p
  let v1 := x.g.vc + 1
  let resolve := m.ro != noneBytes
  let nw := if resolve then {pg with buf := m.t,base := m.ro,typed := true,ver := v1, conflict := pg.obs != m.ro,risk := pg.risk || pg.obs != m.ro} else submitTo c pg m.t m.bv v1
  let stale := !resolve && m.bv != pg.ver
  let pages := fun q => if q == m.p then nw else {}
  if !pg.held then Storage21.commit c x sys {x.g with bad := union x.g.bad ["request-to-unheld"]} "uSubmit"
  else Storage21.commit c x (ackMail {sys with pages} m.p m.cur v1 (nw.buf == m.t))
    {x.g with vc := v1,mine := if stale then [] else [m.p], owed := exclude x.g.owed [(m.p,m.t)],bad := union x.g.bad (tags (nw.buf != m.t) "B'-submit-not-taken")}
    (if resolve then "uResolve" else if stale then "uStale" else "uSubmit")
def deliverUpMDO (c : Config P) (x : Storage21.State P) (m : Up P) (tail : List (Up P)) (ok : Bool) : Storage21.State P :=
  if ["open","close","discard","op"].contains m.kind then Storage21.deliverUp c x m tail ok
  else upSubmitMDO c x {x.s with up := tail} m

def executeOld (c : Config P) (x : State P) (a : Storage21.Action P) : State P :=
  match a with
  | .rename => rename c x
  | .dirSync ok => dirSync c x ok
  | .power keep => power c x keep
  | .deliverUp ok =>
    if c.mutant == "MDO" then
      lift x (match x.s.up with | [] => project x | m::tail => deliverUpMDO c (project x) m tail ok)
    else lift x (Storage21.execute c (project x) (.deliverUp ok))
  | a => lift x (Storage21.execute c (project x) a)
def execute (c : Config P) (x : State P) (a : Action P) : State P :=
  match a with
  | .old a => executeOld c x a
  | .opRename src dst refs rt => opRename c x src dst refs rt
  | .opDelete p => opDelete c x p
  | .flushDel p => flushDel c x p
def step (c : Config P) (x : State P) (a : Action P) : Option (State P) :=
  if enabled c x a then some (execute c x a) else none

def noLoss (x : State P) : Bool := Storage21.noLoss (project x)
def accepted (x : State P) : Bool := Storage21.accepted (project x)
def transitions (x : State P) : Bool := Storage21.transitions (project x)
def trashed (x : State P) : Bool := (ids P).all fun p => (x.g.removed p).all fun b =>
  (x.s.trash p).contains b || x.s.disk p == b
def guarantee (x : State P) : Bool := noLoss x && transitions x && accepted x && trashed x

inductive Reachable (c : Config P) (disk : Fin P → Int) : State P → Prop where
  | initial : Reachable c disk (initWith c disk)
  | advance {x y} : Reachable c disk x → (a : Action P) → step c x a = some y → Reachable c disk y
end Storage3
