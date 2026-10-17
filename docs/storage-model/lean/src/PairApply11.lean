import PairOperations11
namespace Storage35
open Storage21 Storage32
set_option maxHeartbeats 4000000
set_option maxRecDepth 8192
set_option linter.unusedSimpArgs false
theorem pairSafety_apply11 (c : Config P) (x : State P) (eff plan : Storage33.Map P)
    (dst src : Option (Fin P)) (mutant : c.mutant = "none") (h : PairSafety10 x)
    (en : enabled c x (.opApply eff plan dst src) = true) :
    PairSafety10 (execute c x (.opApply eff plan dst src)) := by
  have baseEn : Storage33.enabledApply c (project x) eff plan dst src = true := by
    simp only [enabled,Storage34.enabled,Storage34.enabledApply,Bool.and_eq_true] at en
    exact en.1
  let y := execute c x (.opApply eff plan dst src)
  let oldy := {y with g := {y.g with renamed := unrename x.g.renamed (Storage33.keys eff)}}
  have unchanged (p : Fin P) (np : p ∉ Storage33.keys eff) : y.s.pages p = x.s.pages p := by
    simp [y,execute,Storage34.opApply,Storage33.opApply,Storage32.commit,Storage3.commit,Storage21.commit,
      Storage3.fold_put_if,np,project,Storage34.project]
  have oldSafety : PairSafety10 oldy := by
    apply pairSafety_endpoints11 x oldy h rfl
    · intro r mem; exact (List.mem_filter.mp mem).1
    · intro r mem
      have ne : r.1 ∉ Storage33.keys eff ∧ r.2 ∉ Storage33.keys eff := by
        simpa [unrename] using (List.mem_filter.mp mem).2
      exact ⟨by simpa only [oldy,unchanged r.1 ne.1] using pageSafe_refl11 (x.s.disk r.1) (x.s.pages r.1),
        by simpa only [oldy,unchanged r.2 ne.2] using pageSafe_refl11 (x.s.disk r.2) (x.s.pages r.2)⟩
    · intro gt gm
      simp only [oldy,y,execute,Storage34.opApply,Storage33.opApply,Storage32.commit]
      split <;> simp_all [project,Storage34.project]
    · rfl
  intro r mem
  change r ∈ (Storage33.opApply c (project x) eff plan dst src).g.renamed at mem
  simp only [Storage33.opApply,Storage32.commit] at mem
  rcases (mem_union_general _ _ _).mp mem with old | fresh
  · exact oldSafety r old
  · split at fresh
    · have eq : r = (src.getD (zero c),dst.getD (zero c)) := by simpa using fresh
      subst r
      cases dst with
      | none => simp_all
      | some d =>
        cases src with
        | none => simp [Storage33.enabledApply] at baseEn
        | some s =>
          have dm : d ∈ Storage33.keys eff := by simp [Storage33.enabledApply] at baseEn; grind
          have sm : s ∈ Storage33.keys eff := by simp [Storage33.enabledApply] at baseEn; grind
          have bytes : Storage33.get eff d ≠ absent := by simp [Storage33.enabledApply] at baseEn; grind
          have disk : x.s.disk s ≠ absent := by
            have raw : (project x).s.disk s ≠ absent := by
              simpa using (by assumption : ((some d : Option (Fin P)).isSome && (project x).s.disk ((some s).getD (zero c)) != absent) = true)
            exact raw
          have free : Storage3.opFree x.s.toBase (Storage33.keys eff) = true := by
            simp [Storage33.enabledApply,project,Storage34.project] at baseEn; grind
          have hf (on : x.s.job.on = true) := opFree_excludes9 _ _ free on
          have dp := Storage33.apply_effects_drafted8 c (project x) eff plan (some d) (some s) mutant d dm
          right; right
          refine ⟨disk,dp.1,?_,dp.2.2.2.1,?_,?_,makeGate (project x) d s (Storage33.keys eff),?_,rfl,rfl,sm,dm⟩
          · change ((Storage33.opApply c (project x) eff plan (some d) (some s)).s.pages d).buf ≠ absent
            rw [dp.2.1]; exact bytes
          · intro on; exact Ne.symm (hf on s sm)
          · intro on eq; exact False.elim (hf on d dm eq.symm)
          · simp [execute,Storage34.opApply,Storage33.opApply,Storage32.commit,mutant,project,Storage34.project]
    · simp at fresh
end Storage35
