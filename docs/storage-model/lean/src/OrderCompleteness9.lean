import OrderTransitions9
namespace Storage32
open Storage21
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

theorem complete_discharge9 (sys : Sys P) (obl : List (Obl P)) (q : Fin P) (ver : Int)
    (valid : ∀ gt ∈ sys.gates, gt.vers gt.dst ≠ noneBytes)
    (complete : ∀ gt ∈ sys.gates, gt.src ∈ gt.pending → gt.dst ∈ gt.pending →
      (gt.src,gt.dst,gt.vers gt.dst,gt.dst) ∈ obl) :
    ∀ gt ∈ (discharge sys q ver).gates, gt.src ∈ gt.pending → gt.dst ∈ gt.pending →
      (gt.src,gt.dst,gt.vers gt.dst,gt.dst) ∈ oblPublished obl q ver := by
  intro ng hng ha hb
  obtain ⟨gt,hgt,eq⟩ := List.mem_filterMap.mp hng
  let next := if gt.vers q != noneBytes && ver >= gt.vers q then {gt with pending := exclude gt.pending [q]} else gt
  have egn : next = ng := by
    change (if next.pending.isEmpty then none else some next) = some ng at eq
    split at eq
    · contradiction
    · exact Option.some.inj eq
  subst ng
  by_cases cond : (gt.vers q != noneBytes && ver >= gt.vers q) = true
  · simp only [next,cond,ite_true] at ha hb ⊢
    have ha0 : gt.src ∈ gt.pending ∧ gt.src ≠ q := by simpa [exclude] using ha
    have hb0 : gt.dst ∈ gt.pending ∧ gt.dst ≠ q := by simpa [exclude] using hb
    apply List.mem_filter.mpr
    exact ⟨complete gt hgt ha0.1 hb0.1,by simp [hb0.2]⟩
  · simp only [next,cond,ite_false] at ha hb ⊢
    apply List.mem_filter.mpr
    refine ⟨complete gt hgt ha hb,?_⟩
    simp only [oblPublished]
    change (!(gt.dst == q && ver >= gt.vers gt.dst)) = true
    by_cases dstq : gt.dst = q
    · have notle : ver < gt.vers gt.dst := by
        have cond0 : ¬(gt.vers q ≠ noneBytes ∧ ver ≥ gt.vers q) := by simpa using cond
        have valid0 := valid gt hgt
        have qvalid : gt.vers q ≠ noneBytes := by simpa [dstq] using valid0
        have notle : ¬ver ≥ gt.vers q := fun le => cond0 ⟨qvalid,le⟩
        rw [dstq]
        omega
      simp [notle]
    · simp [dstq]
theorem complete_discarded9 (sys : Sys P) (obl : List (Obl P)) (k : Fin P)
    (complete : ∀ gt ∈ sys.gates, gt.src ∈ gt.pending → gt.dst ∈ gt.pending →
      (gt.src,gt.dst,gt.vers gt.dst,gt.dst) ∈ obl) :
    ∀ gt ∈ (discarded sys k).gates, gt.src ∈ gt.pending → gt.dst ∈ gt.pending →
      (gt.src,gt.dst,gt.vers gt.dst,gt.dst) ∈ oblDiscarded obl k := by
  intro ng hng ha hb
  obtain ⟨gt,hgt,eq⟩ := List.mem_filterMap.mp hng
  let next := if gt.dst == k then gt else {gt with pending := exclude gt.pending [k]}
  have notcancel : (gt.dst == k && gt.pending.contains k) = false := by
    change (if (gt.dst == k && gt.pending.contains k) || next.pending.isEmpty then none else some next) = some ng at eq
    by_cases cancel : (gt.dst == k && gt.pending.contains k) = true
    · rw [cancel] at eq
      simp at eq
    · simpa using cancel
  have egn : next = ng := by
    change (if (gt.dst == k && gt.pending.contains k) || next.pending.isEmpty then none else some next) = some ng at eq
    split at eq
    · contradiction
    · exact Option.some.inj eq
  subst ng
  by_cases destk : gt.dst = k
  · have memk : k ∈ gt.pending := by simpa [next,destk] using hb
    simp [destk,memk] at notcancel
  · simp only [next,show (gt.dst == k) = false by simpa using destk,ite_false] at ha hb ⊢
    have ha0 : gt.src ∈ gt.pending ∧ gt.src ≠ k := by simpa [exclude] using ha
    have hb0 : gt.dst ∈ gt.pending ∧ gt.dst ≠ k := by simpa [exclude] using hb
    apply List.mem_filter.mpr
    exact ⟨complete gt hgt ha0.1 hb0.1,by simp [ha0.2,hb0.2]⟩
#print axioms complete_discharge9
#print axioms complete_discarded9
end Storage32
