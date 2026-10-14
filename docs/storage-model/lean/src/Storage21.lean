import Std
namespace Storage21

def absent : Int := -1
def noneBytes : Int := -2
def unknown : Int := -3

structure Page where
  held : Bool := false
  buf : Int := absent
  base : Int := absent
  ver : Int := 0
  risk : Bool := false
  conflict : Bool := false
  typed : Bool := false
  obs : Int := absent
  deriving BEq, Repr, DecidableEq
structure Draft where
  bytes : Int := noneBytes
  base : Int := noneBytes
  ver : Int := 0
  deriving BEq, Repr, DecidableEq
structure Job (P : Nat) where
  on : Bool
  p : Fin P
  bytes : Int := 0
  base : Int := 0
  ver : Int := 0
  phase : Int := 0
  ep : Int := 0
  deriving Repr
structure W where
  on : Bool := false
  text : Int := absent
  bv : Int := -1
  pend : Bool := false
  sent : Bool := false
  obs : Int := absent
  conf : Bool := false
  deriving BEq, Repr, DecidableEq
structure Mail where
  on : Bool := false
  held : Bool := false
  ver : Int := 0
  text : Int := absent
  obs : Int := absent
  conf : Bool := false
  ack : Int := -1
  took : Bool := false
  deriving BEq, Repr, DecidableEq
structure Up (P : Nat) where
  kind : String
  p : Fin P
  q : Fin P
  t : Int
  t2 : Int := 0
  bv : Int
  bv2 : Int := 0
  ro : Int
  cur : Bool := true
  deriving Repr
structure Promise where
  on : Bool := false
  bytes : Int := noneBytes
  ver : Int := 0
  saved : Bool := false
  ep : Int := 0
  deriving BEq, Repr, DecidableEq
structure Sys (P : Nat) where
  alive : Bool
  disk : Fin P → Int
  stable : Fin P → Int
  drafts : Fin P → Draft
  pages : Fin P → Page
  job : Job P
  w : Fin P → W
  mb : Fin P → Mail
  up : List (Up P)
structure Ghost (P : Nat) where
  vc : Int
  promise : Fin P → Promise
  ext : Fin P → Int
  wrote : Fin P → List (Int × Int)
  owed : List (Fin P × Int)
  guard : Bool
  seen : Fin P → List Int
  mine : List (Fin P)
  bad : List String
structure State (P : Nat) where
  s : Sys P
  g : Ghost P
structure Config (P : Nat) where
  positive : 0 < P
  texts : List Int
  nonempty : texts ≠ []
  r1 : Bool := false
  weak : Bool := false
  crashes : Bool := true
  powers : Bool := true
  mutant : String := "none"

def ids (P : Nat) : List (Fin P) := List.finRange P
def zero (c : Config P) : Fin P := ⟨0, c.positive⟩
def next (c : Config P) (p : Fin P) : Fin P :=
  ⟨(p.val + 1) % P, Nat.mod_lt _ c.positive⟩
def put [DecidableEq α] (m : α → β) (p : α) (v : β) : α → β :=
  fun q => if q = p then v else m q
def union [BEq α] (a b : List α) : List α :=
  b.foldl (fun xs x => if xs.contains x then xs else xs ++ [x]) a
def exclude [BEq α] (a b : List α) : List α := a.filter fun x => !b.contains x
def tags (condition : Bool) (tag : String) : List String := if condition then [tag] else []
def clean (p : Page) : Bool := !p.typed && p.buf == p.base && !p.risk
def dirty (p : Page) : Bool := !clean p
def draftEntry (p : Page) : Draft := if p.held && p.risk then ⟨p.buf,p.base,p.ver⟩ else {}
def nojob (c : Config P) : Job P := {on := false, p := zero c}
def table (c : Config P) (pg : Page) (v nv : Int) : Page :=
  let q := {pg with obs := v}
  if c.mutant == "MQ" && v == q.buf then {q with base := v, conflict := false, risk := false, typed := false}
  else if v == q.base then {q with conflict := false}
  else if clean q || (c.mutant == "MLO" && !q.risk) then
    {q with buf := v, base := v, ver := nv, conflict := false, typed := false}
  else if v == q.buf then {q with base := v, conflict := false}
  else {q with conflict := true, risk := true}
