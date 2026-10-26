import Conservativity8
import Guarantee31
namespace Storage32
open Storage21
set_option linter.unusedSimpArgs false
set_option maxHeartbeats 4000000
def NoFullRename (x : Storage31.State P) : Action P → Prop
  | .opRename src _ _ _ => Storage3.opCur x.s src = absent
  | _ => True
theorem enabled_embed8 (c : Config P) (x : Storage31.State P) (a : Action P) :
    enabled c (embed x) a = Storage31.enabled c x a := by
  cases a with
  | old a => exact old_enabled_embed8 c x a
  | opRename src dst refs rt => simp [enabled,touches,embed,project]
  | opDelete p => simp [enabled,touches,embed,project]
  | flushDel p => simp [enabled,gated,embed,project]
  | load p => simp [enabled,embed,project]
theorem execute_embed_noFull8 (c : Config P) (x : Storage31.State P) (a : Action P)
    (mutant : c.mutant = "none") (h : NoFullRename x a) :
    execute c (embed x) a = embed (Storage31.execute c x a) := by
  cases a with
  | old a => exact old_execute_embed8 c x a mutant
  | opRename src dst refs rt =>
    simp only [NoFullRename] at h
    simp [execute,opRename,embed,project,lift,h,unrename,Storage31.execute,Storage31.baseline,Storage3.execute,union]
  | opDelete p => simp [execute,opDelete,embed,project,lift,unrename,Storage31.execute,Storage31.baseline,Storage3.execute]
  | flushDel p => simp [execute,flush,embed,project,lift,Storage31.execute,Storage31.baseline,Storage3.execute]
  | load p => rfl
inductive ReachableNoFull (c : Config P) (disk : Fin P → Int) : State P → Prop where
  | initial : ReachableNoFull c disk (initWith c disk)
  | advance {x y} : ReachableNoFull c disk x → (a : Action P) →
      NoFullRename (project x) a → step c x a = some y → ReachableNoFull c disk y
theorem noFull_actual_reachable8 (c : Config P) (disk : Fin P → Int) (x : State P)
    (hr : ReachableNoFull c disk x) : Reachable c disk x := by
  induction hr with
  | initial => exact .initial
  | advance hr a _ hs ih => exact .advance ih a hs
theorem noFull_refines31_8 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (h : ReachableNoFull c disk x) :
    x = embed (project x) ∧ Storage31.Reachable c disk (project x) := by
  induction h with
  | initial => exact ⟨rfl,Storage31.Reachable.initial⟩
  | @advance x y hr a nf hs ih =>
    rw [ih.1] at hs
    simp only [step,enabled_embed8] at hs
    split at hs
    · rw [execute_embed_noFull8 c _ a mutant nf] at hs
      cases hs
      exact ⟨rfl,Storage31.Reachable.advance ih.2 a (by simp [Storage31.step,‹Storage31.enabled c (project x) a = true›])⟩
    · contradiction
theorem noFull_guarantee8 (c : Config P) (T : Nat)
    (texts : c.texts = Storage3.positiveTexts4 T) (two : 2 ≤ T) (mutant : c.mutant = "none")
    (x : State P) (hr : ReachableNoFull c (Storage31.initDisk c) x) : guarantee x = true :=
  Storage31.guarantee_positiveTexts7 c T texts two mutant (project x) (noFull_refines31_8 c _ x mutant hr).2
end Storage32
namespace Storage33
open Storage21
set_option linter.unusedSimpArgs false
set_option maxHeartbeats 4000000
def EmptyOrders (x : State P) : Prop :=
  x.s.gates = [] ∧ x.g.obl = [] ∧ x.g.renamed = [] ∧ x.g.orderOk = true
theorem emptyOrders_iff_embed8 (x : State P) : EmptyOrders x ↔ x = Storage32.embed (Storage32.project x) := by
  rcases x with ⟨⟨s,gates⟩,⟨g,obl,renamed,orderOk⟩⟩
  simp [EmptyOrders,Storage32.embed,Storage32.project]
theorem emptyOrders_outcomes8 (x : State P) (h : EmptyOrders x) :
    orderHolds x = true ∧ renameResolves x = true := by
  simp [Storage32.orderHolds,Storage32.renameResolves,h.2.2.1,h.2.2.2]
def NoMove (x : State P) : Action P → Prop
  | .prior a => Storage32.NoFullRename (Storage32.project x) a
  | .opApply _ _ dst _ => dst = none
theorem apply_emptyOrders8 (c : Config P) (x : State P) (eff plan : Map P) (src : Option (Fin P))
    (h : EmptyOrders x) : EmptyOrders (opApply c x eff plan none src) := by
  simp [EmptyOrders,opApply,Storage32.commit,Storage32.unrename,h.1,h.2.1,h.2.2.1,h.2.2.2,union]
theorem noMove_emptyOrders_step8 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (h : EmptyOrders x) (nm : NoMove x a) :
    EmptyOrders (execute c x a) := by
  cases a with
  | prior a =>
    rw [(emptyOrders_iff_embed8 x).mp h]
    rw [execute,Storage32.execute_embed_noFull8 c _ a mutant nm]
    exact (emptyOrders_iff_embed8 _).mpr rfl
  | opApply eff plan dst src =>
    simp only [NoMove] at nm
    subst dst
    exact apply_emptyOrders8 c x eff plan src h
inductive ReachableNoMove (c : Config P) (disk : Fin P → Int) : State P → Prop where
  | initial : ReachableNoMove c disk (initWith c disk)
  | advance {x y} : ReachableNoMove c disk x → (a : Action P) →
      NoMove x a → step c x a = some y → ReachableNoMove c disk y
theorem noMove_actual_reachable8 (c : Config P) (disk : Fin P → Int) (x : State P)
    (hr : ReachableNoMove c disk x) : Reachable c disk x := by
  induction hr with
  | initial => exact .initial
  | advance hr a _ hs ih => exact .advance ih a hs
theorem noMove_order_resolves8 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : ReachableNoMove c disk x) :
    EmptyOrders x ∧ orderHolds x = true ∧ renameResolves x = true := by
  have empty : EmptyOrders x := by
    induction hr with
    | initial => exact ⟨rfl,rfl,rfl,rfl⟩
    | @advance x y hr a nm hs ih =>
      unfold step at hs
      split at hs
      · cases hs; exact noMove_emptyOrders_step8 c x a mutant ih nm
      · contradiction
  exact ⟨empty,emptyOrders_outcomes8 x empty⟩
#print axioms noMove_order_resolves8
#print axioms Storage32.noFull_guarantee8
end Storage33
