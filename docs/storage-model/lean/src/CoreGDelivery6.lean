import CoreAnchors6
import CoreResolution6
namespace Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

theorem G_upSubmit_heldAuth (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (s : Sys P) (m : Up P) (hx : GInv ⟨s,x.g⟩)
    (current : (s.pages m.p).held = true → m.bv = (s.pages m.p).ver → Auth x.g m.p (s.pages m.p).buf)
    (resolve : m.ro ≠ noneBytes → Auth x.g m.p m.ro) : GInv (upSubmit c x s m) := by
  by_cases hp : (s.pages m.p).held = true
  · exact G_upSubmit c mutant x s m hx (current hp) resolve
  · have hn : (!(s.pages m.p).held) = true := by simpa using hp
    simp only [upSubmit,hn,ite_true]
    apply G_commit
    rcases hx with ⟨hb,hp,hd,hj,hw,hc⟩
    exact ⟨by simpa [GTag] using hb,hp,hd,hj,hw,hc⟩

theorem G_upOp_heldAuth (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (s : Sys P) (m : Up P) (ok : Bool) (hx : GInv ⟨s,x.g⟩)
    (src : (s.pages m.p).held = true → m.bv = (s.pages m.p).ver → Auth x.g m.p (s.pages m.p).buf)
    (dst : (s.pages m.q).held = true → m.bv2 = (s.pages m.q).ver → Auth x.g m.q (s.pages m.q).buf) :
    GInv (upOp c x s m ok) := by
  by_cases hh : (s.pages m.p).held = true ∧ (s.pages m.q).held = true
  · exact G_upOp c mutant x s m ok hx (src hh.1) (dst hh.2)
  · have hn : (!(s.pages m.p).held || !(s.pages m.q).held) = true := by cases hp : (s.pages m.p).held <;> cases hq : (s.pages m.q).held <;> simp_all
    simp only [upOp,hn,ite_true]
    apply G_commit
    rcases hx with ⟨hb,hp,hd,hj,hw,hc⟩
    exact ⟨by simpa [GTag] using hb,hp,hd,hj,hw,hc⟩

theorem G_deliverUp_of_queueAnchors (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (m : Up P) (tail : List (Up P)) (ok : Bool) (hx : GInv x)
    (hq : x.s.up = m::tail) (ha : QueueAnchors x) (hr : ResolutionAuth x)
    (hb : ByteDomain c x.s) : GInv (deliverUp c x m tail ok) := by
  have mem : m ∈ x.s.up := by simp [hq]
  have hm := ha m mem
  have hrest : GInv ⟨{x.s with up := tail},x.g⟩ := hx
  simp only [deliverUp]; split
  · exact G_upOpen c x _ m ok hrest
  · split
    · exact G_upClose c x _ m hrest
    · split
      · exact G_upDiscard c x _ m ok hrest
      · split
        · have hop : m.kind = "op" := by simpa using (by assumption : (m.kind == "op") = true)
          exact G_upOp_heldAuth c mutant x _ m ok hrest (hm.1 (Or.inr hop)) (hm.2 hop)
        · have hkind : m.kind = "submit" := by
            have kinds := (hb.up m mem).1
            simp only [beq_iff_eq] at *
            simp_all
          exact G_upSubmit_heldAuth c mutant x _ m hrest (hm.1 (Or.inl hkind)) (hr m mem)


end Storage21