def onReply (c : Config P) (pg : Page) (out : String) (bytes : Int) : Page :=
  if out == "Published" || (out == "Uncertain" && c.mutant == "MLS") then
    {pg with base := bytes, typed := false, risk := false}
  else {pg with risk := true}
def submitTo (c : Config P) (pg : Page) (t bv v : Int) : Page :=
  if c.mutant == "MIS" then pg
  else if bv == pg.ver || c.mutant == "MST" then {pg with buf := t, typed := true, ver := v}
  else {pg with buf := t, base := unknown, typed := true, ver := v, conflict := true, risk := true}
def ack (pr : Promise) (bytes ver : Int) (saved : Bool) (ep : Int) : Promise :=
  if ver > pr.ver || (ver == pr.ver && pr.on && saved && !pr.saved) then
    ⟨true,bytes,ver,saved,ep⟩ else pr
def endPromise (pr : Promise) (watermark : Int) : Promise := {pr with on := false, ver := watermark}
def down (c : Config P) (s : Sys P) : Sys P :=
  {s with alive := false, pages := fun _ => {}, job := nojob c,     w := fun _ => {}, mb := fun _ => {}, up := []}
def gDown (g : Ghost P) : Ghost P := {g with owed := [], guard := false}
def locked (s : Sys P) (p : Fin P) : Bool := s.job.on && s.job.p == p
def window (s : Sys P) (p : Fin P) : Bool := s.job.on && s.job.phase == 2 && s.job.p == p
def ackMail (s : Sys P) (p : Fin P) (ok : Bool) (v : Int) (took : Bool) : Sys P :=
  if !ok then s else
    let pg := s.pages p
    {s with mb := put s.mb p ⟨true,pg.held,pg.ver,pg.buf,pg.obs,pg.conflict,v,took⟩}
def up1 (c : Config P) (kind : String) (p : Fin P) (t bv ro : Int) : Up P :=
  {kind,p,q := zero c,t,bv,ro}
def carries (u : Up P) : List (Fin P × Int) :=
  if u.kind == "submit" then [(u.p,u.t)] else if u.kind == "op" then [(u.p,u.t),(u.q,u.t2)] else []
def keptB (x : State P) (p : Fin P) (ns : Sys P) (np : Fin P → Promise)
    (mine : List (Fin P)) (name : String) : Bool :=
  let a := x.s.pages p
  let b := ns.pages p
  let pr := np p
  let replaced := a.held && (!b.held || b.buf != a.buf)
  let safe := (!a.typed && a.buf == a.obs) || (pr.ver >= a.ver && pr.bytes == a.buf)
  let shown := (name == "uStale" || name == "uOpStale") && ((x.g.seen p).contains a.buf || !a.typed)
  !replaced || mine.contains p || name == "crash" || name == "power" || safe || shown ||
    (name == "switchFin" && (ns.drafts p).bytes == a.buf)
def keptW (x : State P) (p : Fin P) (ns : Sys P) (owed : List (Fin P × Int)) (name : String) : Bool :=
  let a := x.s.w p
  let b := ns.w p
  let replaced := a.on && (!b.on || b.text != a.text)
  !replaced || ["wEdit","wResolve","wOp","windowCrash","crash","power"].contains name ||
    (!a.pend && (!a.sent || (name == "wAck" && !owed.contains (p,a.text))))
def commit (c : Config P) (x : State P) (ns : Sys P) (ng : Ghost P) (name : String) : State P :=
  let np := fun p =>
    let a := x.s.pages p
    let b := ns.pages p
    if !(ng.mine.contains p) && a.held && dirty a && b.held && clean b && b.buf == a.buf then
      ack (ng.promise p) b.buf b.ver false 0 else ng.promise p
  let mb := fun p =>
    let a := x.s.pages p
    let b := ns.pages p
    let m := ns.mb p
    let wp := ns.w p
    if ns.alive && (wp.on || wp.sent) &&
      (a.ver != b.ver || a.buf != b.buf || a.obs != b.obs || a.conflict != b.conflict || a.held != b.held) then
      ⟨true,b.held,b.ver,b.buf,b.obs,b.conflict,if m.on then m.ack else -1,m.on && m.took⟩
    else m
  {s := {ns with mb}, g := {ng with promise := np, mine := [],     seen := fun p => union (ng.seen p) ([ (ns.w p).text ].filter fun t =>       (ns.w p).on && (t == absent || c.texts.contains t)),     bad := union (union ng.bad (tags (!(ids P).all fun p => keptB x p ns np ng.mine name) "B-unsaved-replaced"))       (tags (!(ids P).all fun p => keptW x p ns ng.owed name) "B'-window-replaced")}}
