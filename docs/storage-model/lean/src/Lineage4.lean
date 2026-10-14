import Bounds4
namespace Storage3
open Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

theorem ack_idem4 (pr : Promise) (b v : Int) :
    ack (ack pr b v false 0) b v false 0 = ack pr b v false 0 := by
  simp only [ack]
  split
  · simp
  · rename_i no; simp [no]

theorem fold_ack_point4 (es : List (Fin P × Int)) (pr : Fin P → Promise)
    (v : Fin P → Int) (p : Fin P) (b : Int)
    (same : ∀ e ∈ es, e.1 = p → e.2 = b) :
    es.foldl (fun acc e => put acc e.1 (ack (acc e.1) e.2 (v e.1) false 0)) pr p =
      if p ∈ es.map Prod.fst then ack (pr p) b (v p) false 0 else pr p := by
  induction es generalizing pr with
  | nil => simp
  | cons e tail ih =>
    rw [List.foldl_cons,ih _ (by intro f hf; exact same f (by simp [hf]))]
    by_cases he : e.1 = p
    · have hb := same e (by simp) he
      simp [he,hb,put,ack_idem4]
    · by_cases ht : p ∈ tail.map Prod.fst <;> simp [ht,put,he,Ne.symm he]

abbrev renamePages4 (x : State P) (src dst : Fin P) (refs : List (Fin P)) (rt : Fin P → Int) :=
  refs.foldl (fun acc r => put acc r {opEdit x.s r (rt r) (x.g.vc+1+r.val) with risk := true})
    (if opCur x.s src != absent then
      put (put x.s.pages dst {opEdit x.s dst (opCur x.s src) (x.g.vc+1+dst.val) with risk := true})
        src {opEdit x.s src absent (x.g.vc+1+src.val) with risk := true} else x.s.pages)
abbrev renameGot4 (x : State P) (src dst : Fin P) (refs : List (Fin P)) (rt : Fin P → Int) :=
  union (union (refs.map fun r => (r,rt r))
    (if opCur x.s src != absent then [(dst,opCur x.s src)] else []))
    (if opCur x.s src != absent then [(src,absent)] else [])
abbrev renamePromise4 (x : State P) (src dst : Fin P) (refs : List (Fin P)) (rt : Fin P → Int) :=
  (renameGot4 x src dst refs rt).foldl
    (fun acc e => put acc e.1 (ack (acc e.1) e.2 (x.g.vc+1+e.1.val) false 0)) x.g.promise

theorem rename_refs4 (c : Config P) (x : State P) (src dst : Fin P)
    (refs : List (Fin P)) (rt : Fin P → Int)
    (hen : enabled c x (.opRename src dst refs rt) = true) :
    src ≠ dst ∧ ∀ p ∈ refs, p ≠ src ∧ p ≠ dst := by
  simp only [enabled,Bool.and_eq_true] at hen
  have all : refs.all (fun r => r != src && r != dst && c.texts.contains (rt r)) = true := by grind
  constructor
  · grind
  · intro p hp; have h := List.all_eq_true.mp all p hp; simp only [Bool.and_eq_true,bne_iff_ne] at h; exact h.1

theorem rename_got_shape4 (c : Config P) (x : State P) (src dst : Fin P)
    (refs : List (Fin P)) (rt : Fin P → Int)
    (hen : enabled c x (.opRename src dst refs rt) = true) :
    ∀ e ∈ renameGot4 x src dst refs rt,
      (renamePages4 x src dst refs rt e.1).buf = e.2 ∧
      (renamePages4 x src dst refs rt e.1).ver = x.g.vc+1+e.1.val ∧
      (renamePages4 x src dst refs rt e.1).typed = true := by
  have ne := rename_refs4 c x src dst refs rt hen
  have srcnot : src ∉ refs := by intro hs; exact (ne.2 src hs).1 rfl
  have dstnot : dst ∉ refs := by intro hs; exact (ne.2 dst hs).2 rfl
  intro e he
  rcases (mem_union_general _ _ _).mp he with first | last
  · rcases (mem_union_general _ _ _).mp first with ref | dest
    · obtain ⟨p,hp,rfl⟩ := List.mem_map.mp ref
      simp [renamePages4,fold_put_if,hp,opEdit]
    · by_cases full : opCur x.s src = absent
      · simp [full] at dest
      · have eq : e = (dst,opCur x.s src) := by simpa [full] using dest
        subst e
        simp [renamePages4,fold_put_if,dstnot,full,put,Ne.symm ne.1,opEdit]
  · by_cases full : opCur x.s src = absent
    · simp [full] at last
    · have eq : e = (src,absent) := by simpa [full] using last
      subst e
      simp [renamePages4,fold_put_if,srcnot,full,put,opEdit]

