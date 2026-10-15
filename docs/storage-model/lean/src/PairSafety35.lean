import Resolution35
namespace Storage35
open Storage21
set_option maxHeartbeats 4000000
set_option maxRecDepth 8192
set_option linter.unusedSimpArgs false
def StableSide10 (x : State P) (p : Fin P) : Prop :=
  x.s.disk p ≠ absent ∧ ((x.s.pages p).held = true → (x.s.pages p).buf ≠ absent) ∧
    (x.s.job.on = true → x.s.job.p = p → x.s.job.bytes ≠ absent)
def PendingPair10 (x : State P) (src dst : Fin P) : Prop :=
  x.s.disk src ≠ absent ∧ (x.s.pages dst).held = true ∧ (x.s.pages dst).buf ≠ absent ∧
  (x.s.pages dst).risk = true ∧ (x.s.job.on = true → x.s.job.p ≠ src) ∧
  (x.s.job.on = true → x.s.job.p = dst → x.s.job.bytes ≠ absent) ∧
  ∃ gt ∈ x.s.gates, gt.src = src ∧ gt.dst = dst ∧ src ∈ gt.pending ∧ dst ∈ gt.pending
def PairSafety10 (x : State P) : Prop := ∀ r ∈ x.g.renamed,
  StableSide10 x r.1 ∨ StableSide10 x r.2 ∨ PendingPair10 x r.1 r.2

theorem pairSafety_resolves10 (x : State P) (h : PairSafety10 x) : renameResolves x = true := by
  apply List.all_eq_true.mpr
  intro r mem
  simp only [Bool.or_eq_true,bne_iff_ne]
  rcases h r mem with src | dst | pending
  · exact Or.inl src.1
  · exact Or.inr dst.1
  · exact Or.inl pending.1

theorem pending_gated10 (c : Config P) (x : State P) (src dst : Fin P)
    (mutant : c.mutant = "none") (h : PendingPair10 x src dst) (ne : src ≠ dst) :
    Storage32.gated c x.s.to32 src = true := by
  obtain ⟨gt,mem,gs,gd,sm,dm⟩ := h.2.2.2.2.2.2
  apply List.any_eq_true.mpr
  exact ⟨gt,mem,by simp [Storage32.gates1,gs,gd,sm,dm,ne,mutant]⟩

theorem stable_flushDelete10 (c : Config P) (x : State P) (p q : Fin P)
    (stable : StableSide10 x q) (en : enabled c x (.prior (.flushDel p)) = true) :
    StableSide10 (execute c x (.prior (.flushDel p))) q := by
  have held : (x.s.pages p).held = true := by
    simp [enabled,Storage34.enabled,Storage32.enabled,Storage31.enabled,Storage3.enabled,
      project,Storage34.project,Storage32.project] at en; grind
  have deleted : (x.s.pages p).buf = absent := by
    simp [enabled,Storage34.enabled,Storage32.enabled,Storage31.enabled,Storage3.enabled,
      project,Storage34.project,Storage32.project] at en; grind
  have ne : p ≠ q := by intro eq; subst p; exact stable.2.1 held deleted
  refine ⟨stable.1,stable.2.1,?_⟩
  intro on same
  change p = q at same
  exact False.elim (ne same)

theorem pairSafety_flushDelete10 (c : Config P) (x : State P) (p : Fin P)
    (mutant : c.mutant = "none") (h : PairSafety10 x)
    (distinct : ∀ r ∈ x.g.renamed,r.1 ≠ r.2)
    (en : enabled c x (.prior (.flushDel p)) = true) :
    PairSafety10 (execute c x (.prior (.flushDel p))) := by
  intro r mem
  rcases h r mem with src | dst | pending
  · exact Or.inl (stable_flushDelete10 c x p r.1 src en)
  · exact Or.inr (Or.inl (stable_flushDelete10 c x p r.2 dst en))
  · right; right
    have dstNe : p ≠ r.2 := by
      have deleted : (x.s.pages p).buf = absent := by
        simp [enabled,Storage34.enabled,Storage32.enabled,Storage31.enabled,Storage3.enabled,
          project,Storage34.project,Storage32.project] at en; grind
      intro eq; subst p; exact pending.2.2.1 deleted
    have srcNe : p ≠ r.1 := by
      have ungated : Storage32.gated c x.s.to32 p = false := by
        simp only [enabled,Storage34.enabled,Storage32.enabled,Bool.and_eq_true] at en
        simpa [mutant,project,Storage34.project] using en.2
      intro eq
      have gated := pending_gated10 c x r.1 r.2 mutant pending (distinct r mem)
      rw [← eq,ungated] at gated
      cases gated
    refine ⟨pending.1,pending.2.1,pending.2.2.1,pending.2.2.2.1,?_,?_,pending.2.2.2.2.2.2⟩
    · intro _; exact srcNe
    · intro _ eq
      change p = r.2 at eq
      exact False.elim (dstNe eq)

-- Exact local physical-publication obligation. It is not assumed at a
-- reachable-state headline, and its reachable induction is still open.
def FileSurvivor10 (x : State P) : Prop := ∀ r ∈ x.g.renamed,
  (x.s.job.bytes ≠ absent ∧ (r.1 = x.s.job.p ∨ r.2 = x.s.job.p)) ∨
    (r.1 ≠ x.s.job.p ∧ x.s.disk r.1 ≠ absent) ∨
    (r.2 ≠ x.s.job.p ∧ x.s.disk r.2 ≠ absent)
theorem fileRename_resolves_iff10 (c : Config P) (x : State P) (mutant : c.mutant = "none") :
    renameResolves (execute c x (.prior (.old .rename))) = true ↔ FileSurvivor10 x := by
  simp only [renameResolves,Storage34.renameResolves,Storage33.renameResolves,
    Storage32.renameResolves,execute,Storage35Base.execute,Storage32.execute,
    Storage32.rename,mutant,show ("none" == "MPR") = false from rfl,
    Bool.false_eq_true,ite_false,Storage32.lift,Storage3.rename,Storage3.commit,
    Storage21.commit,project,Storage34.project,lift,Storage34.lift,Storage32.project,
    List.all_eq_true,FileSurvivor10,Bool.or_eq_true,bne_iff_ne]
  constructor
  · intro h r mem
    have raw := h r mem
    by_cases source : r.1 = x.s.job.p <;> by_cases destination : r.2 = x.s.job.p <;>
      simp [put,source,destination] at raw ⊢ <;> grind
  · intro h r mem
    rcases h r mem with bytes | source | destination
    · rcases bytes.2 with hs | hd
      · exact Or.inl (by simpa [put,hs] using bytes.1)
      · exact Or.inr (by simpa [put,hd] using bytes.1)
    · exact Or.inl (by simpa [put,source.1] using source.2)
    · exact Or.inr (by simpa [put,destination.1] using destination.2)
end Storage35
