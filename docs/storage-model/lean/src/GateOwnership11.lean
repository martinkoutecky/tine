import Safety35
namespace Storage32
open Storage21
set_option maxHeartbeats 4000000
set_option maxRecDepth 8192
set_option linter.unusedSimpArgs false

-- A gate keeps its entire original footprint until the gate retires.
-- Pending shrinks, but ownership is deliberately defined using vers.
def GateShape11 (gt : Gate P) : Prop :=
  gt.src ≠ gt.dst ∧ gt.vers gt.src ≠ noneBytes ∧ gt.vers gt.dst ≠ noneBytes ∧
  ∀ p ∈ gt.pending, gt.vers p ≠ noneBytes
def GateOwnership11 (gs : List (Gate P)) : Prop :=
  (∀ gt ∈ gs, GateShape11 gt) ∧
  ∀ gt ∈ gs, ∀ ht ∈ gs, ∀ p, gt.vers p ≠ noneBytes → ht.vers p ≠ noneBytes →
    gt.src = ht.src ∧ gt.dst = ht.dst
def GateDescends11 (ns os : List (Gate P)) : Prop :=
  ∀ ng ∈ ns, ∃ gt ∈ os, ng.src = gt.src ∧ ng.dst = gt.dst ∧ ng.vers = gt.vers ∧
    ∀ p ∈ ng.pending, p ∈ gt.pending
@[simp] theorem partner_up11 (sys : Sys P) (tail : List (Up P)) (k : Fin P) :
    partner {sys with up := tail} k = partner sys k := rfl

theorem ownership_empty11 : GateOwnership11 ([] : List (Gate P)) := by
  simp [GateOwnership11]
theorem descends_refl11 (gs : List (Gate P)) : GateDescends11 gs gs := by
  intro g hm; exact ⟨g,hm,rfl,rfl,rfl,fun _ hp => hp⟩
theorem ownership_descends11 (ns os : List (Gate P))
    (h : GateOwnership11 os) (d : GateDescends11 ns os) : GateOwnership11 ns := by
  constructor
  · intro ng hm
    obtain ⟨gt,gm,hs,hd,hv,hp⟩ := d ng hm
    obtain ⟨ne,sv,dv,pv⟩ := h.1 gt gm
    exact ⟨by simpa [hs,hd] using ne,by simpa [hv,hs] using sv,
      by simpa [hv,hd] using dv,by intro p pm; rw [hv]; exact pv p (hp p pm)⟩
  · intro ng nm nh hm p nv hv
    obtain ⟨gt,gm,gs,gd,gv,_⟩ := d ng nm
    obtain ⟨ht,km,ks,kd,kv,_⟩ := d nh hm
    have eq := h.2 gt gm ht km p (by simpa [gv] using nv) (by simpa [kv] using hv)
    exact ⟨gs.trans (eq.1.trans ks.symm),gd.trans (eq.2.trans kd.symm)⟩

theorem discharge_descends11 (sys : Sys P) (q : Fin P) (v : Int) :
    GateDescends11 (discharge sys q v).gates sys.gates := by
  intro ng hm
  obtain ⟨gt,gm,eq⟩ := List.mem_filterMap.mp hm
  simp only [discharge] at eq
  split at eq <;> (try split at eq)
  all_goals first
    | contradiction
    | (cases eq; exact ⟨_,gm,rfl,rfl,rfl,fun _ hp => hp⟩)
    | (cases eq; exact ⟨_,gm,rfl,rfl,rfl,fun _ hp => (List.mem_filter.mp hp).1⟩)

theorem discarded_descends11 (sys : Sys P) (q : Fin P) :
    GateDescends11 (discarded sys q).gates sys.gates := by
  intro ng hm
  obtain ⟨gt,gm,eq⟩ := List.mem_filterMap.mp hm
  simp only [discarded] at eq
  split at eq <;> (try split at eq)
  all_goals first
    | contradiction
    | (cases eq; exact ⟨_,gm,rfl,rfl,rfl,fun _ hp => hp⟩)
    | (cases eq; exact ⟨_,gm,rfl,rfl,rfl,fun _ hp => (List.mem_filter.mp hp).1⟩)