def init (c : Config P) : State P :=
  let d := fun p : Fin P => c.texts[p.val % c.texts.length]!
  {s := {alive := true, disk := d, stable := d, drafts := fun _ => {}, pages := fun _ => {},     job := nojob c, w := fun _ => {}, mb := fun _ => {}, up := []},    g := {vc := 0, promise := fun _ => {}, ext := fun _ => 0, wrote := fun _ => [],     owed := [], guard := false, seen := fun _ => [], mine := [], bad := []}}

-- Window actions. Preconditions are tested separately by the executable dispatcher.
def wOpen (c : Config P) (x : State P) (p : Fin P) : State P :=
  commit c x {x.s with w := put x.s.w p {sent := true},     up := x.s.up ++ [up1 c "open" p 0 0 noneBytes]} x.g "wOpen"
def wEdit (c : Config P) (x : State P) (p : Fin P) (v : Int) : State P :=
  commit c x {x.s with w := put x.s.w p {x.s.w p with text := v, pend := true}} x.g "wEdit"
def wSend (c : Config P) (x : State P) (p : Fin P) : State P :=
  let a := x.s.w p
  commit c x {x.s with w := put x.s.w p {a with pend := false, sent := true},     up := x.s.up ++ [up1 c "submit" p a.text a.bv noneBytes]}
    {x.g with owed := union x.g.owed [(p,a.text)]} "wSend"
def wResolve (c : Config P) (x : State P) (p : Fin P) (v : Int) : State P :=
  let a := x.s.w p
  commit c x {x.s with w := put x.s.w p {a with text := v, pend := false, sent := true},     up := x.s.up ++ [up1 c "submit" p v a.bv a.obs]}
    {x.g with owed := union x.g.owed [(p,v)], seen := put x.g.seen p       (union (x.g.seen p) ([a.obs].filter fun t => t == absent || c.texts.contains t))} "wResolve"
def wDiscard (c : Config P) (x : State P) (p : Fin P) : State P :=
  commit c x {x.s with w := put x.s.w p {x.s.w p with pend := false, sent := true},     up := x.s.up ++ [up1 c "discard" p 0 (x.s.w p).bv noneBytes]} x.g "wDiscard"
def wOp (c : Config P) (x : State P) (src dst : Fin P) (dS dD : Int) : State P :=
  let a := x.s.w src
  let b := x.s.w dst
  commit c x {x.s with w := put (put x.s.w src {a with text := dS, sent := true}) dst {b with text := dD, sent := true},     up := x.s.up ++ [{kind := "op",p := src,q := dst,t := dS,t2 := dD,bv := a.bv,bv2 := b.bv,ro := noneBytes}]}
    {x.g with owed := union x.g.owed [(src,dS),(dst,dD)]} "wOp"
def wClose (c : Config P) (x : State P) (p : Fin P) : State P :=
  commit c x {x.s with w := put x.s.w p {}, up := x.s.up ++ [up1 c "close" p 0 0 noneBytes]} x.g "wClose"
def wRecv (c : Config P) (x : State P) (p : Fin P) : State P :=
  let m := x.s.mb p
  let a := x.s.w p
  let isAck := (m.ack >= 0 || c.mutant == "MAK") && a.sent
  let na :=
    if isAck && !m.held then {}
    else if isAck && a.pend then {a with sent := false, bv := if m.took || c.mutant == "MDA" then m.ack else a.bv, obs := m.obs, conf := m.conf}
    else if isAck then ⟨true,m.text,m.ver,false,false,m.obs,m.conf⟩
    else if a.on && !a.sent && (!a.pend || c.mutant == "MWA") then {a with text := m.text, bv := m.ver, obs := m.obs, conf := m.conf}
    else if a.on then {a with obs := m.obs, conf := m.conf}
    else a
  commit c x {x.s with w := put x.s.w p na, mb := put x.s.mb p {}} x.g (if isAck then "wAck" else "wRecv")

