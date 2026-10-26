import OrderOld9
namespace Storage32
open Storage21
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

theorem opFree_excludes9 (sys : Storage3.Sys P) (ps : List (Fin P))
    (free : Storage3.opFree sys ps = true) (on : sys.job.on = true) :
    ∀ p ∈ ps,p ≠ sys.job.p := by
  have no : sys.job.p ∉ ps := by simpa [Storage3.opFree,on] using free
  intro p hp eq
  exact no (eq ▸ hp)

theorem safe_orders_opRename9 (c : Config P) (x : State P) (src dst : Fin P)
    (refs : List (Fin P)) (rt : Fin P → Int) (mutant : c.mutant = "none")
    (h : SafeOrders x) (nonnegative : 0 ≤ x.g.vc) (en : enabled c x (.opRename src dst refs rt) = true) :
    SafeOrders (opRename c x src dst refs rt) := by
  by_cases absentSrc : Storage3.opCur x.s.toBase src = absent
  · apply safe_orders_same9 (x := x) (h := h)
    · simp [opRename,absentSrc,lift]
    · simp [opRename,absentSrc,lift]
    · rfl
    · exact job_follows32_9 c x (.opRename src dst refs rt) mutant (by constructor <;> intro p <;> simp)
  · have old := enabled_projects9 c x (.opRename src dst refs rt) en
    have baseEn : Storage3.enabled c (project x) (.opRename src dst refs rt) = true := by
      simp only [Storage31.enabled,Bool.and_eq_true] at old
      exact old.1
    simp only [project] at baseEn
    have distinct : src ≠ dst := by simp [Storage3.enabled] at baseEn; grind
    have free : Storage3.opFree x.s.toBase (union [src,dst] refs) = true := by
      simp [Storage3.enabled,absentSrc] at baseEn
      grind
    have refShape : ∀ p ∈ refs,p ≠ src ∧ p ≠ dst := by
      intro p hp
      have allRefs : refs.all (fun r => r != src && r != dst && c.texts.contains (rt r)) = true := by
        simp only [Storage3.enabled,Bool.and_eq_true] at baseEn
        grind
      have hr := List.all_eq_true.mp allRefs p hp
      simp only [Bool.and_eq_true] at hr
      simpa using hr.1
    apply safe_orders_installed9 (x := x) (h := h) (dst := dst) (src := src)
      (ps := union [src,dst] refs) (others := refs)
    · exact nonnegative
    · exact distinct
    · simp [mem_union_general]
    · simp [mem_union_general]
    · intro p hp; exact ⟨(mem_union_general _ _ _).mpr (Or.inr hp),refShape p hp⟩
    · simp [opRename,absentSrc,lift]
    · simp [opRename,absentSrc,lift]
    · rfl
    · rfl
    · exact opFree_excludes9 x.s.toBase (union [src,dst] refs) free
end Storage32
namespace Storage33
open Storage21
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

theorem safe_orders_apply9 (c : Config P) (x : State P) (eff plan : Map P) (dst src : Option (Fin P))
    (mutant : c.mutant = "none") (h : Storage32.SafeOrders x) (nonnegative : 0 ≤ x.g.vc)
    (en : enabledApply c x eff plan dst src = true) :
    Storage32.SafeOrders (opApply c x eff plan dst src) := by
  cases dst with
  | none =>
    apply Storage32.safe_orders_same9 (x := x) (h := h)
    · rfl
    · rfl
    · rfl
    · exact fun on => ⟨on,rfl⟩
  | some d =>
    cases src with
    | none => simp [enabledApply] at en
    | some s =>
      have ds : d ≠ s := by simp [enabledApply] at en; grind
      have dm : d ∈ keys eff := by simp [enabledApply] at en; grind
      have sm : s ∈ keys eff := by simp [enabledApply] at en; grind
      have free : Storage3.opFree x.s.toBase (keys eff) = true := by
        simp only [enabledApply,Bool.and_eq_true] at en; grind
      apply Storage32.safe_orders_installed9 (x := x) (h := h) (dst := d) (src := s)
        (ps := keys eff) (others := exclude (keys eff) [d,s])
      · exact nonnegative
      · exact Ne.symm ds
      · exact sm
      · exact dm
      · intro p hp; simp [exclude] at hp; grind
      · simp [opApply,mutant,Storage32.commit]
      · rfl
      · rfl
      · rfl
      · exact Storage32.opFree_excludes9 x.s.toBase (keys eff) free
end Storage33
