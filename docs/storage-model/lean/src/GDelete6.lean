import GCommit6
namespace Storage3
open Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

def deletePage6 (x : State P) (p : Fin P) : Page := {opEdit x.s p absent (x.g.vc+1) with risk := true}
def deleteSys6 (x : State P) (p : Fin P) : Sys P :=
  {x.s with pages := put x.s.pages p (deletePage6 x p), drafts := put x.s.drafts p (draftEntry (deletePage6 x p))}
def deleteGhost6 (x : State P) (p : Fin P) : Ghost P :=
  {x.g with
    vc := x.g.vc+1
    mine := [p]
    opRead := put x.g.opRead p (union (x.g.opRead p) [if (x.s.pages p).held then (x.s.pages p).base else x.s.disk p])
    wrote := put x.g.wrote p (union (x.g.wrote p) [(absent,x.g.vc+1)])
    promise := put x.g.promise p (ack (x.g.promise p) absent (x.g.vc+1) false 0)}

theorem delete_eq6 (c : Config P) (x : State P) (p : Fin P) (mutant : c.mutant = "none") :
    opDelete c x p = commit c x (deleteSys6 x p) (deleteGhost6 x p) "opDelete" := by
  simp [opDelete,deleteSys6,deleteGhost6,deletePage6,mutant]

theorem delete_auth_mono6 (x : State P) (p : Fin P) :
    AuthMono (shadow6 x).g (shadowGhost6 (deleteGhost6 x p)) := by
  apply shadow_auth_mono6
  · intro _ _ h; exact h
  · intro q w hw; by_cases eq : q = p
    · subst q; simp [deleteGhost6,put,mem_union_general,hw]
    · simpa [deleteGhost6,put,eq] using hw
  · intro q b hb; by_cases eq : q = p
    · subst q; simp [deleteGhost6,put,mem_union_general,hb]
    · simpa [deleteGhost6,put,eq] using hb

theorem delete_base_auth6 (x : State P) (p : Fin P) :
    Auth (shadowGhost6 (deleteGhost6 x p)) p (deletePage6 x p).base := by
  right; refine ⟨0,?_⟩
  apply (mem_union_general _ _ _).mpr; right
  apply List.mem_map.mpr
  refine ⟨(deletePage6 x p).base,?_,rfl⟩
  simp [deleteGhost6,deletePage6,put,mem_union_general,opEdit]
  split <;> simp_all

theorem G_delete6 (c : Config P) (x : State P) (p : Fin P)
    (mutant : c.mutant = "none") (h : GInv (shadow6 x)) :
    GInv (shadow6 (opDelete c x p)) := by
  rw [delete_eq6 c x p mutant]
  apply shadow_Gcommit6
  rcases h with ⟨_,hp,hd,hj,hw,hc⟩
  have mono := delete_auth_mono6 x p
  have base : Admissible (shadowGhost6 (deleteGhost6 x p)) p
      (deletePage6 x p).buf (deletePage6 x p).base := Or.inr (Or.inr (delete_base_auth6 x p))
  refine ⟨by simp [shadowGhost6],?_,?_,?_,?_,?_⟩
  · intro q hh; by_cases eq : q = p
    · subst q; simpa only [deleteSys6,put,ite_true] using base
    · simpa only [deleteSys6,put,eq,ite_false,shadow6] using admissible_mono mono (hp q (by simpa [deleteSys6,put,eq,shadow6] using hh))
  · intro q hh; by_cases eq : q = p
    · subst q
      simpa only [deleteSys6,put,ite_true] using admissible_draftEntry _ _ _ (fun _ => base) (by simpa [deleteSys6,put] using hh)
    · simpa only [deleteSys6,put,eq,ite_false,shadow6] using admissible_mono mono (hd q (by simpa [deleteSys6,put,eq,shadow6] using hh))
  · intro on; exact admissible_mono mono (hj on)
  · intro on phase; exact mono _ _ (hw on phase)
  · exact hc

theorem anchors_delete6 (c : Config P) (x : State P) (p : Fin P)
    (mutant : c.mutant = "none") (h : Anchors (shadow6 x))
    (bounds : VersionBounds x.s.toSys x.g.vc) : Anchors (shadow6 (opDelete c x p)) := by
  rw [delete_eq6 c x p mutant]
  apply shadow_anchors_commit6
  apply anchors_transport (shadow6 x) _ _ h bounds
  · intro q hh; by_cases eq : q = p
    · subst q; right; simp [deleteSys6,put,deletePage6,opEdit,shadow6]; omega
    · left; simpa [deleteSys6,put,eq,shadow6] using (show (x.s.pages q).held = true ∧
        (x.s.pages q).ver = (x.s.pages q).ver ∧ (x.s.pages q).buf = (x.s.pages q).buf from ⟨by simpa [deleteSys6,put,eq] using hh,rfl,rfl⟩)
  · rfl
  · rfl
  · intro m hm; exact hm
  · exact delete_auth_mono6 x p

theorem resolution_delete6 (c : Config P) (x : State P) (p : Fin P)
    (mutant : c.mutant = "none") (h : ResolutionAuth (shadow6 x)) :
    ResolutionAuth (shadow6 (opDelete c x p)) := by
  rw [delete_eq6 c x p mutant]
  apply shadow_resolution_commit6
  intro m mem ro; exact delete_auth_mono6 x p _ _ (h m mem ro)
end Storage3
