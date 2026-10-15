import Lean
import headline9
open Storage21
namespace Verifier9
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000

-- Check the literal predicate, without using the prover's property wrapper.
theorem ordering (c : Config P) (disk : Fin P → Int) (x : Storage34.State P)
    (hm : c.mutant = "none") (hr : Storage34.Reachable c disk x) :
    x.g.orderOk = true :=
  Storage34.orderHolds_reachable34_9 c disk x hm hr

theorem historical_slot (c : Config P) (disk : Fin P → Int) (x : Storage34.State P)
    (hm : c.mutant = "none") (hr : Storage34.Reachable c disk x) :
    "name-slot-create-over-claim" ∉ x.g.bad :=
  Storage34.name_slot_history9 c disk x hm hr

theorem literal_trash (c : Config P) (disk : Fin P → Int) (x : Storage34.State P)
    (hm : c.mutant = "none") (hr : Storage34.Reachable c disk x) :
    ∀ p : Fin P, ∀ b ∈ x.g.removed p, b ∈ x.s.trash p ∨ x.s.disk p = b := by
  have h := Storage34.trashed_reachable34_9 c disk x hm hr
  intro p b hb
  have hp := List.all_eq_true.mp h p (by simp [ids])
  have hpb := List.all_eq_true.mp hp b hb
  change ((x.s.trash p).contains b || (x.s.disk p == b)) = true at hpb
  simpa only [Bool.or_eq_true, List.contains_iff_mem, beq_iff_eq] using hpb

theorem literal_restricted_guarantee (c : Config P) (T : Nat)
    (ht : c.texts = Storage3.positiveTexts4 T) (hT : 2 ≤ T) (hm : c.mutant = "none")
    (x : Storage34.State P)
    (hr : Storage34.ReachableNoApplyRestore c (Storage31.initDisk c) x) :
    Storage21.noLoss (Storage3.project (Storage32.project (Storage34.project x))) = true ∧
    x.g.bad = [] ∧
    Storage21.accepted (Storage3.project (Storage32.project (Storage34.project x))) = true ∧
    Storage3.trashed (Storage32.project (Storage34.project x)) = true := by
  have h := Storage34.noApplyRestore_guarantee34_9 c T ht hT hm x hr
  change (Storage21.noLoss (Storage3.project (Storage32.project (Storage34.project x))) &&
    x.g.bad.isEmpty &&
    Storage21.accepted (Storage3.project (Storage32.project (Storage34.project x))) &&
    Storage3.trashed (Storage32.project (Storage34.project x))) = true at h
  simpa only [Bool.and_eq_true, List.isEmpty_iff, and_assoc] using h

theorem claimed_step_stutters (c : Config P) (disk : Fin P → Int)
    (x : Storage34.State P) (p : Fin P) (b : Bool)
    (hm : c.mutant = "none") (hr : Storage34.Reachable c disk x) :
    Storage34.project (Storage34.extClaim c x p b) = Storage34.project x :=
  Storage34.extClaim_projects_idle9 c x p b (Storage34.idle_closed_reachable34_9 c disk x hm hr)

-- Non-vacuity before the first publication: actual restricted reachability,
-- nonempty obligations, both kinds of blocked save, and an enabled destination.
def beforePublish : Storage32.State 3 :=
  Witness9.run32 (Storage32.init Witness9.c) (Witness9.actions.take 4)

theorem move_installed :
    Storage32.ReachableNoRestore Witness9.c (Storage31.initDisk Witness9.c) beforePublish ∧
    beforePublish.g.obl ≠ [] ∧
    Storage32.enabled Witness9.c beforePublish (.old (.flush Witness9.p2)) = true ∧
    Storage32.enabled Witness9.c beforePublish (.old (.flush Witness9.p1)) = false ∧
    Storage32.enabled Witness9.c beforePublish (.flushDel Witness9.p0) = false := by
  constructor
  · apply Witness9.legal_reachable9 _ (Witness9.actions.take 4) .initial
    simp only [Witness9.actions, List.take, Witness9.Legal, Storage32.NoRestore]
    decide
  · decide

-- A second witness exercises an admitted claim, then refusal before creation.
def c : Config 3 := Witness9.c
def initial : Storage34.State 3 := Storage34.init c
def loaded : Storage34.State 3 := Storage34.execute c initial (.prior (.load Witness9.p2))
def claimed : Storage34.State 3 := Storage34.execute c loaded (.extClaim Witness9.p2 true)
def creation : Storage34.Map 3 := fun p => if p = Witness9.p2 then some 3 else none
def planned : Storage34.Map 3 := fun p => some (Storage3.opCur claimed.s.to32.toBase p)

theorem successful_step (cfg : Config P) (x : Storage34.State P) (a : Storage34.Action P)
    (h : Storage34.enabled cfg x a = true) :
    Storage34.step cfg x a = some (Storage34.execute cfg x a) := by
  simp [Storage34.step, h]

theorem claim_refusal :
    Storage34.Reachable c (Storage31.initDisk c) claimed ∧
    Storage34.step c claimed (.opApply creation planned none none) = none ∧
    claimed.s.claimed = [Witness9.p2] ∧
    (claimed.s.drafts Witness9.p2).bytes = noneBytes ∧
    (claimed.g.promise Witness9.p2).on = false := by
  constructor
  · have hloaded : Storage34.Reachable c (Storage31.initDisk c) loaded :=
      .advance .initial (.prior (.load Witness9.p2)) (successful_step c initial _ (by decide))
    exact .advance hloaded (.extClaim Witness9.p2 true) (successful_step c loaded _ (by decide))
  · decide

#print axioms ordering
#print axioms historical_slot
#print axioms literal_trash
#print axioms literal_restricted_guarantee
#print axioms claimed_step_stutters
#print axioms move_installed
#print axioms claim_refusal
end Verifier9

-- Independent direct type and axiom prints for every claimed headline.
set_option pp.universes true