-- Backend request actions.
def upOpen (c : Config P) (x : State P) (sys : Sys P) (m : Up P) (rok : Bool) : State P :=
  let pg := sys.pages m.p
  let d := x.s.disk m.p
  let v1 := x.g.vc + 1
  let np := if pg.held || !rok then pg else {held := true,buf := d,base := d,obs := d,ver := v1 : Page}
  commit c x (ackMail {sys with pages := put sys.pages m.p np} m.p m.cur np.ver false)
    (if np.ver == v1 then {x.g with vc := v1} else x.g) (if pg.held || rok then "uOpen" else "uOpenFail")
def upClose (c : Config P) (x : State P) (sys : Sys P) (m : Up P) : State P :=
  let pg := sys.pages m.p
  let go := pg.held && (clean pg || (c.mutant == "MCL" && !pg.risk))
  commit c x (if go then {sys with pages := put sys.pages m.p {}} else sys) x.g "uClose"
def upDiscard (c : Config P) (x : State P) (sys : Sys P) (m : Up P) (rok : Bool) : State P :=
  let pg := sys.pages m.p
  let d := x.s.disk m.p
  let v1 := x.g.vc + 1
  let pr := x.g.promise m.p
  let hold := pg.risk && (d == pg.buf || d == (x.s.drafts m.p).bytes) && c.mutant != "MDE"
  let nw := {pg with buf := d,base := d,ver := v1,obs := d,conflict := false,risk := hold,typed := false}
  let live := pr.on && !(pr.saved && x.g.ext m.p > pr.ep)
  let npr := if live && pr.bytes == d then pr else endPromise pr v1
  if !pg.held then commit c x sys {x.g with bad := union x.g.bad ["request-to-unheld"]} "uDiscard"
  else if !rok then commit c x (ackMail sys m.p m.cur pg.ver false) x.g "uDiscardFail"
  else commit c x (ackMail {sys with pages := put sys.pages m.p nw} m.p m.cur v1 false)
    {x.g with vc := v1,promise := put x.g.promise m.p npr,mine := [m.p]} "uDiscard"
def upSubmit (c : Config P) (x : State P) (sys : Sys P) (m : Up P) : State P :=
  let pg := sys.pages m.p
  let v1 := x.g.vc + 1
  let resolve := m.ro != noneBytes
  let nw := if resolve && c.mutant != "MIS" then {pg with buf := m.t,base := m.ro,typed := true,ver := v1,     conflict := pg.obs != m.ro,risk := pg.risk || pg.obs != m.ro} else submitTo c pg m.t m.bv v1
  let stale := !resolve && m.bv != pg.ver
  let pages := if c.mutant == "MDO" then put (put sys.pages m.p nw) (next c m.p) {} else put sys.pages m.p nw
  if !pg.held then commit c x sys {x.g with bad := union x.g.bad ["request-to-unheld"]} "uSubmit"
  else commit c x (ackMail {sys with pages} m.p m.cur v1 (nw.buf == m.t))
    {x.g with vc := v1,mine := if stale then [] else [m.p],       owed := exclude x.g.owed [(m.p,m.t)],bad := union x.g.bad (tags (nw.buf != m.t) "B'-submit-not-taken")}
    (if resolve then "uResolve" else if stale then "uStale" else "uSubmit")
def upOp (c : Config P) (x : State P) (sys : Sys P) (m : Up P) (dok : Bool) : State P :=
  let a := sys.pages m.p
  let b := sys.pages m.q
  let vD := x.g.vc + 1
  let vS := x.g.vc + 2
  let nb := {submitTo c b m.t2 m.bv2 vD with risk := true}
  let na := submitTo c a m.t m.bv vS
  let stale := [m.p,m.q].filter fun p => if p == m.p then m.bv != a.ver else m.bv2 != b.ver
  let drafted := if c.mutant == "MOD" then sys.drafts else put sys.drafts m.q (draftEntry nb)
  let owed := exclude x.g.owed [(m.p,m.t),(m.q,m.t2)]
  if !a.held || !b.held then commit c x sys {x.g with bad := union x.g.bad ["request-to-unheld"]} "uOp"
  else if (!dok && c.mutant != "MOD") || (!stale.isEmpty && c.mutant != "MSM") then
    commit c x (ackMail (ackMail sys m.q m.cur b.ver false) m.p m.cur a.ver false) {x.g with owed} "uOpRefused"
  else commit c x (ackMail (ackMail {sys with drafts := drafted,pages := put (put sys.pages m.q nb) m.p na}
    m.q m.cur vD (nb.buf == m.t2)) m.p m.cur vS (na.buf == m.t))
    {x.g with vc := vS,owed,mine := exclude [m.p,m.q] stale,       wrote := put x.g.wrote m.q (union (x.g.wrote m.q) [(m.t2,vD)]),       promise := put x.g.promise m.q (ack (x.g.promise m.q) m.t2 vD false 0),       bad := union x.g.bad (tags (!(nb.buf == m.t2 && na.buf == m.t)) "B'-submit-not-taken")}
    (if stale.isEmpty then "uOp" else "uOpStale")
