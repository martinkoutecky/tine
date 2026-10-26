import Storage33
namespace Storage34
open Storage21
structure Sys (P : Nat) extends to32 : Storage32.Sys P where
  claimed : List (Fin P) := []
structure State (P : Nat) where
  s : Sys P
  g : Storage32.Ghost P
def project (x : State P) : Storage33.State P := ⟨x.s.to32,x.g⟩
def embed (x : Storage33.State P) : State P := ⟨⟨x.s,[]⟩,x.g⟩
def lift (x : State P) (y : Storage33.State P) : State P := ⟨⟨y.s,x.s.claimed⟩,y.g⟩
def initWith (c : Config P) (disk : Fin P → Int) : State P := embed (Storage33.initWith c disk)
def init (c : Config P) : State P := initWith c (Storage31.initDisk c)
abbrev Map := Storage33.Map
def claimedCreates (x : State P) (eff plan : Map P) : Bool :=
  (Storage33.keys eff).any fun p => Storage33.get plan p == absent &&
    Storage33.get eff p != absent && x.s.claimed.contains p
def enabledApply (c : Config P) (x : State P) (eff plan : Map P) (dst src : Option (Fin P)) : Bool :=
  Storage33.enabledApply c (project x) eff plan dst src && (!claimedCreates x eff plan || c.mutant == "MNS")
def opApply (c : Config P) (x : State P) (eff plan : Map P) (dst src : Option (Fin P)) : State P :=
  let y := Storage33.opApply c (project x) eff plan dst src
  ⟨⟨y.s,x.s.claimed⟩,{y.g with bad := union y.g.bad (tags (claimedCreates x eff plan) "name-slot-create-over-claim")}⟩
def extClaim (c : Config P) (x : State P) (p : Fin P) (present : Bool) : State P :=
  let y := Storage32.commit c (project x) x.s.to32 x.g "extClaim"
  ⟨⟨y.s,if present then union x.s.claimed [p] else exclude x.s.claimed [p]⟩,y.g⟩
inductive Action (P : Nat) where
  | prior (a : Storage32.Action P)
  | opApply (eff plan : Map P) (dst src : Option (Fin P))
  | extClaim (p : Fin P) (present : Bool)
def enabled (c : Config P) (x : State P) : Action P → Bool
  | .prior a => Storage32.enabled c (project x) a
  | .opApply eff plan dst src => enabledApply c x eff plan dst src
  | .extClaim p present => x.s.claimed.contains p != present
def execute (c : Config P) (x : State P) : Action P → State P
  | .prior a => lift x (Storage32.execute c (project x) a)
  | .opApply eff plan dst src => opApply c x eff plan dst src
  | .extClaim p present => extClaim c x p present
def step (c : Config P) (x : State P) (a : Action P) : Option (State P) :=
  if enabled c x a then some (execute c x a) else none
def guarantee (x : State P) : Bool := Storage33.guarantee (project x)
def orderHolds (x : State P) : Bool := Storage33.orderHolds (project x)
def renameResolves (x : State P) : Bool := Storage33.renameResolves (project x)
inductive Reachable (c : Config P) (disk : Fin P → Int) : State P → Prop where
  | initial : Reachable c disk (initWith c disk)
  | advance {x y} : Reachable c disk x → (a : Action P) → step c x a = some y → Reachable c disk y
end Storage34
