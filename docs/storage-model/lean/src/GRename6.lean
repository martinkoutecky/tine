import GDelete6
namespace Storage3
open Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

def renameRead6 (x : State P) (src dst : Fin P) (refs : List (Fin P)) : Fin P → List Int :=
  fun p => union (x.g.opRead p)
    (if p == dst && opCur x.s src != absent then [opCur x.s dst].filter fun t => t == absent
     else if (p == src && opCur x.s src != absent) || refs.contains p then
       [if (x.s.pages p).held then (x.s.pages p).base else x.s.disk p] else [])
def renameSys6 (x : State P) (src dst : Fin P) (refs : List (Fin P)) (rt : Fin P → Int) : Sys P :=
  {x.s with pages := renamePages4 x src dst refs rt, drafts := renameDrafts6 x src dst refs rt}
def renameGhost6 (x : State P) (src dst : Fin P) (refs : List (Fin P)) (rt : Fin P → Int) : Ghost P :=
  let full := opCur x.s src != absent
  let named := if full then [src,dst] else []
  {x.g with
    vc := x.g.vc+P
    mine := union named (refs.filter fun r => opClean x.s r)
    opRead := renameRead6 x src dst refs
    wrote := (renameGot4 x src dst refs rt).foldl (fun acc e => put acc e.1 (union (acc e.1) [(e.2,x.g.vc+1+e.1.val)])) x.g.wrote
    promise := renamePromise4 x src dst refs rt
    bad := union (union x.g.bad (tags (!(union named refs).all (opClean x.s)) "D4-unclean-page-changed"))
      (tags (full && opCur x.s dst != absent) "rename-clobbered-target")}

theorem opRename_eq6 (c : Config P) (x : State P) (src dst : Fin P)
    (refs : List (Fin P)) (rt : Fin P → Int) (mutant : c.mutant = "none") :
    opRename c x src dst refs rt = commit c x (renameSys6 x src dst refs rt)
      (renameGhost6 x src dst refs rt) (if opCur x.s src != absent then "opRename" else "opRenameRefs") := by
  unfold opRename renameSys6 renameGhost6 renameRead6
  by_cases full : opCur x.s src = absent <;>
    simp [renamePages4,renameDrafts6,renameGot4,renamePromise4,mutant,full]

theorem rename_auth_mono6 (x : State P) (src dst : Fin P) (refs : List (Fin P)) (rt : Fin P → Int) :
    AuthMono (shadow6 x).g (shadowGhost6 (renameGhost6 x src dst refs rt)) := by
  apply shadow_auth_mono6
  · intro _ _ h; exact h
  · exact fold_wrote_mono6 (renameGot4 x src dst refs rt) x.g.wrote (fun p => x.g.vc+1+p.val)
  · intro p b hb; simp [renameGhost6,renameRead6,mem_union_general,hb]

theorem rename_base_read6 (c : Config P) (x : State P) (src dst : Fin P)
    (refs : List (Fin P)) (rt : Fin P → Int) (mutant : c.mutant = "none")
    (hen : enabled c x (.opRename src dst refs rt) = true)
    (p : Fin P) (mem : p ∈ (renameGot4 x src dst refs rt).map Prod.fst) :
    (renamePages4 x src dst refs rt p).base ∈ renameRead6 x src dst refs p := by
  have ne := rename_refs4 c x src dst refs rt hen
  have dstclean : opCur x.s src ≠ absent → opCur x.s dst = absent ∧ opClean x.s dst = true := by
    simp [enabled,mutant] at hen
    grind
  have selected := (rename_selected6 x src dst refs rt p).mp mem
  have srcnot : src ∉ refs := by intro h; exact (ne.2 src h).1 rfl
  have dstnot : dst ∉ refs := by intro h; exact (ne.2 dst h).2 rfl
  by_cases ref : p ∈ refs
  · have ps := (ne.2 p ref).1
    have pd := (ne.2 p ref).2
    simp [renamePages4,fold_put_if,ref,renameRead6,pd,ps,mem_union_general,opEdit]
    split <;> simp_all
  · rcases selected with hr | ⟨full,ps | pd⟩
    · contradiction
    · subst p
      simp [renamePages4,fold_put_if,srcnot,full,put,opEdit,renameRead6,ne.1,mem_union_general]
      split <;> simp_all
    · subst p
      have free := dstclean full
      have baseabs : (opEdit x.s dst (opCur x.s src) (x.g.vc+1+dst.val)).base = absent := by
        unfold opEdit
        by_cases held : (x.s.pages dst).held = true
        · have cl : clean (x.s.pages dst) = true := by simpa [opClean,held] using free.2
          have eq : (x.s.pages dst).buf = (x.s.pages dst).base := by simp [clean] at cl; grind
          have buf : (x.s.pages dst).buf = absent := by simpa [opCur,held] using free.1
          simpa [held] using eq.symm.trans buf
        · have disk : x.s.disk dst = absent := by simpa [opCur,held] using free.1
          simp [held,disk]
      simp [renamePages4,fold_put_if,dstnot,full,put,Ne.symm ne.1,baseabs,
        renameRead6,free.1,mem_union_general]