theorem rename_not_got4 (x : State P) (src dst : Fin P)
    (refs : List (Fin P)) (rt : Fin P → Int) (p : Fin P)
    (hn : p ∉ (renameGot4 x src dst refs rt).map Prod.fst) :
    renamePages4 x src dst refs rt p = x.s.pages p := by
  have nr : p ∉ refs := by
    intro hp; apply hn; apply List.mem_map.mpr; refine ⟨(p,rt p),?_,rfl⟩
    apply (mem_union_general _ _ _).mpr; left; apply (mem_union_general _ _ _).mpr; left
    exact List.mem_map.mpr ⟨p,hp,rfl⟩
  rw [renamePages4,fold_put_not_mem _ refs _ p nr]
  by_cases full : opCur x.s src = absent
  · simp [full]
  · have ns : p ≠ src := by
      intro eq; subst p; apply hn; apply List.mem_map.mpr; refine ⟨(src,absent),?_,rfl⟩
      apply (mem_union_general _ _ _).mpr; right; simp [full]
    have nd : p ≠ dst := by
      intro eq; subst p; apply hn; apply List.mem_map.mpr; refine ⟨(dst,opCur x.s src),?_,rfl⟩
      apply (mem_union_general _ _ _).mpr; left; apply (mem_union_general _ _ _).mpr; right; simp [full]
    simp [full,put,ns,nd]

theorem rename_lineage_pre4 (c : Config P) (x : State P) (src dst : Fin P)
    (refs : List (Fin P)) (rt : Fin P → Int)
    (h : PromiseLineage (project x)) (hen : enabled c x (.opRename src dst refs rt) = true) :
    PromiseLineage ⟨{x.s.toSys with pages := renamePages4 x src dst refs rt},
      {x.g.toGhost with vc := x.g.vc+P, promise := renamePromise4 x src dst refs rt}⟩ := by
  have fresh : ∀ p : Fin P, x.g.vc < x.g.vc+1+p.val ∧ x.g.vc+1+p.val ≤ x.g.vc+P := by
    intro p; have hp := p.isLt; omega
  have perPage : ∀ p, (renamePromise4 x src dst refs rt p).ver ≤ x.g.vc+P ∧
      LineagePage (renamePages4 x src dst refs rt p) (renamePromise4 x src dst refs rt p) := by
    intro p
    have fg := fold_ack_point4 (renameGot4 x src dst refs rt) x.g.promise
      (fun p => x.g.vc+1+p.val) p (renamePages4 x src dst refs rt p).buf
      (by intro e he eq; have hp := (rename_got_shape4 c x src dst refs rt hen e he).1; simpa [eq] using hp.symm)
    change renamePromise4 x src dst refs rt p = _ at fg
    rw [fg]
    by_cases hm : p ∈ (renameGot4 x src dst refs rt).map Prod.fst
    · obtain ⟨e,he,ep⟩ := List.mem_map.mp hm
      have shape := rename_got_shape4 c x src dst refs rt hen e he
      have ver : (renamePages4 x src dst refs rt p).ver = x.g.vc+1+p.val := by simpa [ep] using shape.2.1
      have typed : (renamePages4 x src dst refs rt p).typed = true := by simpa [ep] using shape.2.2
      have old := h.1 p
      change (x.g.promise p).ver ≤ x.g.vc at old
      have newer : (x.g.promise p).ver < x.g.vc+1+p.val := by have hf := (fresh p).1; omega
      have ackEq := ack_newer (x.g.promise p) (renamePages4 x src dst refs rt p).buf (x.g.vc+1+p.val) false 0 newer
      simp only [hm,ite_true,ackEq]
      refine ⟨(fresh p).2,?_,?_⟩
      · intro _; simp [ver,EqualLineage]
      · simp [clean,typed]
    · simp only [hm,ite_false]
      rw [rename_not_got4 x src dst refs rt p hm]
      have old := h.1 p
      change (x.g.promise p).ver ≤ x.g.vc at old
      exact ⟨by omega,h.2.1 p,h.2.2 p⟩
  exact ⟨fun p => (perPage p).1,fun p => (perPage p).2.1,fun p => (perPage p).2.2⟩