def deliverUp (c : Config P) (x : State P) (m : Up P) (tail : List (Up P)) (rok : Bool) : State P :=
  let rest := {x.s with up := tail}
  if m.kind == "open" then upOpen c x rest m rok
  else if m.kind == "close" then upClose c x rest m
  else if m.kind == "discard" then upDiscard c x rest m rok
  else if m.kind == "op" then upOp c x rest m rok
  else upSubmit c x rest m

def observe (c : Config P) (x : State P) (p : Fin P) : State P :=
  let v1 := x.g.vc + 1
  let nw := table c (x.s.pages p) (x.s.disk p) v1
  commit c x {x.s with pages := put x.s.pages p nw}
    {x.g with vc := if nw.ver == v1 then v1 else x.g.vc} "observe"
def flush (c : Config P) (x : State P) (p : Fin P) : State P :=
  let pg := x.s.pages p
  commit c x {x.s with job := ⟨true,p,pg.buf,pg.base,pg.ver,1,0⟩} x.g "flush"
def check (c : Config P) (x : State P) : State P :=
  let p := x.s.job.p
  let d := x.s.disk p
  let v1 := x.g.vc + 1
  if d != x.s.job.base && c.mutant != "MC" && c.mutant != "MC1" then
    let nw := table c (x.s.pages p) d v1
    commit c x {x.s with job := nojob c,pages := put x.s.pages p nw}
      {x.g with vc := if nw.ver == v1 then v1 else x.g.vc} "mismatch"
  else commit c x {x.s with job := {x.s.job with phase := 2}} {x.g with guard := d == x.s.job.base} "check"
def rename (c : Config P) (x : State P) : State P :=
  let j := x.s.job
  let violates := !x.g.guard || (x.s.disk j.p != j.base && !c.r1)
  let unseen := x.g.guard && j.bytes != j.base &&
    !((x.g.seen j.p).contains j.base || (x.g.wrote j.p).any fun w => w.1 == j.base)
  commit c x {x.s with disk := put x.s.disk j.p j.bytes,job := {j with phase := 3,ep := x.g.ext j.p}}
    {x.g with guard := false,wrote := put x.g.wrote j.p (union (x.g.wrote j.p) [(j.bytes,j.ver)]),       bad := union (union x.g.bad (tags violates "C-overwrote-external")) (tags unseen "G-overwrote-unseen")} "rename"
def dirSync (c : Config P) (x : State P) (ok : Bool) : State P :=
  let j := x.s.job
  let d := x.s.disk j.p
  let st := if ok && !c.weak && d == j.bytes then put x.s.stable j.p d else x.s.stable
  let nw := if !ok && c.mutant == "MRE" then {x.s.pages j.p with base := j.bytes,typed := false,risk := false}
    else onReply c (x.s.pages j.p) (if ok then "Published" else "Uncertain") j.bytes
  commit c x {x.s with stable := st,job := nojob c,pages := put x.s.pages j.p nw}
    {x.g with promise := if ok then put x.g.promise j.p (ack (x.g.promise j.p) j.bytes j.ver true j.ep) else x.g.promise} "dirSync"
def saveFail (c : Config P) (x : State P) : State P :=
  commit c x {x.s with job := nojob c,pages := put x.s.pages x.s.job.p     (onReply c (x.s.pages x.s.job.p) "Failed" x.s.job.bytes)} {x.g with guard := false} "saveFail"
def draftSync (c : Config P) (x : State P) (p : Fin P) : State P :=
  let e0 := draftEntry (x.s.pages p)
  let e := if c.mutant == "MBD" && e0.base == unknown then {e0 with base := x.s.disk p} else e0
  let ng := if e.bytes != noneBytes then {x.g with     wrote := put x.g.wrote p (union (x.g.wrote p) [(e.bytes,e.ver)]),     promise := put x.g.promise p (ack (x.g.promise p) e.bytes e.ver false 0)} else x.g
  commit c x {x.s with drafts := put x.s.drafts p e} ng "draftWrite"