theorem ownership_install11 (x : State P) (src dst : Fin P) (ps : List (Fin P))
    (h : GateOwnership11 x.s.gates) (ne : src ≠ dst) (sm : src ∈ ps) (dm : dst ∈ ps)
    (nn : 0 ≤ x.g.vc) (free : touches x.s ps = false) :
    GateOwnership11 (x.s.gates ++ [makeGate x dst src ps]) := by
  have v (p : Fin P) (hp : p ∈ ps) : (makeGate x dst src ps).vers p ≠ noneBytes := by
    simp only [makeGate,show ps.contains p = true from by simpa using hp,ite_true]
    unfold noneBytes; omega
  have fresh : GateShape11 (makeGate x dst src ps) := ⟨ne,v src sm,v dst dm,v⟩
  have untouched (gt : Gate P) (gm : gt ∈ x.s.gates) (p : Fin P) (pm : p ∈ ps) :
      gt.vers p = noneBytes := by
    have no := List.any_eq_false.mp free gt gm
    have no' : (ps.any fun k => gt.vers k != noneBytes) = false := by simpa using no
    have atp := List.any_eq_false.mp no' p pm
    simpa using atp
  have footprint (p : Fin P) (hv : (makeGate x dst src ps).vers p ≠ noneBytes) : p ∈ ps := by
    by_cases hm : p ∈ ps
    · exact hm
    · simp [makeGate,hm] at hv
  constructor
  · intro gt gm
    rcases List.mem_append.mp gm with gm | gm
    · exact h.1 gt gm
    · have eq : gt = makeGate x dst src ps := by simpa using gm
      subst gt; exact fresh
  · intro gt gm ht hm p gv hv
    rcases List.mem_append.mp gm with gm | gm <;> rcases List.mem_append.mp hm with hm | hm
    · exact h.2 gt gm ht hm p gv hv
    · have eq : ht = makeGate x dst src ps := by simpa using hm
      subst ht; exact False.elim (gv (untouched gt gm p (footprint p hv)))
    · have eq : gt = makeGate x dst src ps := by simpa using gm
      subst gt; exact False.elim (hv (untouched ht hm p (footprint p gv)))
    · have ge : gt = makeGate x dst src ps := by simpa using gm
      have he : ht = makeGate x dst src ps := by simpa using hm
      simp [ge,he]

theorem partner_some_fold11 (gs : List (Gate P)) (k p : Fin P) (acc : Option (Fin P))
    (h : gs.foldl (fun acc gt => if acc.isNone && gt.dst == k && gt.pending.contains k
      then some gt.src else acc) acc = some p) :
    acc = some p ∨ ∃ gt ∈ gs, gt.dst = k ∧ k ∈ gt.pending ∧ gt.src = p := by
  induction gs generalizing acc with
  | nil => exact Or.inl h
  | cons gt gs ih =>
    simp only [List.foldl_cons] at h
    rcases ih _ h with found | ⟨ht,hm,hd,hp,hs⟩
    · split at found
      · have cond : (acc.isNone && gt.dst == k && gt.pending.contains k) = true := by assumption
        have parts : gt.dst = k ∧ k ∈ gt.pending := by
          simp only [Bool.and_eq_true,beq_iff_eq,List.contains_iff_mem] at cond
          exact ⟨cond.1.2,cond.2⟩
        have eq : gt.src = p := Option.some.inj found
        exact Or.inr ⟨gt,by simp,parts.1,parts.2,eq⟩
      · exact Or.inl found
    · exact Or.inr ⟨ht,List.mem_cons_of_mem _ hm,hd,hp,hs⟩

theorem partner_some11 (sys : Sys P) (k p : Fin P) (h : partner sys k = some p) :
    ∃ gt ∈ sys.gates, gt.dst = k ∧ k ∈ gt.pending ∧ gt.src = p := by
  rcases partner_some_fold11 sys.gates k p none h with impossible | found
  · cases impossible
  · exact found

theorem partner_fold_fixed11 (gs : List (Gate P)) (k p : Fin P) :
    gs.foldl (fun acc gt => if acc.isNone && gt.dst == k && gt.pending.contains k
      then some gt.src else acc) (some p) = some p := by
  induction gs with
  | nil => rfl
  | cons gt gs ih => simpa using ih

