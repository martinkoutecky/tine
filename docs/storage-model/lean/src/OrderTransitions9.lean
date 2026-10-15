import OrderCore9
namespace Storage32
open Storage21
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

theorem backed_discharge9 (sys : Sys P) (obl : List (Obl P)) (q : Fin P) (ver : Int)
    (backed : ∀ o ∈ obl, Backed sys o) (free : ∀ o ∈ obl, o.1 ≠ q) :
    ∀ o ∈ oblPublished obl q ver, Backed (discharge sys q ver) o := by
  intro o ho
  have parts := List.mem_filter.mp ho
  have old := backed o parts.1
  obtain ⟨gt,hgt,ha,hb,hab,had,hd,hv,hn,hshape⟩ := old
  have survives : ¬(o.2.1 = q ∧ ver ≥ o.2.2.1) := by
    have hk := parts.2
    change (!(o.2.1 == q && ver >= o.2.2.1)) = true at hk
    rintro ⟨eq,le⟩
    simp [eq,le] at hk
  let ng := if gt.vers q != noneBytes && ver >= gt.vers q then {gt with pending := exclude gt.pending [q]} else gt
  have na : o.1 ∈ ng.pending := by
    dsimp [ng]
    split
    · simp [exclude,ha,free o parts.1]
    · exact ha
  have nb : o.2.1 ∈ ng.pending := by
    dsimp [ng]
    split
    · have neq : o.2.1 ≠ q := by
        intro eq
        have cond : (gt.vers q != noneBytes && ver >= gt.vers q) = true := by assumption
        simp only [Bool.and_eq_true] at cond
        have le : ver ≥ o.2.2.1 := by simpa [← eq,hv] using cond.2
        exact survives ⟨eq,le⟩
      simp [exclude,hb,neq]
    · exact hb
  have nonempty : ng.pending.isEmpty = false := by
    cases h : ng.pending <;> simp_all
  have memng : ng ∈ (discharge sys q ver).gates := by
    apply List.mem_filterMap.mpr
    refine ⟨gt,hgt,?_⟩
    change (if ng.pending.isEmpty then none else some ng) = some ng
    simp [nonempty]
  refine ⟨ng,memng,na,nb,hab,?_,?_,?_,?_,?_⟩
  all_goals dsimp [ng]; split <;> assumption

theorem backed_discarded9 (sys : Sys P) (obl : List (Obl P)) (k : Fin P)
    (backed : ∀ o ∈ obl, Backed sys o)
    (complete : ∀ gt ∈ sys.gates, gt.src ∈ gt.pending → gt.dst ∈ gt.pending →
      (gt.src,gt.dst,gt.vers gt.dst,gt.dst) ∈ obl) :
    ∀ o ∈ oblDiscarded obl k, Backed (discarded sys k) o := by
  intro o ho
  have parts := List.mem_filter.mp ho
  obtain ⟨gt,hgt,ha,hb,hab,had,hd,hv,hn,hshape⟩ := backed o parts.1
  have kept : o.1 ≠ k ∧ o.2.1 ≠ k ∧
      ¬((obl.any fun e => e.2.1 == k && e.2.2.2 == k) = true ∧ o.2.2.2 = k) := by
    have hk := parts.2
    change (!(o.1 == k || o.2.1 == k || ((obl.any fun e => e.2.1 == k && e.2.2.2 == k) && o.2.2.2 == k))) = true at hk
    refine ⟨?_,?_,?_⟩
    · intro eq; simp [eq] at hk
    · intro eq; simp [eq] at hk
    · rintro ⟨cancel,eq⟩; simp [cancel,eq] at hk
  have nocancel : ¬(gt.dst = k ∧ k ∈ gt.pending) := by
    rintro ⟨eq,mem⟩
    rcases hshape with source | dest
    · have srcmem : gt.src ∈ gt.pending := source ▸ ha
      have dm : gt.dst ∈ gt.pending := eq ▸ mem
      have edge := complete gt hgt srcmem dm
      have cancel : (obl.any fun e => e.2.1 == k && e.2.2.2 == k) = true := by
        apply List.any_eq_true.mpr
        exact ⟨(gt.src,gt.dst,gt.vers gt.dst,gt.dst),edge,by simp [eq]⟩
      exact kept.2.2 ⟨cancel,hd.trans eq⟩
    · exact kept.2.1 (dest.trans eq)
  let ng := if gt.dst == k then gt else {gt with pending := exclude gt.pending [k]}
  have na : o.1 ∈ ng.pending := by
    dsimp [ng]; split
    · exact ha
    · simp [exclude,ha,kept.1]
  have nb : o.2.1 ∈ ng.pending := by
    dsimp [ng]; split
    · exact hb
    · simp [exclude,hb,kept.2.1]
  have nonempty : ng.pending.isEmpty = false := by cases h : ng.pending <;> simp_all
  have memng : ng ∈ (discarded sys k).gates := by
    apply List.mem_filterMap.mpr
    refine ⟨gt,hgt,?_⟩
    change (if (gt.dst == k && gt.pending.contains k) || ng.pending.isEmpty then none else some ng) = some ng
    simp [nonempty,nocancel]
  refine ⟨ng,memng,na,nb,hab,?_,?_,?_,?_,?_⟩
  all_goals dsimp [ng]; split <;> assumption
#print axioms backed_discharge9
#print axioms backed_discarded9
end Storage32
