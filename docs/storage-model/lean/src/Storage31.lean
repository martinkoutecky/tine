import Storage3
namespace Storage31
open Storage21
abbrev State := Storage3.State
abbrev initWith := @Storage3.initWith
abbrev initDisk := @Storage3.initDisk
abbrev init := @Storage3.init
abbrev guarantee := @Storage3.guarantee

-- Successful one-path read; the ordinary commit also updates any live mailbox.
def load (c : Config P) (x : State P) (p : Fin P) : State P :=
  let d := x.s.disk p
  let v1 := x.g.vc + 1
  Storage3.commit c x {x.s with pages := put x.s.pages p ({held := true, buf := d, base := d, obs := d, ver := v1} : Page)}
    {x.g with vc := v1} "load"

def opHeld (c : Config P) (x : State P) (ps : List (Fin P)) : Bool :=
  c.mutant == "MUL" || ps.all (fun p => (x.s.pages p).held)

inductive Action (P : Nat) where
  | old (a : Storage21.Action P)
  | opRename (src dst : Fin P) (refs : List (Fin P)) (rt : Fin P → Int)
  | opDelete (p : Fin P)
  | flushDel (p : Fin P)
  | load (p : Fin P)

def baseline : Action P → Storage3.Action P
  | .old a => .old a
  | .opRename src dst refs rt => .opRename src dst refs rt
  | .opDelete p => .opDelete p
  | .flushDel p => .flushDel p
  | .load p => .opDelete p -- consulted only by the four non-load branches

def enabled (c : Config P) (x : State P) : Action P → Bool
  | .load p => x.s.alive && !(x.s.pages p).held
  | .opRename src dst refs rt =>
    let named := if Storage3.opCur x.s src != absent then [src,dst] else []
    Storage3.enabled c x (.opRename src dst refs rt) && opHeld c x (union (union named refs) [src])
  | .opDelete p => Storage3.enabled c x (.opDelete p) && opHeld c x [p]
  | .old a => Storage3.enabled c x (.old a)
  | .flushDel p => Storage3.enabled c x (.flushDel p)

def execute (c : Config P) (x : State P) (a : Action P) : State P :=
  match a with
  | .load p => load c x p
  | a => Storage3.execute c x (baseline a)
def step (c : Config P) (x : State P) (a : Action P) : Option (State P) :=
  if enabled c x a then some (execute c x a) else none
inductive Reachable (c : Config P) (disk : Fin P → Int) : State P → Prop where
  | initial : Reachable c disk (initWith c disk)
  | advance {x y} : Reachable c disk x → (a : Action P) → step c x a = some y → Reachable c disk y

-- A non-load s3.1 step uses the identical update with a stronger guard.
theorem baseline_enabled (c : Config P) (x : State P) (a : Action P)
    (notLoad : ∀ p, a ≠ .load p) (h : enabled c x a = true) :
    Storage3.enabled c x (baseline a) = true := by
  cases a <;> simp_all [enabled,baseline,Bool.and_eq_true]

def openMsg (c : Config P) (p : Fin P) : Up P :=
  {kind := "open", p, q := zero c, cur := false, t := 0, bv := 0, ro := noneBytes}

-- This equality has no queue/reachability premise: the helper reads the page
-- without consuming a request and cur=false prevents an explicit answer.
theorem load_upOpen (c : Config P) (x : State P) (p : Fin P)
    (unheld : (x.s.pages p).held = false) :
    Storage3.project (load c x p) =
      Storage21.upOpen c (Storage3.project x) x.s.toSys (openMsg c p) true := by
  simp [load,Storage3.commit,Storage3.project,Storage21.upOpen,openMsg,unheld,
    ackMail,Storage21.commit,keptB,keptW]
end Storage31
