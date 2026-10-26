import PairSafety35
namespace Witness35
open Storage21
set_option maxHeartbeats 8000000
set_option maxRecDepth 8192
def c : Config 3 := {positive := by decide,texts := [1,2,3],nonempty := by decide}
def p0 : Fin 3 := ⟨0,by decide⟩
def p1 : Fin 3 := ⟨1,by decide⟩
def p2 : Fin 3 := ⟨2,by decide⟩
def moveActions : List (Storage35.Action 3) := [
  .prior (.load p0),.prior (.load p1),.prior (.load p2),
  .prior (.opRename p0 p2 [p1] (fun _ => 3)),
  .prior (.old (.flush p2)),.prior (.old .check),.prior (.old .rename),.prior (.old (.dirSync true)),
  .prior (.old (.flush p1)),.prior (.old .check),.prior (.old .rename),.prior (.old (.dirSync true)),
  .prior (.flushDel p0),.prior (.old .check),.prior (.old .rename),.prior (.old (.dirSync true))]
def restoreActions : List (Storage35.Action 3) := [
  .prior (.load p0),.prior (.load p1),.prior (.load p2),
  .prior (.opRename p0 p2 [p1] (fun _ => 3)),
  .prior (.old (.wOpen p2)),.prior (.old (.deliverUp true)),.prior (.old (.wRecv p2)),
  .prior (.old (.wDiscard p2)),.prior (.old (.deliverUp true))]
def applyActions : List (Storage35.Action 3) := [
  .prior (.load p0),.prior (.load p1),.prior (.load p2),
  .opApply (fun p => if p = p0 then some absent else if p = p1 then some 3 else some 1)
    (fun p => some (Storage31.initDisk c p)) (some p2) (some p0),
  .prior (.old (.flush p2)),.prior (.old .check),.prior (.old .rename),.prior (.old (.dirSync true)),
  .prior (.old (.flush p1)),.prior (.old .check),.prior (.old .rename),.prior (.old (.dirSync true)),
  .prior (.flushDel p0),.prior (.old .check),.prior (.old .rename),.prior (.old (.dirSync true))]
def run (x : Storage35.State 3) : List (Storage35.Action 3) → Storage35.State 3
  | [] => x
  | a::as => run (Storage35.execute c x a) as
def Legal (x : Storage35.State 3) : List (Storage35.Action 3) → Prop
  | [] => True
  | a::as => Storage35.enabled c x a = true ∧ Legal (Storage35.execute c x a) as
theorem legal_reachable10 (x : Storage35.State 3) (as : List (Storage35.Action 3))
    (hx : Storage35.Reachable c (Storage31.initDisk c) x) (h : Legal x as) :
    Storage35.Reachable c (Storage31.initDisk c) (run x as) := by
  induction as generalizing x with
  | nil => exact hx
  | cons a as ih => exact ih _ (.advance hx a (by simp [Storage35.step,h.1])) h.2
def moved := run (Storage35.init c) moveActions
def restored := run (Storage35.init c) restoreActions
def applied := run (Storage35.init c) applyActions
theorem move_reachable10 : Storage35.Reachable c (Storage31.initDisk c) moved := by
  apply legal_reachable10 _ moveActions .initial
  simp only [Legal,moveActions]; decide
theorem restoration_reachable10 : Storage35.Reachable c (Storage31.initDisk c) restored := by
  apply legal_reachable10 _ restoreActions .initial
  simp only [Legal,restoreActions]; decide
theorem apply_move_reachable10 : Storage35.Reachable c (Storage31.initDisk c) applied := by
  apply legal_reachable10 _ applyActions .initial
  simp only [Legal,applyActions]; decide
theorem outcomes10 :
    moved.s.disk p0 = absent ∧ moved.s.disk p2 = 1 ∧ moved.s.disk p1 = 3 ∧
    (restored.s.pages p0).buf = 1 ∧ (restored.s.pages p0).ver = restored.g.vc ∧
    applied.s.disk p0 = absent ∧ applied.s.disk p2 = 1 ∧ applied.s.disk p1 = 3 := by decide
