import Storage35
import CoreBytes21

namespace Stage6
open Storage21

abbrev Slot := Fin
abbrev WriteMap := Storage33.Map

structure Move (P : Nat) where
  dst : Slot P
  src : Slot P
  deriving DecidableEq

def endpoints (moves : List (Move P)) : List (Slot P) :=
  moves.flatMap fun m => [m.dst, m.src]

structure Batch (P : Nat) where
  eff : WriteMap P
  plan : WriteMap P
  moves : List (Move P)

def Batch.keys (b : Batch P) : List (Slot P) := Storage33.keys b.eff

def Batch.Shape (b : Batch P) : Prop :=
  b.keys ≠ [] ∧ (endpoints b.moves).Nodup ∧
    ∀ m ∈ b.moves,
      b.eff m.src = some absent ∧
      Storage33.get b.eff m.dst ≠ absent ∧ Storage33.get b.plan m.src ≠ absent

instance (b : Batch P) : Decidable b.Shape := by
  unfold Batch.Shape
  infer_instance

structure Gate (P : Nat) where
  moves : List (Move P)
  vers : Slot P → Option Int
  pending : List (Slot P)

def Gate.gated (gt : Gate P) (p : Slot P) : Bool :=
  let receiver := fun q => gt.moves.any fun m => q == m.dst
  let source := fun q => gt.moves.any fun m => q == m.src
  gt.pending.contains p && !receiver p &&
    (gt.pending.any receiver || (source p && gt.pending.any fun q => !source q))

def touches (gates : List (Gate P)) (keys : List (Slot P)) : Bool :=
  gates.any fun gt => keys.any fun p => (gt.vers p).isSome

def Gate.retained (gt : Gate P) : Option (Gate P) :=
  if gt.pending.isEmpty || gt.moves.isEmpty then none else some gt

def published (gates : List (Gate P)) (p : Slot P) (v : Int) : List (Gate P) :=
  gates.filterMap fun gt =>
    (if (gt.vers p).any (fun wanted => v >= wanted) then
      {gt with pending := exclude gt.pending [p]}
    else gt).retained

def discarded (gates : List (Gate P)) (p : Slot P) : List (Gate P) :=
  gates.filterMap fun gt =>
    if gt.pending.contains p then
      let cancelled := gt.moves.filter fun m => m.dst == p
      {gt with
        moves := gt.moves.filter (fun m => m.dst != p)
        pending := exclude gt.pending (p :: cancelled.map Move.src)}.retained
    else gt.retained

structure State (P : Nat) where
  core : Storage31.State P
  claimed : List (Slot P)
  gates : List (Gate P)
  obl : List (Storage32.Obl P)
  renamed : List (Slot P × Slot P)
  orderOk : Bool

def State.fromCore (x : Storage35.State P) (gates : List (Gate P)) : State P :=
  ⟨Storage32.project (Storage35.project x), x.s.claimed, gates,
    x.g.obl, x.g.renamed, x.g.orderOk⟩

/-- Supply the old executor with its existing partner lookup to reuse the
compound Discard calculation. Stage 6 owns and updates the actual gates. -/
def State.legacyView (x : State P) : Storage35.State P :=
  let pairs := x.gates.flatMap fun gt =>
    gt.moves.map fun m =>
      (⟨m.dst, m.src, fun p => (gt.vers p).getD noneBytes, gt.pending⟩ : Storage32.Gate P)
  ⟨⟨⟨x.core.s, pairs⟩, x.claimed⟩, ⟨x.core.g, x.obl, x.renamed, x.orderOk⟩⟩

def State.admissionView (x : State P) : Storage35.State P :=
  {x.legacyView with s := {x.legacyView.s with gates := []}}

def initWith (c : Config P) (disk : Slot P → Int) : State P :=
  State.fromCore (Storage35.initWith c disk) []

def InitialValid (c : Config P) (disk : Slot P → Int) : Prop :=
  (∀ p, RealByte c (disk p)) ∧
    absent ∉ c.texts ∧ noneBytes ∉ c.texts ∧ unknown ∉ c.texts

def version (x : State P) (p : Slot P) : Int := x.core.g.vc + 1 + p.val

def install (x : State P) (moves : List (Move P)) (keys : List (Slot P)) : List (Gate P) :=
  if moves.isEmpty then x.gates else
    x.gates ++ [⟨moves, fun p => if keys.contains p then some (version x p) else none, keys⟩]

