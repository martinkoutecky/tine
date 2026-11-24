import Stage6PairView

namespace Stage6
open Storage21

def PendingMove (gates : List (Gate P)) (src dst : Slot P) : Prop :=
  ∃ gt ∈ gates, ⟨dst, src⟩ ∈ gt.moves ∧ src ∈ gt.pending ∧ dst ∈ gt.pending

theorem pendingMove_iff (x : State P) (src dst : Slot P) :
    PendingMove x.gates src dst ↔
      ∃ g ∈ x.pairView.s.gates, g.src = src ∧ g.dst = dst ∧ src ∈ g.pending ∧ dst ∈ g.pending := by
  constructor
  · rintro ⟨gt, gm, mm, sm, dm⟩
    exact ⟨pairGate gt ⟨dst, src⟩, pairGate_mem.mpr ⟨gt, gm, _, mm, rfl⟩, rfl, rfl,
      pairGate_pending.mpr ⟨sm, Or.inl rfl⟩, pairGate_pending.mpr ⟨dm, Or.inr rfl⟩⟩
  · rintro ⟨g, gm, hs, hd, sm, dm⟩
    obtain ⟨gt, gtmem, ⟨d, s⟩, mm, rfl⟩ := pairGate_mem.mp gm
    change s = src at hs
    change d = dst at hd
    subst s; subst d
    exact ⟨gt, gtmem, mm, (pairGate_pending.mp sm).1, (pairGate_pending.mp dm).1⟩

private theorem pending_original {x : State P} {g : Storage32.Gate P} (gm : g ∈ x.pairView.s.gates)
    (sm : g.src ∈ g.pending) (dm : g.dst ∈ g.pending) : PendingMove x.gates g.src g.dst :=
  (pendingMove_iff x g.src g.dst).mpr ⟨g, gm, rfl, rfl, sm, dm⟩

