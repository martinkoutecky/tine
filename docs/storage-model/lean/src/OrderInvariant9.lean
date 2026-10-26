import OrderInstall9
import Bounds9
namespace Storage32
open Storage21
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

def GateValid (sys : Sys P) : Prop := ∀ gt ∈ sys.gates, gt.vers gt.dst ≠ noneBytes
def SafeOrders (x : State P) : Prop := OrderInv x ∧ GateValid x.s

theorem valid_discharge9 (sys : Sys P) (q : Fin P) (ver : Int) (h : GateValid sys) :
    GateValid (discharge sys q ver) := by
  intro ng hm
  obtain ⟨gt,mem,eq⟩ := List.mem_filterMap.mp hm
  have old := h gt mem
  simp only [discharge] at eq
  split at eq <;> (try split at eq)
  all_goals first | contradiction | (cases eq; exact old)

theorem valid_discarded9 (sys : Sys P) (k : Fin P) (h : GateValid sys) :
    GateValid (discarded sys k) := by
  intro ng hm
  obtain ⟨gt,mem,eq⟩ := List.mem_filterMap.mp hm
  have old := h gt mem
  simp only [discarded] at eq
  split at eq <;> (try split at eq)
  all_goals first | contradiction | (cases eq; exact old)

theorem safe_orders_same9 (x y : State P) (h : SafeOrders x)
    (gates : y.s.gates = x.s.gates) (obl : y.g.obl = x.g.obl)
    (ok : y.g.orderOk = x.g.orderOk)
    (job : y.s.job.on = true → x.s.job.on = true ∧ y.s.job.p = x.s.job.p) : SafeOrders y := by
  refine ⟨⟨?_,?_,?_,?_⟩,?_⟩
  · intro o ho
    obtain ⟨gt,mem,rest⟩ := h.1.1 o (obl ▸ ho)
    exact ⟨gt,gates.symm ▸ mem,rest⟩
  · intro gt mem src dst
    rw [obl]
    exact h.1.2.1 gt (gates ▸ mem) src dst
  · intro on o ho
    have hj := job on
    rw [hj.2]
    exact h.1.2.2.1 hj.1 o (obl ▸ ho)
  · rw [ok]; exact h.1.2.2.2
  · intro gt mem; exact h.2 gt (gates ▸ mem)

theorem safe_orders_published9 (x y : State P) (h : SafeOrders x) (q : Fin P) (ver : Int)
    (gates : y.s.gates = (discharge x.s q ver).gates)
    (obl : y.g.obl = oblPublished x.g.obl q ver) (ok : y.g.orderOk = x.g.orderOk)
    (job : y.s.job.on = false) (free : ∀ o ∈ x.g.obl, o.1 ≠ q) : SafeOrders y := by
  refine ⟨⟨?_,?_,?_,?_⟩,?_⟩
  · intro o ho
    obtain ⟨gt,mem,rest⟩ := backed_discharge9 x.s x.g.obl q ver h.1.1 free o (obl ▸ ho)
    exact ⟨gt,gates.symm ▸ mem,rest⟩
  · intro gt mem src dst
    rw [obl]
    exact complete_discharge9 x.s x.g.obl q ver h.2 h.1.2.1 gt (gates ▸ mem) src dst
  · simp [job]
  · rw [ok]; exact h.1.2.2.2
  · intro gt mem; exact valid_discharge9 x.s q ver h.2 gt (gates ▸ mem)

theorem safe_orders_discarded9 (x y : State P) (h : SafeOrders x) (k : Fin P)
    (gates : y.s.gates = (discarded x.s k).gates)
    (obl : y.g.obl = oblDiscarded x.g.obl k) (ok : y.g.orderOk = x.g.orderOk)
    (job : y.s.job = x.s.job) : SafeOrders y := by
  refine ⟨⟨?_,?_,?_,?_⟩,?_⟩
  · intro o ho
    obtain ⟨gt,mem,rest⟩ := backed_discarded9 x.s x.g.obl k h.1.1 h.1.2.1 o (obl ▸ ho)
    exact ⟨gt,gates.symm ▸ mem,rest⟩
  · intro gt mem src dst
    rw [obl]
    exact complete_discarded9 x.s x.g.obl k h.1.2.1 gt (gates ▸ mem) src dst
  · intro on o ho
    rw [job] at on ⊢
    have old : o ∈ x.g.obl := (List.mem_filter.mp (obl ▸ ho)).1
    exact h.1.2.2.1 on o old
  · rw [ok]; exact h.1.2.2.2
  · intro gt mem; exact valid_discarded9 x.s k h.2 gt (gates ▸ mem)

