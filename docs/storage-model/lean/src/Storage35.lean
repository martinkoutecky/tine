import Storage34
namespace Storage35Base
open Storage21 Storage32
abbrev State := Storage32.State
abbrev Sys := Storage32.Sys
abbrev Action := Storage32.Action
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
  let revert := src != m.p && ((sys.pages src).held || c.mutant == "MRH") && (sys.pages src).buf == absent
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
    else Storage32.lift x (Storage3.executeOld c (Storage32.project x) (.deliverUp rok))
def execute (c : Config P) (x : State P) (a : Action P) : State P :=
  match a with
  | .old (.deliverUp ok) => deliverUp c x ok
  | a => Storage32.execute c x a
end Storage35Base
namespace Storage35
open Storage21
abbrev State := Storage34.State
abbrev Action := Storage34.Action
abbrev initWith := @Storage34.initWith
abbrev init := @Storage34.init
abbrev project := @Storage34.project
abbrev embed := @Storage34.embed
abbrev lift := @Storage34.lift
abbrev enabled := @Storage34.enabled
abbrev opApply := @Storage34.opApply
abbrev extClaim := @Storage34.extClaim
abbrev guarantee := @Storage34.guarantee
abbrev orderHolds := @Storage34.orderHolds
abbrev renameResolves := @Storage34.renameResolves
def execute (c : Config P) (x : State P) : Action P → State P
  | .prior a => lift x (Storage35Base.execute c (project x) a)
  | .opApply eff plan dst src => opApply c x eff plan dst src
  | .extClaim p present => extClaim c x p present
def step (c : Config P) (x : State P) (a : Action P) : Option (State P) :=
  if enabled c x a then some (execute c x a) else none
inductive Reachable (c : Config P) (disk : Fin P → Int) : State P → Prop where
  | initial : Reachable c disk (initWith c disk)
  | advance {x y} : Reachable c disk x → (a : Action P) → step c x a = some y → Reachable c disk y
end Storage35