def switchReq (c : Config P) (x : State P) : State P :=
  commit c x {x.s with pages := fun p => let pg := x.s.pages p; if pg.held && dirty pg then {pg with risk := true} else pg} x.g "switchReq"
def canSwitch (c : Config P) (x : State P) : Bool :=
  x.s.alive && !x.s.job.on && x.s.up.isEmpty && (ids P).all fun p =>
    let pg := x.s.pages p
    let a := x.s.w p
    !a.pend && !a.sent && (!pg.held || clean pg || pg.risk) &&
      (x.s.drafts p == draftEntry pg || (c.mutant == "MX" && pg.held && pg.risk))
def switchFin (c : Config P) (x : State P) : State P := commit c x (down c x.s) (gDown x.g) "switchFin"
def extWriteD (c : Config P) (x : State P) (p : Fin P) (v : Int) (dur : Bool) : State P :=
  commit c x {x.s with disk := put x.s.disk p v,stable := if dur then put x.s.stable p v else x.s.stable}
    {x.g with ext := put x.g.ext p (x.g.ext p + 1)} "extWrite"
def extWrite (c : Config P) (x : State P) (p : Fin P) (v : Int) : State P :=
  extWriteD c x p v true
def crash (c : Config P) (x : State P) : State P := commit c x (down c x.s) (gDown x.g) "crash"
def windowCrash (c : Config P) (x : State P) : State P :=
  commit c x {x.s with w := fun _ => {},mb := fun _ => {},     up := if c.mutant == "MDQ" then [] else x.s.up.map fun u => {u with cur := false}} x.g "windowCrash"
def power (c : Config P) (x : State P) (keep : Fin P → Bool) : State P :=
  let nd := fun p => if keep p then x.s.disk p else x.s.stable p
  let np := fun p => let pr := x.g.promise p
    let kept := [nd p,(x.s.drafts p).bytes].contains pr.bytes
    if c.weak && pr.on && pr.saved && !kept then endPromise pr pr.ver else pr
  commit c x {down c x.s with disk := nd,stable := nd} {gDown x.g with promise := np} "power"
def restored (c : Config P) (x : State P) (p : Fin P) : Page :=
  let d := x.s.drafts p
  if d.bytes == noneBytes then {} else
    {held := true,buf := d.bytes,      base := if c.mutant == "MRB" && d.base == unknown then x.s.disk p else d.base,      obs := noneBytes,ver := x.g.vc + 1 + ((ids P).filter fun q => q.val < p.val && (x.s.drafts q).bytes != noneBytes).length,      risk := true,typed := true}
def launch (c : Config P) (x : State P) : State P :=
  commit c x {x.s with alive := true,pages := restored c x}
    {x.g with vc := x.g.vc + ((ids P).filter fun p => (x.s.drafts p).bytes != noneBytes).length} "launch"

inductive Action (P : Nat) where
  | wOpen (p : Fin P) | wEdit (p : Fin P) (v : Int) | wSend (p : Fin P)
  | wResolve (p : Fin P) (v : Int) | wDiscard (p : Fin P)
  | wOp (src dst : Fin P) (dS dD : Int) | wClose (p : Fin P) | wRecv (p : Fin P)
  | deliverUp (rok : Bool) | observe (p : Fin P) | flush (p : Fin P)
  | check | rename | dirSync (ok : Bool) | saveFail | draftSync (p : Fin P)
  | switchReq | switchFin | extWriteD (p : Fin P) (v : Int) (dur : Bool)
  | crash | windowCrash | power (keep : Fin P → Bool) | launch

