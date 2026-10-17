import Storage32
import CoreAccepted21
namespace Storage32
open Storage21
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false
-- A structural invariant. The active-job clause is needed because Published
-- removes a page from a gate without deleting obligations whose save-page is it.
def Backed (sys : Sys P) (o : Obl P) : Prop :=
  ∃ gt ∈ sys.gates, o.1 ∈ gt.pending ∧ o.2.1 ∈ gt.pending ∧
    o.1 ≠ o.2.1 ∧ o.1 ≠ gt.dst ∧ o.2.2.2 = gt.dst ∧
    gt.vers o.2.1 = o.2.2.1 ∧ gt.vers o.2.1 ≠ noneBytes ∧
    (o.1 = gt.src ∨ o.2.1 = gt.dst)
def OrderInv (x : State P) : Prop :=
  (∀ o ∈ x.g.obl, Backed x.s o) ∧
  (∀ gt ∈ x.s.gates, gt.src ∈ gt.pending → gt.dst ∈ gt.pending →
    (gt.src,gt.dst,gt.vers gt.dst,gt.dst) ∈ x.g.obl) ∧
  (x.s.job.on = true → ∀ o ∈ x.g.obl, o.1 ≠ x.s.job.p) ∧
  x.g.orderOk = true

theorem backed_gated9 (c : Config P) (sys : Sys P) (o : Obl P)
    (mutant : c.mutant = "none") (h : Backed sys o) : gated c sys o.1 = true := by
  obtain ⟨gt,hgt,ha,hb,hab,had,_,_,_,hshape⟩ := h
  apply List.any_eq_true.mpr
  refine ⟨gt,hgt,?_⟩
  have la : gt.pending.contains o.1 = true := by simpa using ha
  have lb : gt.pending.contains o.2.1 = true := by simpa using hb
  have ne : (o.1 != gt.dst) = true := by simpa using had
  have length : 1 < gt.pending.length := by
    by_cases hl : 1 < gt.pending.length
    · exact hl
    have le : gt.pending.length ≤ 1 := by omega
    cases hp : gt.pending with
    | nil => simp [hp] at ha
    | cons a as =>
      have len0 : as.length = 0 := by simp only [hp,List.length_cons] at le; omega
      have az : as = [] := List.length_eq_zero_iff.mp len0
      simp [hp,az] at ha hb
      exact False.elim (hab (ha.trans hb.symm))
  rcases hshape with hs | hd
  · have eqs : (o.1 == gt.src) = true := by simpa using hs
    simp [gates1,la,ne,mutant,eqs,length,ha]
  · have destmem : gt.dst ∈ gt.pending := hd ▸ hb
    have ld : gt.pending.contains gt.dst = true := by simpa [hd] using lb
    simp [gates1,la,ne,mutant,ld,ha,destmem]

theorem order_flush9 (c : Config P) (x : State P) (p : Fin P)
    (mutant : c.mutant = "none") (h : OrderInv x)
    (gate : gated c x.s p = false) : orderHolds (flush c x p) = true := by
  have no : x.g.obl.any (fun o => o.1 == p) = false := by
    apply List.any_eq_false.mpr
    intro o ho
    have hp : o.1 ≠ p := by
      intro he
      have yes := backed_gated9 c x.s o mutant (h.1 o ho)
      simp [he,gate] at yes
    simpa using hp
  simp [orderHolds,flush,lift,h.2.2.2,no]
#print axioms backed_gated9
#print axioms order_flush9
end Storage32
