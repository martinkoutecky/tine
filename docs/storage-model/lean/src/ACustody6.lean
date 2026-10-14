import ARecovery6
namespace Storage3
open Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

abbrev renameDrafts6 (x : State P) (src dst : Fin P) (refs : List (Fin P)) (rt : Fin P → Int) :=
  refs.foldl (fun acc r => put acc r (draftEntry {opEdit x.s r (rt r) (x.g.vc+1+r.val) with risk := true}))
    (if opCur x.s src == absent then x.s.drafts else
      put (put x.s.drafts dst (draftEntry {opEdit x.s dst (opCur x.s src) (x.g.vc+1+dst.val) with risk := true}))
        src (draftEntry {opEdit x.s src absent (x.g.vc+1+src.val) with risk := true}))

theorem rename_selected6 (x : State P) (src dst : Fin P) (refs : List (Fin P)) (rt : Fin P → Int)
    (p : Fin P) : p ∈ (renameGot4 x src dst refs rt).map Prod.fst ↔
    p ∈ refs ∨ opCur x.s src ≠ absent ∧ (p = src ∨ p = dst) := by
  simp [renameGot4,mem_union_general,List.mem_map]
  grind

theorem rename_draft_selected6 (x : State P) (src dst : Fin P) (refs : List (Fin P)) (rt : Fin P → Int)
    (p : Fin P) (mem : p ∈ (renameGot4 x src dst refs rt).map Prod.fst) :
    (renameDrafts6 x src dst refs rt p).bytes = (renamePages4 x src dst refs rt p).buf ∧
    (renamePages4 x src dst refs rt p).held = true ∧ (renamePages4 x src dst refs rt p).risk = true := by
  have selected := (rename_selected6 x src dst refs rt p).mp mem
  by_cases ref : p ∈ refs <;> by_cases full : opCur x.s src = absent <;>
    by_cases ps : p = src <;> by_cases pd : p = dst
  all_goals cases hh : (x.s.pages p).held <;> simp_all [renameDrafts6,renamePages4,fold_put_if,put,draftEntry,opEdit]

theorem rename_draft_untouched6 (x : State P) (src dst : Fin P) (refs : List (Fin P)) (rt : Fin P → Int)
    (p : Fin P) (mem : p ∉ (renameGot4 x src dst refs rt).map Prod.fst) :
    renameDrafts6 x src dst refs rt p = x.s.drafts p := by
  have selected := fun hs => mem ((rename_selected6 x src dst refs rt p).mpr hs)
  by_cases ref : p ∈ refs <;> by_cases full : opCur x.s src = absent <;>
    by_cases ps : p = src <;> by_cases pd : p = dst
  all_goals simp_all [renameDrafts6,fold_put_if,put]

theorem fold_wrote_mono6 (es : List (Fin P × Int)) (wr : Fin P → List (Int × Int))
    (v : Fin P → Int) : ∀ p w, w ∈ wr p →
    w ∈ es.foldl (fun acc e => put acc e.1 (union (acc e.1) [(e.2,v e.1)])) wr p := by
  induction es generalizing wr with
  | nil => intro p w hw; exact hw
  | cons e tail ih =>
    intro p w hw; apply ih
    by_cases eq : p = e.1
    · subst p; simp [put,mem_union_general,hw]
    · simpa [put,eq] using hw

theorem custody_delete6 (c : Config P) (x : State P) (p : Fin P)
    (mutant : c.mutant = "none") (h : Custody c (project x))
    (lineage : PromiseLineage (project x)) : Custody c (project (opDelete c x p)) := by
  have old := lineage.1 p
  have fresh : (x.g.promise p).ver < x.g.vc+1 := by change (x.g.promise p).ver ≤ x.g.vc at old; omega
  have ack := ack_newer (x.g.promise p) absent (x.g.vc+1) false 0 fresh
  simp only [opDelete,commit,project,mutant]
  apply custody_commit
  · simp only [show ("none" == "MDD") = false from rfl,Bool.false_eq_true,ite_false,ack]
    intro q need
    by_cases eq : q = p
    · subst q; cases hh : (x.s.pages p).held <;> simp [put,draftEntry,opEdit,hh]
    · have oldneed : NeedsDraft c (project x) q := by
        simpa only [NeedsDraft,LivePromise,Covers,put,eq,ite_false,project] using need
      have old := h q oldneed
      simpa only [put,eq,ite_false,project] using old
  · apply postPromises_noCleaning
    intro q hh hd
    by_cases eq : q = p
    · subst q; simp [put,opEdit,dirty,clean]
    · simpa [put,eq] using hd

