import PairLive11
namespace Storage35
open Storage21 Storage32
set_option maxHeartbeats 4000000
set_option maxRecDepth 8192
set_option linter.unusedSimpArgs false
theorem pageSafe_opEdit11 (sys : Storage3.Sys P) (p : Fin P) (t v : Int) (text : t ≠ absent) :
    PageSafe11 (sys.disk p) (sys.pages p) {Storage3.opEdit sys p t v with risk := true} := by
  by_cases held : (sys.pages p).held = true <;> simp_all [PageSafe11,Storage3.opEdit]

theorem rename_pageSafe11 (c : Config P) (x : State P) (src dst : Fin P)
    (refs : List (Fin P)) (rt : Fin P → Int) (texts : absent ∉ c.texts)
    (reftexts : ∀ p ∈ refs, rt p ∈ c.texts) (q : Fin P) (ns : q ≠ src) (nd : q ≠ dst) :
    PageSafe11 (x.s.disk q) (x.s.pages q)
      ((execute c x (.prior (.opRename src dst refs rt))).s.pages q) := by
  simp only [execute,Storage35Base.execute,Storage32.execute,Storage32.opRename,Storage32.lift,
    Storage3.opRename,Storage3.commit,Storage21.commit,Storage34.lift,Storage3.fold_put_if]
  by_cases ref : q ∈ refs
  · simp only [ref,ite_true]
    apply pageSafe_opEdit11
    intro eq; exact texts (eq ▸ reftexts q ref)
  · simp only [ref,ite_false]
    split
    all_goals simpa [put,ns,nd,project,Storage34.project,Storage32.project] using pageSafe_refl11 (x.s.disk q) (x.s.pages q)

theorem pairSafety_opRename11 (c : Config P) (x : State P) (src dst : Fin P)
    (refs : List (Fin P)) (rt : Fin P → Int) (mutant : c.mutant = "none")
    (texts : absent ∉ c.texts) (h : PairSafety10 x)
    (en : enabled c x (.prior (.opRename src dst refs rt)) = true) :
    PairSafety10 (execute c x (.prior (.opRename src dst refs rt))) := by
  let y := execute c x (.prior (.opRename src dst refs rt))
  let oldy := {y with g := {y.g with renamed := unrename x.g.renamed [src,dst]}}
  have baseEn : Storage3.enabled c (Storage32.project (project x)) (.opRename src dst refs rt) = true :=
    (by have raw := Storage32.enabled_projects9 c (project x) (.opRename src dst refs rt) en
        exact (by simp only [Storage31.enabled,Bool.and_eq_true] at raw; exact raw.1))
  have reftexts : ∀ p ∈ refs, rt p ∈ c.texts := by
    simp [Storage3.enabled] at baseEn; grind
  have oldSafety : PairSafety10 oldy := by
    apply pairSafety_endpoints11 x oldy h rfl
    · intro r mem; exact (List.mem_filter.mp mem).1
    · intro r mem
      have ne : (r.1 ≠ src ∧ r.1 ≠ dst) ∧ (r.2 ≠ src ∧ r.2 ≠ dst) := by
        simpa [unrename] using (List.mem_filter.mp mem).2
      exact ⟨rename_pageSafe11 c x src dst refs rt texts reftexts r.1 ne.1.1 ne.1.2,
        rename_pageSafe11 c x src dst refs rt texts reftexts r.2 ne.2.1 ne.2.2⟩
    · intro gt gm
      simp only [oldy,y,execute,Storage35Base.execute,Storage32.execute,Storage32.opRename,
        Storage32.lift,Storage34.lift]
      split <;> simp_all [project,Storage34.project]
    · rfl
  intro r mem
  change r ∈ (Storage32.opRename c (project x) src dst refs rt).g.renamed at mem
  simp only [Storage32.opRename,Storage32.lift] at mem
  rcases (mem_union_general _ _ _).mp mem with old | fresh
  · exact oldSafety r old
  · split at fresh
    · have eq : r = (src,dst) := by simpa using fresh
      subst r
      have full : Storage3.opCur x.s.toBase src ≠ absent ∧ x.s.disk src ≠ absent := by
        simpa [project,Storage34.project] using
          (by assumption : (Storage3.opCur (project x).s.toBase src != absent && (project x).s.disk src != absent) = true)
      have distinct : src ≠ dst := by simp [Storage3.enabled] at baseEn; grind
      have refShape : src ∉ refs ∧ dst ∉ refs := by
        have raw : ∀ p ∈ refs, p ≠ src ∧ p ≠ dst := by simp [Storage3.enabled] at baseEn; grind
        exact ⟨fun hm => (raw src hm).1 rfl,fun hm => (raw dst hm).2 rfl⟩
      have free := Storage3.opRename_unlocked4 c (Storage32.project (project x)) src dst refs rt baseEn
      have free' : x.s.job.on = true → x.s.job.p ≠ src ∧ x.s.job.p ≠ dst := by
        intro on; exact (free on).2 full.1
      right; right
      refine ⟨full.2,?_,?_,?_,fun on => (free' on).1,?_,makeGate (project x) dst src (union [src,dst] refs),?_,rfl,rfl,?_,?_⟩
      · simp [execute,Storage35Base.execute,Storage32.execute,Storage32.opRename,Storage32.lift,
          Storage3.opRename,Storage3.commit,Storage21.commit,Storage34.lift,Storage3.fold_put_if,
          project,Storage34.project,Storage32.project,full.1,refShape.2,put,Ne.symm distinct,Storage3.opEdit_held]
      · simpa [execute,Storage35Base.execute,Storage32.execute,Storage32.opRename,Storage32.lift,
          Storage3.opRename,Storage3.commit,Storage21.commit,Storage34.lift,Storage3.fold_put_if,
          project,Storage34.project,Storage32.project,full.1,refShape.2,put,Ne.symm distinct,Storage3.opEdit] using full.1
      · simp [execute,Storage35Base.execute,Storage32.execute,Storage32.opRename,Storage32.lift,
          Storage3.opRename,Storage3.commit,Storage21.commit,Storage34.lift,Storage3.fold_put_if,
          project,Storage34.project,Storage32.project,full.1,refShape.2,put,Ne.symm distinct]
      · intro on eq; exact False.elim ((free' on).2 eq)
      · simp [execute,Storage35Base.execute,Storage32.execute,Storage32.opRename,Storage32.lift,
          Storage34.lift,project,Storage34.project,full.1]
      · simp [makeGate,mem_union_general]
      · simp [makeGate,mem_union_general]
    · simp at fresh
end Storage35