theorem safe_orders_installed9 (x y : State P) (h : SafeOrders x) (dst src : Fin P)
    (ps others : List (Fin P)) (nonnegative : 0 ≤ x.g.vc) (distinct : src ≠ dst)
    (hsrc : src ∈ ps) (hdst : dst ∈ ps)
    (hother : ∀ p ∈ others, p ∈ ps ∧ p ≠ src ∧ p ≠ dst)
    (gates : y.s.gates = x.s.gates ++ [makeGate x dst src ps])
    (obl : y.g.obl = union x.g.obl (makeObl x dst src others))
    (ok : y.g.orderOk = x.g.orderOk) (job : y.s.job = x.s.job)
    (free : x.s.job.on = true → ∀ p ∈ ps, p ≠ x.s.job.p) : SafeOrders y := by
  have newEdges := makeObl_backed9 x dst src ps others nonnegative distinct hsrc hdst hother
  have version : (makeGate x dst src ps).vers dst = x.g.vc+1+dst.val := by simp [makeGate,hdst]
  have valid : x.g.vc+1+dst.val ≠ noneBytes := by unfold noneBytes; omega
  refine ⟨⟨?_,?_,?_,?_⟩,?_⟩
  · intro o ho
    rw [obl,mem_union_general] at ho
    rcases ho with old | fresh
    · obtain ⟨gt,mem,rest⟩ := h.1.1 o old
      exact ⟨gt,by simp [gates,mem],rest⟩
    · obtain ⟨gt,mem,rest⟩ := newEdges o fresh
      have same : gt = makeGate x dst src ps := by simpa using mem
      subst gt
      exact ⟨makeGate x dst src ps,by simp [gates],rest⟩
  · intro gt mem srcmem dstmem
    rw [gates,List.mem_append,List.mem_singleton] at mem
    rw [obl,mem_union_general]
    rcases mem with old | rfl
    · exact Or.inl (h.1.2.1 gt old srcmem dstmem)
    · right
      change (src,dst,(makeGate x dst src ps).vers dst,dst) ∈ makeObl x dst src others
      rw [version]
      apply (mem_union_general _ _ _).mpr
      right
      simp
  · intro on o ho
    rw [job] at on ⊢
    rw [obl,mem_union_general] at ho
    rcases ho with old | fresh
    · exact h.1.2.2.1 on o old
    · obtain ⟨gt,mem,source,_⟩ := newEdges o fresh
      have same : gt = makeGate x dst src ps := by simpa using mem
      subst gt
      exact free on o.1 source
  · rw [ok]; exact h.1.2.2.2
  · intro gt mem
    rw [gates,List.mem_append,List.mem_singleton] at mem
    rcases mem with old | rfl
    · exact h.2 gt old
    · change (makeGate x dst src ps).vers dst ≠ noneBytes
      rw [version]; exact valid

theorem safe_orders_flush9 (c : Config P) (x : State P) (p : Fin P)
    (mutant : c.mutant = "none") (h : SafeOrders x) (gate : gated c x.s p = false) :
    SafeOrders (flush c x p) := by
  refine ⟨⟨h.1.1,h.1.2.1,?_,?_⟩,h.2⟩
  · intro on o ho eq
    have yes := backed_gated9 c x.s o mutant (h.1.1 o ho)
    change o.1 = p at eq
    simp [eq,gate] at yes
  · exact order_flush9 c x p mutant h.1 gate

#print axioms safe_orders_installed9
#print axioms safe_orders_flush9
#print axioms safe_orders_same9
#print axioms safe_orders_published9
#print axioms safe_orders_discarded9
end Storage32