theorem custody_rename_pre6 (c : Config P) (x : State P) (src dst : Fin P)
    (refs : List (Fin P)) (rt : Fin P → Int) (h : Custody c (project x))
    (lineage : PromiseLineage (project x)) (hen : enabled c x (.opRename src dst refs rt) = true) :
    Custody c ⟨{x.s.toSys with
        pages := renamePages4 x src dst refs rt
        drafts := renameDrafts6 x src dst refs rt},
      {x.g.toGhost with
        promise := renamePromise4 x src dst refs rt
        wrote := (renameGot4 x src dst refs rt).foldl (fun acc e =>
          put acc e.1 (union (acc e.1) [(e.2,x.g.vc+1+e.1.val)])) x.g.wrote}⟩ := by
  intro p need
  by_cases mem : p ∈ (renameGot4 x src dst refs rt).map Prod.fst
  · have draft := rename_draft_selected6 x src dst refs rt p mem
    have pr := rename_promise_match6 c x src dst refs rt lineage hen p mem
    exact ⟨by simp [pr,draft.1],by intro _; exact draft.2⟩
  · have pg := rename_not_got4 x src dst refs rt p mem
    have pr := fold_ack_unchanged6 (renameGot4 x src dst refs rt) x.g.promise
      (fun p => x.g.vc+1+p.val) p mem
    change renamePromise4 x src dst refs rt p = x.g.promise p at pr
    have dr := rename_draft_untouched6 x src dst refs rt p mem
    have oldneed : NeedsDraft c (project x) p := by
      refine ⟨by simpa [LivePromise,pr,project] using need.1,?_⟩
      rcases need.2 with unsigned | lost | stable
      · left; simpa [pr,project] using unsigned
      · right; left
        intro cov; apply lost
        exact covers_mono (project x) _ p _ pr (fold_wrote_mono6 (renameGot4 x src dst refs rt) x.g.wrote (fun p => x.g.vc+1+p.val) p) cov
      · right; right; refine ⟨stable.1,?_⟩
        intro cov; apply stable.2
        exact covers_mono (project x) _ p _ pr (fold_wrote_mono6 (renameGot4 x src dst refs rt) x.g.wrote (fun p => x.g.vc+1+p.val) p) cov
    have old := h p oldneed
    exact ⟨by simpa [dr,pr,project] using old.1,by simpa [pg,project] using old.2⟩

theorem custody_opRename6 (c : Config P) (x : State P) (src dst : Fin P)
    (refs : List (Fin P)) (rt : Fin P → Int) (mutant : c.mutant = "none")
    (h : Custody c (project x)) (lineage : PromiseLineage (project x))
    (hen : enabled c x (.opRename src dst refs rt) = true) :
    Custody c (project (opRename c x src dst refs rt)) := by
  simp only [opRename,commit,project,mutant]
  apply custody_commit
  · by_cases full : opCur x.s src = absent
    all_goals simpa [full,Custody,NeedsDraft,LivePromise,Covers,renamePages4,renameDrafts6,renamePromise4,renameGot4,
      show ("none" == "MRN1") = false from rfl,show ("none" == "MRN4") = false from rfl,
      show ("none" == "MRN5") = false from rfl,show ("none" != "MRN5") = true from rfl,
      Bool.false_eq_true,ite_false,Bool.and_true,bne_iff_ne,beq_iff_eq,ite_not] using custody_rename_pre6 c x src dst refs rt h lineage hen
  · apply postPromises_noCleaning
    intro p hh hd
    by_cases mem : p ∈ (renameGot4 x src dst refs rt).map Prod.fst
    · have typed : (renamePages4 x src dst refs rt p).typed = true := by
        obtain ⟨e,he,ep⟩ := List.mem_map.mp mem
        simpa [ep] using (rename_got_shape4 c x src dst refs rt hen e he).2.2
      simpa only [renamePages4,dirty,clean,typed,Bool.not_true,Bool.false_and,Bool.not_false] using (show True from trivial)
    · simpa only [rename_not_got4 x src dst refs rt p mem] using hd

end Storage3
