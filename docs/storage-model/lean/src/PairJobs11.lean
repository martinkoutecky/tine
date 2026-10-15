import PairBackend11
namespace Storage35
open Storage21 Storage32
set_option maxHeartbeats 4000000
set_option maxRecDepth 8192
set_option linter.unusedSimpArgs false
def PairViewEq11 (x y : State P) : Prop :=
  x.s.disk = y.s.disk ∧ x.s.pages = y.s.pages ∧ x.s.gates = y.s.gates ∧
  x.g.renamed = y.g.renamed ∧ x.s.job.on = y.s.job.on ∧ x.s.job.p = y.s.job.p ∧ x.s.job.bytes = y.s.job.bytes
theorem pairSafety_view11 (x y : State P) (h : PairViewEq11 x y) : PairSafety10 x ↔ PairSafety10 y := by
  simp only [PairSafety10,StableSide10,PendingPair10,h.1,h.2.1,h.2.2.1,h.2.2.2.1,
    h.2.2.2.2.1,h.2.2.2.2.2.1,h.2.2.2.2.2.2]

def fileView11 (x : State P) : State P :=
  {x with s := {x.s with disk := put x.s.disk x.s.job.p x.s.job.bytes}}
theorem pairSafety_fileView11 (x : State P) (h : PairSafety10 x) (on : x.s.job.on = true) :
    PairSafety10 (fileView11 x) := by
  have stable (p : Fin P) (hp : StableSide10 x p) : StableSide10 (fileView11 x) p := by
    refine ⟨?_,hp.2.1,hp.2.2⟩
    by_cases eq : p = x.s.job.p
    · simpa [fileView11,put,eq] using hp.2.2 on eq.symm
    · simpa [fileView11,put,eq] using hp.1
  intro r mem
  rcases h r mem with src | dst | pending
  · exact Or.inl (stable r.1 src)
  · exact Or.inr (Or.inl (stable r.2 dst))
  · right; right
    have ne : r.1 ≠ x.s.job.p := Ne.symm (pending.2.2.2.2.1 on)
    exact ⟨by simpa [fileView11,put,ne] using pending.1,pending.2⟩

theorem pairSafety_fileRename11 (c : Config P) (x : State P) (mutant : c.mutant = "none")
    (h : PairSafety10 x) (on : x.s.job.on = true) :
    PairSafety10 (execute c x (.prior (.old .rename))) := by
  apply (pairSafety_view11 _ (fileView11 x) ?_).mpr (pairSafety_fileView11 x h on)
  simp [PairViewEq11,execute,Storage35Base.execute,Storage32.execute,Storage32.rename,mutant,
    Storage32.lift,Storage3.rename,Storage3.commit,Storage21.commit,project,Storage34.project,
    Storage34.lift,fileView11,Storage32.project]

def flushView11 (c : Config P) (x : State P) (p : Fin P) : State P :=
  {x with s := {x.s with job := ⟨true,p,(x.s.pages p).buf,(x.s.pages p).base,(x.s.pages p).ver,1,0⟩}}
theorem pairSafety_flushView11 (c : Config P) (x : State P) (p : Fin P)
    (mutant : c.mutant = "none") (h : PairSafety10 x) (own : GateOwnership11 x.s.gates)
    (held : (x.s.pages p).held = true) (gate : gated c x.s.to32 p = false) :
    PairSafety10 (flushView11 c x p) := by
  have stable (q : Fin P) (hp : StableSide10 x q) : StableSide10 (flushView11 c x p) q := by
    refine ⟨hp.1,hp.2.1,?_⟩
    intro _ eq
    change p = q at eq
    subst q
    exact hp.2.1 held
  intro r mem
  rcases h r mem with src | dst | pending
  · exact Or.inl (stable r.1 src)
  · exact Or.inr (Or.inl (stable r.2 dst))
  · right; right
    have ne : p ≠ r.1 := by
      intro eq
      obtain ⟨gt,gm,gs,gd,_,_⟩ := pending.2.2.2.2.2.2
      have distinct : r.1 ≠ r.2 := by simpa [gs,gd] using (own.1 gt gm).1
      have yes := pending_gated10 c x r.1 r.2 mutant pending distinct
      rw [← eq,gate] at yes
      cases yes
    refine ⟨pending.1,pending.2.1,pending.2.2.1,pending.2.2.2.1,?_,?_,pending.2.2.2.2.2.2⟩
    · exact fun _ => ne
    · intro _ eq
      change p = r.2 at eq
      simpa [flushView11,eq] using pending.2.2.1

theorem pairSafety_flush11 (c : Config P) (x : State P) (p : Fin P) (deletion : Bool)
    (mutant : c.mutant = "none") (h : PairSafety10 x) (own : GateOwnership11 x.s.gates)
    (en : enabled c x (if deletion then .prior (.flushDel p) else .prior (.old (.flush p))) = true) :
    PairSafety10 (execute c x (if deletion then .prior (.flushDel p) else .prior (.old (.flush p)))) := by
  have held : (x.s.pages p).held = true := by
    cases deletion <;> simp [enabled,Storage34.enabled,Storage32.enabled,Storage31.enabled,Storage3.enabled,
      Storage21.enabled,project,Storage34.project,Storage32.project,Storage3.project] at en <;> grind
  have gate : gated c x.s.to32 p = false := by
    cases deletion <;> simp [enabled,Storage34.enabled,Storage32.enabled,mutant,project,Storage34.project] at en <;> grind
  apply (pairSafety_view11 _ (flushView11 c x p) ?_).mpr
    (pairSafety_flushView11 c x p mutant h own held gate)
  cases deletion <;> exact ⟨rfl,rfl,rfl,rfl,rfl,rfl,rfl⟩
end Storage35