def enabled (c : Config P) (x : State P) (a : Action P) : Bool :=
  match a with
  | .wOpen p => x.s.alive && !(x.s.w p).on && !(x.s.w p).sent
  | .wEdit p v => x.s.alive && (x.s.w p).on && v != (x.s.w p).text && c.texts.contains v
  | .wSend p => x.s.alive && (x.s.w p).on && (x.s.w p).pend && !(x.s.w p).sent
  | .wResolve p v => x.s.alive && (x.s.w p).on && (x.s.w p).conf && !(x.s.w p).sent && c.texts.contains v
  | .wDiscard p => x.s.alive && (x.s.w p).on && !(x.s.w p).sent
  | .wOp p q t u => x.s.alive && p != q && (x.s.w p).on && (x.s.w q).on &&
      !(x.s.w p).pend && !(x.s.w p).sent && !(x.s.w q).pend && !(x.s.w q).sent &&
      t != (x.s.w p).text && u != (x.s.w q).text && c.texts.contains t && c.texts.contains u
  | .wClose p => x.s.alive && (x.s.w p).on && !(x.s.w p).pend && !(x.s.w p).sent
  | .wRecv p => x.s.alive && (x.s.mb p).on
  | .deliverUp _ => x.s.alive && match x.s.up with
      | [] => false
      | m::_ => !locked x.s m.p && (m.kind != "op" || !locked x.s m.q)
  | .observe p => x.s.alive && (x.s.pages p).held && !locked x.s p &&
      table c (x.s.pages p) (x.s.disk p) (x.g.vc + 1) != x.s.pages p
  | .flush p => x.s.alive && !x.s.job.on && (x.s.pages p).held && !(x.s.pages p).conflict &&
      (x.s.pages p).buf != absent && dirty (x.s.pages p)
  | .check => x.s.alive && x.s.job.on && x.s.job.phase == 1
  | .rename => x.s.alive && x.s.job.on && x.s.job.phase == 2
  | .dirSync _ => x.s.alive && x.s.job.on && x.s.job.phase == 3
  | .saveFail => x.s.alive && x.s.job.on && x.s.job.phase <= 2
  | .draftSync p => x.s.alive && x.s.drafts p !=
      (let e := draftEntry (x.s.pages p)
       if c.mutant == "MBD" && e.base == unknown then {e with base := x.s.disk p} else e)
  | .switchReq => x.s.alive && (ids P).any fun p => (x.s.pages p).held && dirty (x.s.pages p) && !(x.s.pages p).risk
  | .switchFin => canSwitch c x
  | .extWriteD p v _ => v != x.s.disk p && (!window x.s p || c.r1) && (v == absent || c.texts.contains v)
  | .crash => x.s.alive && c.crashes
  | .windowCrash => x.s.alive && c.crashes
  | .power _ => c.powers
  | .launch => !x.s.alive
def execute (c : Config P) (x : State P) (a : Action P) : State P :=
  match a with
  | .wOpen p => wOpen c x p | .wEdit p v => wEdit c x p v | .wSend p => wSend c x p
  | .wResolve p v => wResolve c x p v | .wDiscard p => wDiscard c x p
  | .wOp p q t u => wOp c x p q t u | .wClose p => wClose c x p | .wRecv p => wRecv c x p
  | .deliverUp rok => match x.s.up with | [] => x | m::tail => deliverUp c x m tail rok
  | .observe p => observe c x p | .flush p => flush c x p | .check => check c x
  | .rename => rename c x | .dirSync ok => dirSync c x ok | .saveFail => saveFail c x
  | .draftSync p => draftSync c x p | .switchReq => switchReq c x | .switchFin => switchFin c x
  | .extWriteD p v dur => extWriteD c x p v dur | .crash => crash c x | .windowCrash => windowCrash c x
  | .power keep => power c x keep | .launch => launch c x
def step (c : Config P) (x : State P) (a : Action P) : Option (State P) :=
  if enabled c x a then some (execute c x a) else none
def copies (x : State P) (p : Fin P) : List Int := [x.s.disk p,(x.s.drafts p).bytes]
def noLoss (x : State P) : Bool := x.s.alive || (ids P).all fun p =>
  let pr := x.g.promise p
  !pr.on || (pr.saved && x.g.ext p > pr.ep) ||
    (copies x p).any fun v =>
      v == pr.bytes || (x.g.wrote p).any fun w => w.1 == v && w.2 > pr.ver
def accepted (x : State P) : Bool := !x.s.alive || x.g.owed.all fun o => x.s.up.any fun u => (carries u).contains o
def transitions (x : State P) : Bool := x.g.bad.isEmpty
def guarantee (x : State P) : Bool := noLoss x && transitions x && accepted x
-- Diagnostic constraint only; never consulted by enabled, execute, step, or reachability.
def within (x : State P) (vMax eMax : Int) (qMax : Nat) : Bool :=
  x.g.vc <= vMax && (ids P).all (fun p => x.g.ext p <= eMax) && x.s.up.length <= qMax

end Storage21