theorem lineage_delete4 (c : Config P) (x : State P) (p : Fin P)
    (mutant : c.mutant = "none") (h : PromiseLineage (project x))
    (hv : VersionBounds x.s.toSys x.g.vc) : PromiseLineage (project (opDelete c x p)) := by
  have old := h.1 p
  change (x.g.promise p).ver ≤ x.g.vc at old
  have ack := ack_newer (x.g.promise p) absent (x.g.vc+1) false 0 (by omega)
  simp only [opDelete,commit,project,mutant]
  apply lineage_commit
  · simp only [show ("none" == "MDD") = false from rfl,Bool.false_eq_true,ite_false,ack]
    apply lineage_update x.s.toSys x.g.toGhost
    · exact h
    · omega
    · dsimp only; omega
    · constructor
      · intro _; simp [opEdit,EqualLineage]
      · simp [opEdit,clean]
  · apply property_put _ _ _ (fun pg : Page => pg.ver ≤ x.g.vc+1)
    · intro q; exact Int.le_add_one (hv.pages q)
    · simp [opEdit]

theorem lineage_opRename4 (c : Config P) (x : State P) (src dst : Fin P)
    (refs : List (Fin P)) (rt : Fin P → Int) (mutant : c.mutant = "none")
    (h : PromiseLineage (project x)) (hv : VersionBounds x.s.toSys x.g.vc)
    (hen : enabled c x (.opRename src dst refs rt) = true) :
    PromiseLineage (project (opRename c x src dst refs rt)) := by
  simp only [opRename,commit,project,mutant]
  apply lineage_commit
  · simpa only [PromiseLineage,CleanSafe,renamePages4,renamePromise4,renameGot4,project,
      show ("none" != "MRN5") = true from rfl,Bool.and_true] using
      rename_lineage_pre4 c x src dst refs rt h hen
  · exact (bounds_opRename4 c x src dst refs rt hv).pages

theorem lineage_execute4 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (h : PromiseLineage (project x))
    (shape : Shapes x.s.toSys) (hv : VersionBounds x.s.toSys x.g.vc)
    (hj : JobSnapshot x.s.toSys) (hc : JobClear x.s.toSys)
    (hi : InactivePages (project x)) (hen : enabled c x a = true) :
    PromiseLineage (project (execute c x a)) := by
  cases a with
  | old a =>
    have core := lineage_execute c mutant (project x) a h shape hv hj hc hi hen
    have pages : (execute c x (.old a)).s.pages = (Storage21.execute c (project x) a).s.pages :=
      congrArg Storage21.Sys.pages (old_sys4 c x a mutant)
    simpa only [PromiseLineage,CleanSafe,project,pages,
      old_promise4 c x a mutant,old_vc4 c x a mutant] using core
  | opDelete p => exact lineage_delete4 c x p mutant h hv
  | opRename src dst refs rt => exact lineage_opRename4 c x src dst refs rt mutant h hv hen
  | flushDel p => exact lineage_commit c (project x) _ _ _ h hv.pages

theorem lineage_reachable4 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : Reachable c disk x) : PromiseLineage (project x) := by
  induction hr with
  | initial => simp [PromiseLineage,CleanSafe,EqualLineage,initWith,embed,project,Storage21.init]
  | @advance x y hr a hs ih =>
    unfold step at hs; split at hs
    · cases hs; exact lineage_execute4 c x a mutant ih (shapes_reachable4 c disk x mutant hr)
        (bounds_reachable4 c disk x mutant hr) (snapshot_reachable4 c disk x mutant hr)
        (clear_reachable4 c disk x mutant hr) (inactive_reachable3 c disk x mutant hr) (by assumption)
    · contradiction

theorem cleanSafe_reachable4 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : Reachable c disk x) : CleanSafe (project x) :=
  (lineage_reachable4 c disk x mutant hr).2.2
end Storage3