private theorem pending_published (x : State P) (q : Slot P) (v : Int) (own : GateOwnership x.gates)
    (g : Storage32.Gate P) (gm : g ∈ (Storage32.discharge x.pairView.s.to32 q v).gates)
    (sm : g.src ∈ g.pending) (dm : g.dst ∈ g.pending) :
    PendingMove (published x.gates q v) g.src g.dst := by
  obtain ⟨old, oldmem, update⟩ := List.mem_filterMap.mp gm
  obtain ⟨gt, gtmem, m, mm, rfl⟩ := pairGate_mem.mp oldmem
  let lg := pairGate gt m
  let lg' := if lg.vers q != noneBytes && v ≥ lg.vers q
    then {lg with pending := exclude lg.pending [q]} else lg
  have eq : lg' = g := by
    change (if lg'.pending.isEmpty then none else some lg') = some g at update
    split at update <;> simp_all
  subst g
  have src : lg'.src = m.src := by dsimp only [lg']; split <;> rfl
  have dst : lg'.dst = m.dst := by dsimp only [lg']; split <;> rfl
  rw [src] at sm ⊢
  rw [dst] at dm ⊢
  let ht := if (gt.vers q).any (fun wanted => v ≥ wanted)
    then {gt with pending := exclude gt.pending [q]} else gt
  have stay (p : Slot P) (ep : p = m.src ∨ p = m.dst) (hp : p ∈ lg'.pending) : p ∈ ht.pending := by
    have before : p ∈ lg.pending := by
      dsimp only [lg'] at hp
      split at hp
      · exact (mem_exclude.mp hp).1
      · exact hp
    have old := (pairGate_pending.mp before).1
    dsimp only [ht]
    split
    · rename_i ge
      apply mem_exclude.mpr
      refine ⟨old, ?_⟩
      intro mem
      have pq : p = q := by simpa using mem
      subst q
      obtain ⟨w, ver, ne⟩ := endpoint_version (own.1 gt gtmem) mm ep
      have le : w ≤ v := by simpa [ver] using ge
      have de : lg.vers p = w := by simp [lg, pairGate_vers ep, ver]
      simp [lg', de, ne, le, exclude] at hp
    · exact old
  have sm' := stay m.src (Or.inl rfl) sm
  have dm' := stay m.dst (Or.inr rfl) dm
  have mm' : m ∈ ht.moves := by dsimp only [ht]; split <;> exact mm
  have live := retained_some (gt := ht) (by intro h; simp [h] at mm') (by intro h; simp [h] at sm')
  exact ⟨ht, List.mem_filterMap.mpr ⟨gt, gtmem, live⟩, mm', sm', dm'⟩

private theorem pending_discarded (x : State P) (p : Slot P) (own : GateOwnership x.gates)
    (g : Storage32.Gate P) (gm : g ∈ (Storage32.discarded x.pairView.s.to32 p).gates)
    (sm : g.src ∈ g.pending) (dm : g.dst ∈ g.pending) :
    PendingMove (discarded x.gates p) g.src g.dst := by
  obtain ⟨old, oldmem, update⟩ := List.mem_filterMap.mp gm
  obtain ⟨gt, gtmem, m, mm, rfl⟩ := pairGate_mem.mp oldmem
  let lg := pairGate gt m
  let lg' := if lg.dst == p then lg else {lg with pending := exclude lg.pending [p]}
  change (if (lg.dst == p && lg.pending.contains p) || lg'.pending.isEmpty then none else some lg') =
    some g at update
  have kept : ((lg.dst == p && lg.pending.contains p) || lg'.pending.isEmpty) = false := by
    split at update
    · contradiction
    · exact Bool.eq_false_iff.mpr ‹¬_ = true›
  have eq : lg' = g := by
    rw [kept, if_neg Bool.false_ne_true] at update
    exact Option.some.inj update
  subst g
  have src : lg'.src = m.src := by dsimp only [lg']; split <;> rfl
  have dst : lg'.dst = m.dst := by dsimp only [lg']; split <;> rfl
  rw [src] at sm ⊢
  rw [dst] at dm ⊢
  have dp : m.dst ≠ p := by
    intro mp
    have de : lg.dst = p := mp
    have mem : m.dst ∈ lg.pending := by simpa [lg', de] using dm
    have pending : p ∈ lg.pending := mp ▸ mem
    simp [de, pending] at kept
  have off : (lg.dst == p) = false := Bool.eq_false_iff.mpr (fun h => dp (beq_iff_eq.mp h))
  have sm0 : m.src ∈ exclude lg.pending [p] := by simpa [lg', off] using sm
  have dm0 : m.dst ∈ exclude lg.pending [p] := by simpa [lg', off] using dm
  let gone := gt.moves.filter fun n => n.dst == p
  have noPartner (q : Slot P) (ep : q = m.src ∨ q = m.dst) : q ∉ gone.map Move.src := by
    intro h
    obtain ⟨n, nm', ns⟩ := List.mem_map.mp h
    have ⟨nm, nd⟩ := List.mem_filter.mp nm'
    have eq := endpoint_unique (own.1 gt gtmem).1 mm nm ep.symm (Or.inr ns.symm)
    subst n
    exact dp (beq_iff_eq.mp nd)
  let ht := if gt.pending.contains p then
    {gt with moves := gt.moves.filter (fun n => n.dst != p),
             pending := exclude gt.pending (p :: gone.map Move.src)}
    else gt
  have keep (q : Slot P) (ep : q = m.src ∨ q = m.dst) (qp : q ∈ exclude lg.pending [p]) :
      q ∈ ht.pending := by
    obtain ⟨old, ne⟩ := mem_exclude.mp qp
    have mem := (pairGate_pending.mp old).1
    dsimp only [ht]
    split
    · exact mem_exclude.mpr ⟨mem, by simpa using And.intro (show q ≠ p by simpa using ne) (noPartner q ep)⟩
    · exact mem
  have sm' := keep m.src (Or.inl rfl) sm0
  have dm' := keep m.dst (Or.inr rfl) dm0
  have mm' : m ∈ ht.moves := by
    dsimp only [ht]
    split
    · simp [mm, dp]
    · exact mm
  have live := retained_some (gt := ht) (by intro h; simp [h] at mm') (by intro h; simp [h] at sm')
  refine ⟨ht, List.mem_filterMap.mpr ⟨gt, gtmem, ?_⟩, mm', sm', dm'⟩
  by_cases yes : p ∈ gt.pending <;> simpa [discarded, ht, gone, yes] using live

theorem coreAndPairs_eq (c : Config P) (x : State P) (a : Storage32.Action P)
    (normal : c.mutant = "none") :
    let y := Storage35.execute c x.pairView (.prior a)
    let z := execute c x (.prior a)
    Storage32.project (Storage35.project y) = z.core ∧ y.g.renamed = z.renamed := by
  cases a <;> try exact ⟨rfl, rfl⟩
  case old a =>
    cases a <;> try exact ⟨rfl, rfl⟩
    case dirSync ok => cases ok <;>
      simp [Storage35.execute, Storage35Base.execute, Storage32.execute, Storage32.dirSync,
        Storage32.discharge, Storage32.lift, Storage32.project, Storage34.project, Storage34.lift,
        State.pairView, State.legacyView, execute, State.fromCore, normal]
    case rename =>
      simp [Storage35.execute, Storage35Base.execute, Storage32.execute, Storage32.rename,
        Storage32.lift, Storage32.project, Storage34.project, Storage34.lift, State.pairView,
        State.legacyView, execute, State.fromCore, normal]
    case check =>
      simp [Storage35.execute, Storage35Base.execute, Storage32.execute, Storage32.check,
        Storage32.lift, Storage32.project, Storage34.project, Storage34.lift, State.pairView,
        State.legacyView, execute, State.fromCore, normal]
    case saveFail =>
      simp [Storage35.execute, Storage35Base.execute, Storage32.execute, Storage32.saveFail,
        Storage32.lift, Storage32.project, Storage34.project, Storage34.lift, State.pairView,
        State.legacyView, execute, State.fromCore, normal]
    case deliverUp ok =>
      let z := Storage35.project x.legacyView
      let y := Storage35.project x.pairView
      have partner (p : Slot P) : Storage32.partner y.s p = Storage32.partner z.s p := pairView_partner x p
      change Storage32.project (Storage35Base.deliverUp c y ok) =
        Storage32.project (Storage35Base.deliverUp c z ok) ∧
        (Storage35Base.deliverUp c y ok).g.renamed = (Storage35Base.deliverUp c z ok).g.renamed
      have qY : y.s.up = x.core.s.up := rfl
      have qZ : z.s.up = x.core.s.up := rfl
      cases q : x.core.s.up with
      | nil => simp only [Storage35Base.deliverUp, qY, qZ, q]; exact ⟨rfl, rfl⟩
      | cons m tail =>
        simp only [Storage35Base.deliverUp, qY, qZ, q]
        by_cases kind : m.kind = "discard"
        · have yh : (y.s.pages m.p).held = (x.core.s.pages m.p).held := rfl
          have zh : (z.s.pages m.p).held = (x.core.s.pages m.p).held := rfl
          cases ok <;> cases held : (x.core.s.pages m.p).held <;> cases m.cur <;>
            simp only [kind, beq_self_eq_true, if_true, Storage35Base.upDiscard, Storage32.partner_up11,
              partner, yh, zh, held, Bool.not_true, Bool.not_false, Bool.false_eq_true, if_false,
              Storage32.project, Storage32.commit, Storage32.discarded] <;>
            exact ⟨rfl, rfl⟩
        · have miss : (m.kind == "discard") = false := by simpa using kind
          simp only [miss, Bool.false_eq_true, if_false]
          exact ⟨rfl, rfl⟩

private theorem pending_prior (c : Config P) (x : State P) (a : Storage32.Action P)
    (normal : c.mutant = "none") (own : GateOwnership x.gates)
    (g : Storage32.Gate P) (gm : g ∈ (Storage35.execute c x.pairView (.prior a)).s.gates)
    (sm : g.src ∈ g.pending) (dm : g.dst ∈ g.pending) :
    PendingMove (priorGates x a) g.src g.dst := by
  cases a <;> try exact pending_original gm sm dm
  case opRename src dst refs rt =>
    have full : (Storage3.opCur x.core.s src != absent) = true ∨
        (Storage3.opCur x.core.s src != absent) = false := by
      cases h : Storage3.opCur x.core.s src != absent <;> simp
    have raw : g ∈ (if Storage3.opCur x.core.s src != absent then
        x.pairView.s.gates ++ [Storage32.makeGate (Storage35.project x.pairView) dst src (union [src, dst] refs)]
        else x.pairView.s.gates) := by
      change g ∈ (if Storage3.opCur x.core.s src != absent then
        x.pairView.s.gates ++ [Storage32.makeGate (Storage35.project x.pairView) dst src
          (union (if Storage3.opCur x.core.s src != absent then [src, dst] else []) refs)]
        else x.pairView.s.gates) at gm
      rcases full with h | h <;> simpa [h] using gm
    rcases full with h | h
    · simp only [h, if_true, List.mem_append, List.mem_singleton] at raw
      change PendingMove (if Storage3.opCur x.core.s src != absent then
        install x [⟨dst, src⟩] (union [src, dst] refs) else x.gates) g.src g.dst
      simp only [h, if_true]
      let gt := newGate x [⟨dst, src⟩] (union [src, dst] refs)
      change PendingMove (x.gates ++ [gt]) g.src g.dst
      rcases raw with old | rfl
      · obtain ⟨ht, hm, rest⟩ := pending_original old sm dm
        exact ⟨ht, List.mem_append_left _ hm, rest⟩
      · refine ⟨gt, by simp, by simp [gt, newGate, Storage32.makeGate], ?_, ?_⟩
        · apply (mem_union_general _ _ _).mpr
          exact Or.inl (by simp [Storage32.makeGate])
        · apply (mem_union_general _ _ _).mpr
          exact Or.inl (by simp [Storage32.makeGate])
    · have old : g ∈ x.pairView.s.gates := by simpa [h] using raw
      simpa [priorGates, h] using pending_original old sm dm
  case old a =>
    cases a <;> try exact pending_original gm sm dm
    case crash => cases gm
    case power keep => cases gm
    case switchFin => cases gm
    case rename =>
      have old : g ∈ x.pairView.s.gates := by
        simpa [Storage35.execute, Storage35Base.execute, Storage32.execute, Storage32.rename,
          Storage32.lift, Storage34.lift, Storage34.project, normal] using gm
      exact pending_original old sm dm
    case check =>
      have old : g ∈ x.pairView.s.gates := by
        simpa [Storage35.execute, Storage35Base.execute, Storage32.execute, Storage32.check,
          Storage32.lift, Storage34.lift, Storage34.project, normal] using gm
      exact pending_original old sm dm
    case saveFail =>
      have old : g ∈ x.pairView.s.gates := by
        simpa [Storage35.execute, Storage35Base.execute, Storage32.execute, Storage32.saveFail,
          Storage32.lift, Storage34.lift, Storage34.project, normal] using gm
      exact pending_original old sm dm
    case dirSync ok =>
      cases ok
      · have old : g ∈ x.pairView.s.gates := by
          simpa [Storage35.execute, Storage35Base.execute, Storage32.execute, Storage32.dirSync,
            Storage32.lift, Storage34.lift, Storage34.project, normal] using gm
        exact pending_original old sm dm
      · have old : g ∈ (Storage32.discharge x.pairView.s.to32 x.core.s.job.p x.core.s.job.ver).gates := by
          simpa [Storage35.execute, Storage35Base.execute, Storage32.execute, Storage32.dirSync,
            Storage32.discharge, Storage32.lift, Storage34.lift, Storage34.project, normal,
            State.pairView, State.legacyView] using gm
        exact pending_published x _ _ own g old sm dm
    case deliverUp ok =>
      let z := Storage35.project x.pairView
      change g ∈ (Storage35Base.deliverUp c z ok).s.gates at gm
      have up : z.s.up = x.core.s.up := rfl
      cases hq : x.core.s.up with
      | nil =>
        simp only [Storage35Base.deliverUp, up, hq] at gm
        simpa [priorGates, hq] using pending_original gm sm dm
      | cons m tail =>
        simp only [Storage35Base.deliverUp, up, hq] at gm
        by_cases kind : m.kind = "discard"
        · have heldZ : (z.s.pages m.p).held = (x.core.s.pages m.p).held := rfl
          simp only [kind, beq_self_eq_true, if_true, Storage35Base.upDiscard, heldZ] at gm
          by_cases held : (x.core.s.pages m.p).held = true <;> cases ok
          · simp only [held, Bool.not_true, Bool.false_eq_true, if_false, Bool.not_false, if_true] at gm
            simpa [priorGates, hq] using pending_original gm sm dm
          · simp only [held, Bool.not_true, Bool.false_eq_true, if_false] at gm
            have old : g ∈ (Storage32.discarded z.s m.p).gates := by
              cases m.cur <;> exact gm
            simpa [priorGates, hq, kind, held] using pending_discarded x m.p own g old sm dm
          · have off : (x.core.s.pages m.p).held = false := Bool.eq_false_iff.mpr held
            simp only [off, Bool.not_false, if_true] at gm
            simpa [priorGates, hq] using pending_original gm sm dm
          · have off : (x.core.s.pages m.p).held = false := Bool.eq_false_iff.mpr held
            simp only [off, Bool.not_false, if_true] at gm
            simpa [priorGates, hq, off] using pending_original gm sm dm
        · have miss : (m.kind == "discard") = false := by simpa using kind
          simp only [miss, Bool.false_eq_true, if_false, Storage32.lift] at gm
          simpa [priorGates, hq, kind] using pending_original gm sm dm

theorem pairSafety_prior (c : Config P) (x : State P) (a : Storage32.Action P)
    (normal : c.mutant = "none") (texts : absent ∉ c.texts) (own : GateOwnership x.gates)
    (safe : Storage35.PairSafety10 x.pairView)
    (pub : Storage32.PairPublished11 (Storage35.project x.pairView))
    (live : Storage35.LivePairs11 x.pairView) (kinds : Storage3.Kinds (Storage35.core11 x.pairView).s)
    (inputs : InputTexts11 x.core.s.w x.core.s.up) (en : enabledPrior c x a = true) :
    Storage35.PairSafety10 (execute c x (.prior a)).pairView := by
  let y := Storage35.execute c x.pairView (.prior a)
  let z := execute c x (.prior a)
  have next := Storage35.pairSafety_execute11 c x.pairView (.prior a) normal texts safe
    (pairView_owns x own) pub live kinds inputs (pairView_enabled c x a normal own en)
  obtain ⟨core, renamed⟩ := coreAndPairs_eq c x a normal
  have disk : y.s.disk = z.core.s.disk := congrArg (fun t : Storage31.State P => t.s.disk) core
  have pages : y.s.pages = z.core.s.pages := congrArg (fun t : Storage31.State P => t.s.pages) core
  have job : y.s.job = z.core.s.job := congrArg (fun t : Storage31.State P => t.s.job) core
  intro r rm
  change r ∈ z.renamed at rm
  rw [← renamed] at rm
  rcases next r rm with s | d | pending
  · left
    exact ⟨disk ▸ s.1, pages ▸ s.2.1, job ▸ s.2.2⟩
  · right; left
    exact ⟨disk ▸ d.1, pages ▸ d.2.1, job ▸ d.2.2⟩
  · obtain ⟨sd, dh, db, risk, free, bytes, g, gm, gs, gd, sm, dm⟩ := pending
    have kept : PendingMove z.gates r.1 r.2 := by
      rw [← gs, ← gd]
      exact pending_prior c x a normal own g gm (gs ▸ sm) (gd ▸ dm)
    right; right
    exact ⟨disk ▸ sd, pages ▸ dh, pages ▸ db, pages ▸ risk, job ▸ free, job ▸ bytes,
      (pendingMove_iff z r.1 r.2).mp kept⟩

end Stage6