theorem partner_unique_fold11 (gs : List (Gate P)) (k p : Fin P)
    (all : ∀ gt ∈ gs, gt.dst = k → k ∈ gt.pending → gt.src = p)
    (existsGate : ∃ gt ∈ gs, gt.dst = k ∧ k ∈ gt.pending) :
    gs.foldl (fun acc gt => if acc.isNone && gt.dst == k && gt.pending.contains k
      then some gt.src else acc) none = some p := by
  induction gs with
  | nil => simpa using existsGate
  | cons gt gs ih =>
    by_cases head : gt.dst = k ∧ k ∈ gt.pending
    · simp only [List.foldl_cons]
      have eq := all gt (by simp) head.1 head.2
      simpa [head.1,head.2,eq] using partner_fold_fixed11 gs k p
    · have tail : ∃ ht ∈ gs, ht.dst = k ∧ k ∈ ht.pending := by
        obtain ⟨ht,hm,hd,hp⟩ := existsGate
        rcases List.mem_cons.mp hm with eq | mem
        · subst ht; exact False.elim (head ⟨hd,hp⟩)
        · exact ⟨ht,mem,hd,hp⟩
      have old := ih (by intro ht hm; exact all ht (List.mem_cons_of_mem _ hm)) tail
      simpa [head] using old

theorem partner_owned11 (sys : Sys P) (gt : Gate P) (hm : gt ∈ sys.gates)
    (h : GateOwnership11 sys.gates) (dm : gt.dst ∈ gt.pending) :
    partner sys gt.dst = some gt.src := by
  apply partner_unique_fold11
  · intro ht mem hd _
    have eq := h.2 ht mem gt hm gt.dst
      (by simpa [← hd] using (h.1 ht mem).2.2.1) (h.1 gt hm).2.2.1
    exact eq.1
  · exact ⟨gt,hm,rfl,dm⟩

theorem partner_into_owned11 (sys : Sys P) (gt : Gate P) (hm : gt ∈ sys.gates)
    (h : GateOwnership11 sys.gates) (k p : Fin P) (hp : partner sys k = some p)
    (touch : gt.vers p ≠ noneBytes) : k = gt.dst ∧ p = gt.src := by
  obtain ⟨ht,mem,hd,_,hs⟩ := partner_some11 sys k p hp
  have eq := h.2 ht mem gt hm p (by simpa [← hs] using (h.1 ht mem).2.1) touch
  exact ⟨hd.symm.trans eq.2,hs.symm.trans eq.1⟩

theorem ownership_old11 (c : Config P) (x : State P) (a : Storage21.Action P)
    (mutant : c.mutant = "none") (h : GateOwnership11 x.s.gates) :
    GateOwnership11 (execute c x (.old a)).s.gates := by
  cases a <;> simp only [execute]
  case deliverUp ok =>
    cases hq : x.s.up with
    | nil => simpa [deliverUp,hq] using h
    | cons m tail =>
      simp only [deliverUp,hq]
      split
      · simp only [upDiscard]; split <;> (try split)
        · exact h
        · exact h
        · exact ownership_descends11 _ _ h (discarded_descends11 _ _)
      · exact h
  case dirSync ok =>
    cases ok
    · simpa [dirSync,mutant,lift] using h
    · simpa [dirSync,mutant,lift,discharge] using ownership_descends11 _ _ h (discharge_descends11 _ _ _)
  case check => simpa [check,mutant,lift] using h
  case rename => simpa [rename,mutant,lift] using h
  case saveFail => simpa [saveFail,mutant,lift] using h
  case crash => exact ownership_empty11
  case switchFin => exact ownership_empty11
  case power keep => exact ownership_empty11
  all_goals exact h

