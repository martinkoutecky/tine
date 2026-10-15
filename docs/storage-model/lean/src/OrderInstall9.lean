import OrderCompleteness9
namespace Storage32
open Storage21
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

theorem makeObl_backed9 (x : State P) (dst src : Fin P) (ps others : List (Fin P))
    (nonnegative : 0 ≤ x.g.vc) (distinct : src ≠ dst)
    (hsrc : src ∈ ps) (hdst : dst ∈ ps)
    (hother : ∀ p ∈ others, p ∈ ps ∧ p ≠ src ∧ p ≠ dst) :
    ∀ o ∈ makeObl x dst src others,
      Backed {x.s with gates := [makeGate x dst src ps]} o := by
  intro o ho
  have unpack : (∃ r ∈ others, o = (r,dst,x.g.vc+1+dst.val,dst)) ∨
      (∃ r ∈ others, o = (src,r,x.g.vc+1+r.val,dst)) ∨
      o = (src,dst,x.g.vc+1+dst.val,dst) := by
    simp only [makeObl,mem_union_general,List.mem_map,List.mem_singleton] at ho
    rcases ho with (first | second) | last
    · obtain ⟨r,mem,eq⟩ := first
      exact Or.inl ⟨r,mem,eq.symm⟩
    · obtain ⟨r,mem,eq⟩ := second
      exact Or.inr (Or.inl ⟨r,mem,eq.symm⟩)
    · exact Or.inr (Or.inr last)
  have vers (p : Fin P) (hp : p ∈ ps) :
      (makeGate x dst src ps).vers p = x.g.vc + 1 + p.val := by simp [makeGate,hp]
  have notnone (p : Fin P) : x.g.vc+1+p.val ≠ noneBytes := by unfold noneBytes; omega
  rcases unpack with ⟨r,hr,rfl⟩ | ⟨r,hr,rfl⟩ | rfl
  · obtain ⟨mem,rs,rd⟩ := hother r hr
    exact ⟨makeGate x dst src ps,by simp,mem,hdst,rd,rd,rfl,vers dst hdst,by rw [vers dst hdst]; exact notnone dst,Or.inr rfl⟩
  · obtain ⟨mem,rs,rd⟩ := hother r hr
    exact ⟨makeGate x dst src ps,by simp,hsrc,mem,Ne.symm rs,distinct,rfl,vers r mem,by rw [vers r mem]; exact notnone r,Or.inl rfl⟩
  · exact ⟨makeGate x dst src ps,by simp,hsrc,hdst,distinct,distinct,rfl,vers dst hdst,by rw [vers dst hdst]; exact notnone dst,Or.inl rfl⟩

#print axioms makeObl_backed9
end Storage32