/-- The fourth field identifies a designation by its receiver. Source edges
belong to the blocked source's pair. Other edges belong to the prerequisite
receiver's pair. This gives the existing obligation retirement its meaning. -/
def obligations (x : State P) (moves : List (Move P)) (keys : List (Slot P)) :
    List (Storage32.Obl P) :=
  let others := exclude keys (endpoints moves)
  union
    (union
      (others.flatMap fun r => moves.map fun m => (r, m.dst, version x m.dst, m.dst))
      (moves.flatMap fun m => others.map fun r => (m.src, r, version x r, m.dst)))
    (moves.flatMap fun m => moves.map fun n => (m.src, n.dst, version x n.dst, m.dst))

inductive Action (P : Nat) where
  | prior (a : Storage32.Action P)
  | opApply (b : Batch P)
  | extClaim (p : Slot P) (present : Bool)
  | malformedApply (eff plan : WriteMap P) (p : Slot P) (receiverOnly : Bool)

def enabledPrior (c : Config P) (x : State P) (a : Storage32.Action P) : Bool :=
  let gated := fun p => x.gates.any fun gt => gt.gated p
  Storage31.enabled c x.core a &&
    match a with
    | .old (.flush p) | .flushDel p => !gated p
    | .opRename src dst refs _ => !touches x.gates (union refs [src, dst])
    | .opDelete p => !touches x.gates [p]
    | .old (.deliverUp _) => match x.core.s.up with
      | m :: _ => m.kind != "discard" ||
          !(Storage32.partner x.legacyView.s.to32 m.p).any (locked x.core.s.toSys)
      | [] => true
    | _ => true

def enabledApply (c : Config P) (x : State P) (b : Batch P) : Bool :=
  decide b.Shape && !touches x.gates b.keys &&
    Storage34.enabledApply c x.admissionView b.eff b.plan none none

def enabled (c : Config P) (x : State P) : Action P → Bool
  | .prior a => enabledPrior c x a
  | .opApply b => enabledApply c x b
  | .extClaim p present => x.claimed.contains p != present
  | .malformedApply .. => false

def priorGates (x : State P) : Storage32.Action P → List (Gate P)
  | .old (.dirSync ok) =>
    if ok then published x.gates x.core.s.job.p x.core.s.job.ver else x.gates
  | .old (.deliverUp ok) =>
    match x.core.s.up with
    | m :: _ =>
      if m.kind == "discard" && ok && (x.core.s.pages m.p).held then discarded x.gates m.p
      else x.gates
    | [] => x.gates
  | .old .crash | .old (.power _) | .old .switchFin => []
  | .opRename src dst refs _ =>
    if Storage3.opCur x.core.s src != absent then
      install x [⟨dst, src⟩] (union [src, dst] refs)
    else x.gates
  | _ => x.gates

def opApply (c : Config P) (x : State P) (b : Batch P) : State P :=
  let y := Storage35.opApply c x.legacyView b.eff b.plan none none
  let renamed := (b.moves.filter fun m => x.core.s.disk m.src != absent).map fun m => (m.src, m.dst)
  {State.fromCore y (install x b.moves b.keys) with
    obl := union x.obl (obligations x b.moves b.keys),
    renamed := union (Storage32.unrename x.renamed b.keys) renamed}

def execute (c : Config P) (x : State P) : Action P → State P
  | .prior a => State.fromCore (Storage35.execute c x.legacyView (.prior a)) (priorGates x a)
  | .opApply b => opApply c x b
  | .extClaim p present => State.fromCore (Storage35.extClaim c x.legacyView p present) x.gates
  | .malformedApply .. => x

def step (c : Config P) (x : State P) (a : Action P) : Option (State P) :=
  if enabled c x a then some (execute c x a) else none

inductive Reachable (c : Config P) (disk : Slot P → Int) : State P → Prop where
  | initial : InitialValid c disk → Reachable c disk (initWith c disk)
  | advance {x y} : Reachable c disk x → (a : Action P) → step c x a = some y → Reachable c disk y

def guarantee (x : State P) : Bool := Storage31.guarantee x.core
def orderHolds (x : State P) : Bool := x.orderOk
def renameResolves (x : State P) : Bool :=
  x.renamed.all fun r => x.core.s.disk r.1 != absent || x.core.s.disk r.2 != absent

end Stage6