theorem ownership_execute11 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (h : GateOwnership11 x.s.gates)
    (nn : 0 ≤ x.g.vc) (en : enabled c x a = true) :
    GateOwnership11 (execute c x a).s.gates := by
  cases a with
  | old a => exact ownership_old11 c x a mutant h
  | load p => exact h
  | flushDel p => exact h
  | opDelete p => exact h
  | opRename src dst refs rt =>
    by_cases full : Storage3.opCur x.s.toBase src ≠ absent
    · have old := enabled_projects9 c x (.opRename src dst refs rt) en
      have ne : src ≠ dst := by simp [Storage31.enabled,Storage3.enabled] at old; grind
      have no : touches x.s (union refs [src,dst]) = false := by
        have raw := en
        simp only [enabled,Bool.and_eq_true] at raw
        simpa using raw.2
      have nf : touches x.s (union [src,dst] refs) = false := by
        apply List.any_eq_false.mpr
        intro gt gm
        apply Bool.eq_false_iff.mp
        apply List.any_eq_false.mpr
        intro p pm
        have pm' : p ∈ union refs [src,dst] := by
          simpa only [mem_union_general,or_comm] using pm
        have no' : ((union refs [src,dst]).any fun k => gt.vers k != noneBytes) = false := by
          simpa using List.any_eq_false.mp no gt gm
        exact List.any_eq_false.mp no' p pm'
      simpa [execute,opRename,full,lift] using ownership_install11 x src dst (union [src,dst] refs) h ne
        (by simp [mem_union_general]) (by simp [mem_union_general]) nn nf
    · simpa [execute,opRename,full,lift] using h
end Storage32

namespace Storage33
open Storage21 Storage32
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false
theorem ownership_apply11 (c : Config P) (x : State P) (eff plan : Map P) (dst src : Option (Fin P))
    (mutant : c.mutant = "none") (h : GateOwnership11 x.s.gates) (nn : 0 ≤ x.g.vc)
    (en : enabledApply c x eff plan dst src = true) :
    GateOwnership11 (opApply c x eff plan dst src).s.gates := by
  cases dst with
  | none => exact h
  | some d =>
    cases src with
    | none => simp [enabledApply] at en
    | some s =>
      have ds : d ≠ s := by simp [enabledApply] at en; grind
      have dm : d ∈ keys eff := by simp [enabledApply] at en; grind
      have sm : s ∈ keys eff := by simp [enabledApply] at en; grind
      have no : touches x.s (keys eff) = false := by simp [enabledApply] at en; grind
      simpa [opApply,mutant,Storage32.commit] using ownership_install11 x s d (keys eff) h (Ne.symm ds) sm dm nn no
end Storage33

namespace Storage35Base
open Storage21 Storage32
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false
theorem ownership_discard11 (c : Config P) (x : State P) (m : Up P) (tail : List (Up P))
    (ok : Bool) (h : GateOwnership11 x.s.gates) :
    GateOwnership11 (upDiscard c x {x.s with up := tail} m ok).s.gates := by
  simp only [upDiscard]; split <;> (try split)
  · exact h
  · exact h
  · exact ownership_descends11 _ _ h (discarded_descends11 _ _)
theorem ownership_execute11 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (h : GateOwnership11 x.s.gates) (nn : 0 ≤ x.g.vc)
    (en : Storage32.enabled c x a = true) :
    GateOwnership11 (execute c x a).s.gates := by
  rcases execute_cases10 c x a with same | ⟨ok,m,tail,_,_,same⟩
  · rw [same]; exact Storage32.ownership_execute11 c x a mutant h nn en
  · rw [same]; exact ownership_discard11 c x m tail ok h
end Storage35Base

namespace Storage35
open Storage21 Storage32
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false
theorem gateOwnership_reachable35_11 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : Reachable c disk x) : GateOwnership11 x.s.gates := by
  induction hr with
  | initial => exact ownership_empty11
  | @advance x y hr a hs ih =>
    have nn := (aux_reachable10 c disk x mutant hr).bounds.nonnegative
    unfold step at hs; split at hs
    · cases hs
      have en : enabled c x a = true := by assumption
      cases a with
      | prior a => exact Storage35Base.ownership_execute11 c (project x) a mutant ih nn en
      | opApply eff plan dst src =>
        have raw : Storage33.enabledApply c (project x) eff plan dst src = true := by
          simp only [enabled,Storage34.enabled,Storage34.enabledApply,Bool.and_eq_true] at en
          exact en.1
        exact Storage33.ownership_apply11 c (project x) eff plan dst src mutant ih nn raw
      | extClaim p present => exact ih
    · contradiction
end Storage35