theorem all_guarantees10 : Storage35.guarantee moved = true ∧ Storage35.guarantee restored = true ∧
    Storage35.guarantee applied = true := by
  exact ⟨Storage35.guarantee_reachable35_10 c 3 rfl (by decide) rfl _ move_reachable10,
    Storage35.guarantee_reachable35_10 c 3 rfl (by decide) rfl _ restoration_reachable10,
    Storage35.guarantee_reachable35_10 c 3 rfl (by decide) rfl _ apply_move_reachable10⟩

-- A fabricated overlapping gate, not an initialized reachable model state.
-- This demonstrates exactly why the existing joint invariant and ordering
-- invariant alone cannot close the proposed pair induction.
def waiting := run (Storage35.init c) (restoreActions.take 8)
def foreignGate : Storage32.Gate 3 :=
  {src := p1,dst := p2,vers := fun p => 4+p.val,pending := [p0,p2,p1]}
def overlapping := {waiting with s := {waiting.s with to32 :=
  {waiting.s.to32 with gates := foreignGate :: waiting.s.gates}}}
def cancelledOverlap := Storage35.execute c overlapping (.prior (.old (.deliverUp true)))
theorem waiting_reachable10 : Storage35.Reachable c (Storage31.initDisk c) waiting := by
  apply legal_reachable10 _ (restoreActions.take 8) .initial
  simp only [restoreActions,Legal,List.take]; decide
theorem overlapping_joint_inv10 :
    Storage31.Inv c (Storage32.project (Storage35.project overlapping)) :=
  Storage35.inv_reachable35_10 c (Storage31.initDisk c) waiting rfl
    (Storage3.positiveTexts_initial_real4 c 3 rfl (by decide))
    (Storage3.positiveTexts_noNone6 c 3 rfl) (Storage3.positiveTexts_noUnknown6 c 3 rfl) waiting_reachable10
theorem overlapping_orders10 : Storage32.SafeOrders (Storage35.project overlapping) := by
  have h := (Storage35.aux_reachable10 c (Storage31.initDisk c) waiting rfl waiting_reachable10).orders
  refine ⟨⟨?_,?_,?_,h.1.2.2.2⟩,?_⟩
  · intro o mem
    obtain ⟨gt,gm,rest⟩ := h.1.1 o mem
    exact ⟨gt,List.mem_cons_of_mem foreignGate gm,rest⟩
  · intro gt gm sm dm
    have cases : gt = foreignGate ∨ gt ∈ waiting.s.gates := by
      simpa [overlapping,Storage35.project,Storage34.project] using gm
    rcases cases with rfl | old
    · change (p1,p2,(4+p2.val : Int),p2) ∈ waiting.g.obl
      decide
    · exact h.1.2.1 gt old sm dm
  · intro on
    have off : overlapping.s.job.on = false := by decide
    simp [Storage35.project,Storage34.project,off] at on
  · intro gt gm
    have cases : gt = foreignGate ∨ gt ∈ waiting.s.gates := by
      simpa [overlapping,Storage35.project,Storage34.project] using gm
    rcases cases with rfl | old
    · decide
    · exact h.2 gt old
def stableCheck10 (x : Storage35.State 3) (p : Fin 3) : Bool :=
  x.s.disk p != absent && (!(x.s.pages p).held || (x.s.pages p).buf != absent) &&
    (!x.s.job.on || x.s.job.p != p || x.s.job.bytes != absent)
def pendingCheck10 (x : Storage35.State 3) : Bool :=
  x.s.disk p0 != absent && (x.s.pages p2).held && (x.s.pages p2).buf != absent && (x.s.pages p2).risk &&
    (!x.s.job.on || x.s.job.p != p0) && (!x.s.job.on || x.s.job.p != p2 || x.s.job.bytes != absent) &&
    x.s.gates.any (fun gt => gt.src == p0 && gt.dst == p2 && gt.pending.contains p0 && gt.pending.contains p2)
def pairCheck10 (x : Storage35.State 3) : Bool := stableCheck10 x p0 || stableCheck10 x p2 || pendingCheck10 x
theorem candidate_not_inductive10 :
    pairCheck10 overlapping = true ∧ overlapping.g.renamed = [(p0,p2)] ∧
    Storage35.enabled c overlapping (.prior (.old (.deliverUp true))) = true ∧
    pairCheck10 cancelledOverlap = false ∧ Storage35.renameResolves cancelledOverlap = true := by decide
end Witness35
