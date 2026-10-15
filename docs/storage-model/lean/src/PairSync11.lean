import PairJobs11
namespace Storage35
open Storage21 Storage32
set_option maxHeartbeats 4000000
set_option maxRecDepth 8192
set_option linter.unusedSimpArgs false
def syncView11 (c : Config P) (x : State P) (ok : Bool) : State P :=
  {x with s := {x.s with pages := put x.s.pages x.s.job.p (onReply c (x.s.pages x.s.job.p) (if ok then "Published" else "Uncertain") x.s.job.bytes),job := nojob c,gates := if ok then (discharge x.s.to32 x.s.job.p x.s.job.ver).gates else x.s.gates}}

theorem pairSafety_syncFalse11 (c : Config P) (x : State P)
    (mutant : c.mutant = "none") (h : PairSafety10 x) : PairSafety10 (syncView11 c x false) := by
  apply pairSafety_frame11 x (syncView11 c x false) h rfl (fun _ hm => hm)
  · apply pageSafe_put11
    · exact fun p => pageSafe_refl11 _ _
    · exact pageSafe_reply11 c _ _ _ "Uncertain" mutant (by decide)
  · exact fun _ hm => hm
  · intro on; cases on

theorem stable_syncTrue11 (c : Config P) (x : State P) (p : Fin P) (h : StableSide10 x p) :
    StableSide10 (syncView11 c x true) p := by
  refine ⟨h.1,?_,?_⟩
  · by_cases eq : p = x.s.job.p
    · subst p; simpa [syncView11,put,onReply] using h.2.1
    · simpa [syncView11,put,eq] using h.2.1
  · intro on; cases on

theorem pending_syncOther11 (c : Config P) (x : State P) (src dst : Fin P)
    (h : PendingPair10 x src dst) (ns : src ≠ x.s.job.p) (nd : dst ≠ x.s.job.p) :
    PendingPair10 (syncView11 c x true) src dst := by
  obtain ⟨gt,gm,gs,gd,sm,dm⟩ := h.2.2.2.2.2.2
  let ng := if gt.vers x.s.job.p != noneBytes && x.s.job.ver >= gt.vers x.s.job.p
    then {gt with pending := exclude gt.pending [x.s.job.p]} else gt
  have nsm : src ∈ ng.pending := by dsimp [ng]; split <;> simp_all [exclude]
  have ndm : dst ∈ ng.pending := by dsimp [ng]; split <;> simp_all [exclude]
  have nonempty : ng.pending.isEmpty = false := by cases hp : ng.pending <;> simp_all
  have ngmem : ng ∈ (discharge x.s.to32 x.s.job.p x.s.job.ver).gates := by
    apply List.mem_filterMap.mpr
    refine ⟨gt,gm,?_⟩
    change (if ng.pending.isEmpty then none else some ng) = some ng
    simp [nonempty]
  refine ⟨h.1,?_,?_,?_,?_,?_,ng,ngmem,?_,?_,nsm,ndm⟩
  · simpa [syncView11,put,nd] using h.2.1
  · simpa [syncView11,put,nd] using h.2.2.1
  · simpa [syncView11,put,nd] using h.2.2.2.1
  · intro on; cases on
  · intro on; cases on
  · dsimp [ng]; split <;> exact gs
  · dsimp [ng]; split <;> exact gd

theorem pairSafety_syncTrue11 (c : Config P) (x : State P) (h : PairSafety10 x)
    (published : PairPublished11 (project x)) (on : x.s.job.on = true) (phase : x.s.job.phase = 3) :
    PairSafety10 (syncView11 c x true) := by
  intro r mem
  rcases h r mem with src | dst | pending
  · exact Or.inl (stable_syncTrue11 c x r.1 src)
  · exact Or.inr (Or.inl (stable_syncTrue11 c x r.2 dst))
  · have ns : r.1 ≠ x.s.job.p := Ne.symm (pending.2.2.2.2.1 on)
    by_cases nd : r.2 = x.s.job.p
    · right; left
      have eq := published r mem on phase (Or.inr nd.symm)
      change x.s.disk x.s.job.p = x.s.job.bytes at eq
      have disk : x.s.disk r.2 ≠ absent := by
        rw [nd]
        rw [eq]
        exact pending.2.2.2.2.2.1 on nd.symm
      refine ⟨disk,?_,?_⟩
      · simpa [syncView11,put,nd,onReply] using (show (x.s.pages x.s.job.p).held = true → (x.s.pages x.s.job.p).buf ≠ absent from fun _ => nd ▸ pending.2.2.1)
      · intro on; cases on
    · right; right
      exact pending_syncOther11 c x r.1 r.2 pending ns nd

theorem pairSafety_dirSync11 (c : Config P) (x : State P) (ok : Bool)
    (mutant : c.mutant = "none") (h : PairSafety10 x) (published : PairPublished11 (project x))
    (en : enabled c x (.prior (.old (.dirSync ok))) = true) :
    PairSafety10 (execute c x (.prior (.old (.dirSync ok)))) := by
  have on : x.s.job.on = true := by
    simp [enabled,Storage34.enabled,Storage32.enabled,Storage31.enabled,Storage3.enabled,
      Storage21.enabled,project,Storage34.project,Storage32.project,Storage3.project] at en; grind
  have phase : x.s.job.phase = 3 := by
    simp [enabled,Storage34.enabled,Storage32.enabled,Storage31.enabled,Storage3.enabled,
      Storage21.enabled,project,Storage34.project,Storage32.project,Storage3.project] at en; grind
  apply (pairSafety_view11 _ (syncView11 c x ok) ?_).mpr
    (by cases ok; exact pairSafety_syncFalse11 c x mutant h; exact pairSafety_syncTrue11 c x h published on phase)
  cases ok <;> simp [PairViewEq11,syncView11,execute,Storage35Base.execute,Storage32.execute,
    Storage32.dirSync,Storage32.lift,Storage32.discharge,Storage3.dirSync,Storage3.lift,
    Storage21.dirSync,Storage21.commit,project,Storage34.project,Storage32.project,Storage3.project,
    Storage34.lift,mutant]
end Storage35