theorem G_rename_op6 (c : Config P) (x : State P) (src dst : Fin P)
    (refs : List (Fin P)) (rt : Fin P → Int) (mutant : c.mutant = "none")
    (h : GInv (shadow6 x)) (hen : enabled c x (.opRename src dst refs rt) = true) :
    GInv (shadow6 (opRename c x src dst refs rt)) := by
  rw [opRename_eq6 c x src dst refs rt mutant]
  apply shadow_Gcommit6
  rcases h with ⟨_,hp,hd,hj,hw,hc⟩
  have mono := rename_auth_mono6 x src dst refs rt
  have base : ∀ p, p ∈ (renameGot4 x src dst refs rt).map Prod.fst →
      Admissible (shadowGhost6 (renameGhost6 x src dst refs rt)) p
        (renamePages4 x src dst refs rt p).buf (renamePages4 x src dst refs rt p).base := by
    intro p mem; right; right; right
    refine ⟨0,?_⟩
    apply (mem_union_general _ _ _).mpr; right
    exact List.mem_map.mpr ⟨_,rename_base_read6 c x src dst refs rt mutant hen p mem,rfl⟩
  refine ⟨by simp [shadowGhost6],?_,?_,?_,?_,?_⟩
  · intro p hh; by_cases mem : p ∈ (renameGot4 x src dst refs rt).map Prod.fst
    · exact base p mem
    · have pg := rename_not_got4 x src dst refs rt p mem
      simpa only [renameSys6,pg,shadow6] using admissible_mono mono (hp p (by simpa [renameSys6,shadow6,pg] using hh))
  · intro p hh; by_cases mem : p ∈ (renameGot4 x src dst refs rt).map Prod.fst
    · have dr : renameDrafts6 x src dst refs rt p = draftEntry (renamePages4 x src dst refs rt p) := by
        by_cases ref : p ∈ refs <;> by_cases full : opCur x.s src = absent <;>
          by_cases ps : p = src <;> by_cases pd : p = dst
        all_goals simp_all [renameDrafts6,renamePages4,fold_put_if,put,rename_selected6]
      simpa only [renameSys6,dr] using admissible_draftEntry _ _ _ (fun _ => base p mem) (by simpa [renameSys6,dr] using hh)
    · have dr := rename_draft_untouched6 x src dst refs rt p mem
      simpa only [renameSys6,dr,shadow6] using admissible_mono mono (hd p (by simpa [renameSys6,shadow6,dr] using hh))
  · intro on; exact admissible_mono mono (hj on)
  · intro on phase; exact mono _ _ (hw on phase)
  · exact hc

theorem anchors_rename6 (c : Config P) (x : State P) (src dst : Fin P)
    (refs : List (Fin P)) (rt : Fin P → Int) (mutant : c.mutant = "none")
    (h : Anchors (shadow6 x)) (bounds : VersionBounds x.s.toSys x.g.vc)
    (hen : enabled c x (.opRename src dst refs rt) = true) :
    Anchors (shadow6 (opRename c x src dst refs rt)) := by
  rw [opRename_eq6 c x src dst refs rt mutant]
  apply shadow_anchors_commit6
  apply anchors_transport (shadow6 x) _ _ h bounds
  · intro p hh; by_cases mem : p ∈ (renameGot4 x src dst refs rt).map Prod.fst
    · obtain ⟨e,he,ep⟩ := List.mem_map.mp mem
      have ver : (renamePages4 x src dst refs rt p).ver = x.g.vc+1+p.val := by simpa [ep] using (rename_got_shape4 c x src dst refs rt hen e he).2.1
      right; change x.g.vc < (renamePages4 x src dst refs rt p).ver; rw [ver]; omega
    · left; have pg := rename_not_got4 x src dst refs rt p mem
      exact ⟨by simpa [renameSys6,shadow6,pg] using hh,by simp [renameSys6,shadow6,pg],by simp [renameSys6,shadow6,pg]⟩
  · rfl
  · rfl
  · intro m hm; exact hm
  · exact rename_auth_mono6 x src dst refs rt

theorem resolution_rename6 (c : Config P) (x : State P) (src dst : Fin P)
    (refs : List (Fin P)) (rt : Fin P → Int) (mutant : c.mutant = "none")
    (h : ResolutionAuth (shadow6 x)) : ResolutionAuth (shadow6 (opRename c x src dst refs rt)) := by
  rw [opRename_eq6 c x src dst refs rt mutant]
  apply shadow_resolution_commit6
  intro m mem ro; exact rename_auth_mono6 x src dst refs rt _ _ (h m mem ro)
end Storage3
