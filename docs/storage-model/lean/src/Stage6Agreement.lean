import Stage6
import ArbitraryInitialDisk

namespace Stage6
open Storage21

def Gate.ofLegacy (gt : Storage32.Gate P) : Gate P :=
  ⟨[⟨gt.dst, gt.src⟩], fun p => if gt.vers p = noneBytes then none else some (gt.vers p), gt.pending⟩

def ofLegacy (x : Storage35.State P) : State P :=
  State.fromCore x (x.s.gates.map Gate.ofLegacy)

def embedAction : Storage35.Action P → Action P
  | .prior a => .prior a
  | .extClaim p present => .extClaim p present
  | .opApply eff plan none none => .opApply ⟨eff, plan, []⟩
  | .opApply eff plan (some dst) (some src) => .opApply ⟨eff, plan, [⟨dst, src⟩]⟩
  | .opApply eff plan (some p) none => .malformedApply eff plan p true
  | .opApply eff plan none (some p) => .malformedApply eff plan p false

theorem embedAction_injective {a b : Storage35.Action P}
    (eq : embedAction a = embedAction b) : a = b := by
  cases a with
  | prior a =>
    cases b with
    | opApply e p d s => cases d <;> cases s <;> cases eq
    | _ => cases eq <;> rfl
  | extClaim p b =>
    rename_i other
    cases other with
    | opApply e plan d s => cases d <;> cases s <;> cases eq
    | _ => cases eq <;> rfl
  | opApply e₁ p₁ d₁ s₁ =>
    cases b with
    | opApply e₂ p₂ d₂ s₂ =>
      cases d₁ <;> cases s₁ <;> cases d₂ <;> cases s₂ <;> cases eq <;> rfl
    | _ => cases d₁ <;> cases s₁ <;> cases eq

private theorem flatten_singletons (gs : List (Storage32.Gate P)) :
    (gs.map Gate.ofLegacy).flatMap (fun gt => gt.moves.map fun m =>
      (⟨m.dst, m.src, fun p => (gt.vers p).getD noneBytes, gt.pending⟩ : Storage32.Gate P)) = gs := by
  induction gs with
  | nil => rfl
  | cons g gs ih =>
    have vers : (fun p => (if g.vers p = noneBytes then none else some (g.vers p)).getD noneBytes) =
        g.vers := by
      funext p
      by_cases h : g.vers p = noneBytes <;> simp [h]
    cases g
    simp_all only [List.map_cons, List.flatMap_cons, Gate.ofLegacy, List.map_nil, List.singleton_append]

@[simp] theorem legacyView_ofLegacy (x : Storage35.State P) : (ofLegacy x).legacyView = x := by
  cases x with
  | mk s g =>
    cases s with
    | mk s claimed =>
      cases s
      cases g
      simp only [State.legacyView, ofLegacy, State.fromCore, Storage35.project,
        Storage34.project, Storage32.project, flatten_singletons]

private theorem touches_legacy (x : Storage35.State P) (ps : List (Slot P)) :
    touches (ofLegacy x).gates ps = Storage32.touches x.s.to32 ps := by
  unfold touches Storage32.touches
  simp only [ofLegacy, State.fromCore, List.any_map]
  congr 1
  funext gt
  change (ps.any fun p => ((Gate.ofLegacy gt).vers p).isSome) = _
  congr 1
  funext p
  by_cases eq : gt.vers p = noneBytes <;> simp [Gate.ofLegacy, eq]

private theorem source_wait (ps : List (Slot P)) (s : Slot P)
    (nd : ps.Nodup) (mem : s ∈ ps) :
    (ps.any fun p => p != s) = decide (ps.length > 1) := by
  apply Bool.eq_iff_iff.mpr
  simp only [List.any_eq_true, bne_iff_ne, decide_eq_true_eq]
  constructor
  · rintro ⟨p, hp, ne⟩
    by_cases long : ps.length > 1
    · exact long
    exfalso
    cases ps with
    | nil => simp at mem
    | cons q qs =>
      have tail : qs.length = 0 := by simp only [List.length_cons] at long; omega
      have empty := List.length_eq_zero_iff.mp tail
      simp [empty] at hp mem
      exact ne (hp.trans mem.symm)
  · intro long
    cases ps with
    | nil => simp at mem
    | cons q qs =>
      cases qs with
      | nil => simp at long
      | cons r rs =>
        by_cases hq : q = s
        · have hr : r ≠ s := by
            intro hr
            simp [hq, hr] at nd
          exact ⟨r, by simp, hr⟩
        · exact ⟨q, by simp, hq⟩

private theorem gated_legacy (c : Config P) (g : Storage32.Gate P) (p : Slot P)
    (normal : c.mutant = "none") (nd : g.pending.Nodup) :
    (Gate.ofLegacy g).gated p = Storage32.gates1 c g p := by
  have receiver : (g.pending.any fun q => q == g.dst) = g.pending.contains g.dst := by
    apply Bool.eq_iff_iff.mpr
    simp
  by_cases mem : p ∈ g.pending
  · by_cases src : p = g.src
    · subst p
      have wait := source_wait g.pending g.src nd mem
      change (g.pending.any fun q => !(q == g.src)) = _ at wait
      simp [Gate.gated, Gate.ofLegacy, Storage32.gates1, normal, receiver, mem, wait, bne]
    · have no := beq_eq_false_iff_ne.mpr src
      simp [Gate.gated, Gate.ofLegacy, Storage32.gates1, normal, receiver, mem, no, bne]
  · simp [Gate.gated, Gate.ofLegacy, Storage32.gates1, mem]

private theorem gated_any_legacy (c : Config P) (x : Storage35.State P) (p : Slot P)
    (normal : c.mutant = "none") (nd : ∀ g ∈ x.s.gates, g.pending.Nodup) :
    ((ofLegacy x).gates.any fun gt => gt.gated p) = Storage32.gated c x.s.to32 p := by
  simp only [ofLegacy, State.fromCore, List.any_map, Storage32.gated]
  apply Bool.eq_iff_iff.mpr
  simp only [List.any_eq_true]
  exact ⟨fun ⟨g, hg, he⟩ => ⟨g, hg, (gated_legacy c g p normal (nd g hg)).symm.trans he⟩,
    fun ⟨g, hg, he⟩ => ⟨g, hg, (gated_legacy c g p normal (nd g hg)).trans he⟩⟩

private theorem enabled_prior_legacy (c : Config P) (x : Storage35.State P) (a : Storage32.Action P)
    (normal : c.mutant = "none") (nd : ∀ g ∈ x.s.gates, g.pending.Nodup) :
    enabledPrior c (ofLegacy x) a = Storage32.enabled c (Storage35.project x) a := by
  cases a with
  | opRename =>
    simp only [enabledPrior, Storage32.enabled, touches_legacy]
    rfl
  | opDelete =>
    simp only [enabledPrior, Storage32.enabled, touches_legacy]
    rfl
  | flushDel =>
    simp only [enabledPrior, Storage32.enabled, normal, gated_any_legacy c x _ normal nd]
    rfl
  | load => rfl
  | old a =>
    cases a <;> try rfl
    case flush =>
      simp only [enabledPrior, Storage32.enabled, gated_any_legacy c x _ normal nd]
      rfl
    case deliverUp => simp only [enabledPrior, Storage32.enabled, legacyView_ofLegacy]; rfl

private theorem shape_empty (eff plan : WriteMap P) :
    (Batch.mk eff plan []).Shape ↔ Storage33.keys eff ≠ [] := by
  simp [Batch.Shape, Batch.keys, endpoints]

private theorem shape_single (eff plan : WriteMap P) (d s : Slot P) :
    (Batch.mk eff plan [⟨d, s⟩]).Shape ↔
      d ≠ s ∧ d ∈ Storage33.keys eff ∧ s ∈ Storage33.keys eff ∧
        Storage33.get eff s = absent ∧ Storage33.get eff d ≠ absent ∧
        Storage33.get plan s ≠ absent := by
  have nonempty : d ∈ Storage33.keys eff → Storage33.keys eff ≠ [] := by
    intro hd eq
    simp [eq] at hd
  cases hd : eff d <;> cases hs : eff s <;>
    simp_all [Batch.Shape, Batch.keys, endpoints, Storage33.keys_mem8, Storage33.get]

private theorem enabled_apply_legacy (c : Config P) (x : Storage35.State P)
    (eff plan : WriteMap P) (d s : Option (Slot P)) :
    enabled c (ofLegacy x) (embedAction (.opApply eff plan d s)) =
      Storage35.enabled c x (.opApply eff plan d s) := by
  cases d <;> cases s
  all_goals
    apply Bool.eq_iff_iff.mpr
    simp only [embedAction, enabled, enabledApply, State.admissionView, legacyView_ofLegacy, Batch.keys,
      touches_legacy, Storage35.enabled, Storage34.enabled, Storage34.enabledApply,
      Storage33.enabledApply, Storage34.project, Storage32.project, Storage32.touches,
      List.any_nil, Bool.not_false, Bool.and_true, Bool.and_eq_true, decide_eq_true_eq]
  · simp only [shape_empty]
    simp only [Storage33.stale, Storage34.claimedCreates]
    grind
  · simp
  · simp
  · simp only [shape_single, bne_iff_ne, beq_iff_eq, List.contains_iff_mem,
      Storage33.stale, Storage34.claimedCreates]
    grind

private theorem published_legacy (sys : Storage32.Sys P) (p : Slot P) (v : Int) :
    published (sys.gates.map Gate.ofLegacy) p v =
      (Storage32.discharge sys p v).gates.map Gate.ofLegacy := by
  rw [published, Storage32.discharge, List.filterMap_map, List.map_filterMap]
  apply congrArg (fun f => sys.gates.filterMap f)
  funext gt
  by_cases ver : gt.vers p = noneBytes
  · simp [Gate.ofLegacy, Gate.retained, ver]
    split <;> rfl
  · by_cases ge : v ≥ gt.vers p
    · simp [Gate.ofLegacy, Gate.retained, ver, ge]
      split <;> rfl
    · simp [Gate.ofLegacy, Gate.retained, ver, ge]
      split <;> rfl

private theorem discard_legacy (sys : Storage32.Sys P) (p : Slot P) :
    discarded (sys.gates.map Gate.ofLegacy) p =
      (Storage32.discarded sys p).gates.map Gate.ofLegacy := by
  rw [discarded, Storage32.discarded, List.filterMap_map, List.map_filterMap]
  apply congrArg (fun f => sys.gates.filterMap f)
  funext gt
  by_cases mem : p ∈ gt.pending
  · by_cases dst : gt.dst = p
    · simp [Gate.ofLegacy, Gate.retained, mem, dst]
    · have ne := beq_eq_false_iff_ne.mpr dst
      by_cases empty : exclude gt.pending [p] = [] <;>
        simp [Gate.ofLegacy, Gate.retained, mem, ne, dst, bne, empty]
  · have unchanged : exclude gt.pending [p] = gt.pending := by
      apply List.filter_eq_self.mpr
      intro q hq
      have ne : q ≠ p := fun eq => mem (eq ▸ hq)
      simp [ne]
    simp [Gate.ofLegacy, Gate.retained, unchanged, mem]
    split <;> rfl

private theorem makeGate_legacy (x : Storage35.State P) (d s : Slot P) (ps : List (Slot P))
    (nn : 0 ≤ x.g.vc) :
    Gate.ofLegacy (Storage32.makeGate (Storage35.project x) d s ps) =
      ⟨[⟨d, s⟩], fun p => if ps.contains p then some (version (ofLegacy x) p) else none, ps⟩ := by
  have nonSentinel (p : Slot P) : x.g.vc + 1 + (p.val : Int) ≠ noneBytes := by
    unfold noneBytes
    omega
  simp only [Gate.ofLegacy, Storage32.makeGate, Gate.mk.injEq, true_and, and_true]
  funext p
  cases mem : ps.contains p <;>
    simp [version, ofLegacy, State.fromCore, Storage35.project, Storage34.project,
      Storage32.project, nonSentinel]

private theorem priorGates_legacy (c : Config P) (x : Storage35.State P) (a : Storage32.Action P)
    (normal : c.mutant = "none") (nn : 0 ≤ x.g.vc) :
    priorGates (ofLegacy x) a = (Storage35.execute c x (.prior a)).s.gates.map Gate.ofLegacy := by
  cases a with
  | load | opDelete | flushDel => rfl
  | opRename src dst refs rt =>
    change (if Storage3.opCur (Storage35.project x).s.toBase src != absent then
      install (ofLegacy x) [⟨dst, src⟩] (union [src, dst] refs) else (ofLegacy x).gates) =
      (Storage32.opRename c (Storage35.project x) src dst refs rt).s.gates.map Gate.ofLegacy
    simp only [Storage32.opRename, Storage32.lift]
    by_cases full : (Storage3.opCur (Storage35.project x).s.toBase src != absent) = true
    · simp only [full, if_true, install, List.isEmpty_cons, Bool.false_eq_true, if_false,
        List.map_append, List.map_cons, List.map_nil]
      rw [makeGate_legacy x dst src _ nn]
      rfl
    · simp only [full]
      rfl
  | old a =>
    cases a <;> try rfl
    case rename =>
      simp [Storage35.execute, Storage35Base.execute, Storage32.execute, Storage32.rename,
        Storage32.lift, normal, Storage34.lift]
      rfl
    case check =>
      simp [Storage35.execute, Storage35Base.execute, Storage32.execute, Storage32.check,
        Storage32.lift, normal, Storage34.lift]
      rfl
    case saveFail =>
      simp [Storage35.execute, Storage35Base.execute, Storage32.execute, Storage32.saveFail,
        Storage32.lift, normal, Storage34.lift]
      rfl
    case dirSync ok =>
      change (if ok then published (ofLegacy x).gates x.s.job.p x.s.job.ver else (ofLegacy x).gates) =
        (Storage32.dirSync c (Storage35.project x) ok).s.gates.map Gate.ofLegacy
      cases ok
      · simp [Storage32.dirSync, Storage32.lift, normal]; rfl
      · simp only [if_true, Storage32.dirSync, normal, Bool.true_and, bne]
        exact published_legacy (Storage32.lift (Storage35.project x)
          (Storage3.dirSync c (Storage32.project (Storage35.project x)) true)).s _ _
    case deliverUp ok =>
      change (match x.s.up with
        | m :: _ => if m.kind == "discard" && ok && (x.s.pages m.p).held then
                      discarded (ofLegacy x).gates m.p else (ofLegacy x).gates
        | [] => (ofLegacy x).gates) =
        (Storage35Base.deliverUp c (Storage35.project x) ok).s.gates.map Gate.ofLegacy
      cases queue : x.s.up with
      | nil =>
        unfold Storage35Base.deliverUp
        change x.s.up = [] at queue
        rw [show (Storage35.project x).s.up = [] from queue]
        rfl
      | cons m tail =>
        unfold Storage35Base.deliverUp
        rw [show (Storage35.project x).s.up = m :: tail from queue]
        by_cases kind : m.kind = "discard"
        · have pages : (Storage35.project x).s.pages = x.s.pages := rfl
          cases ok <;> cases held : (x.s.pages m.p).held
          all_goals
            simp only [kind, beq_self_eq_true, Storage35Base.upDiscard, Storage32.commit, pages, held]
          all_goals try rfl
          exact discard_legacy _ _
        · simp [kind, Storage32.lift]
          rfl

private theorem execute_prior_legacy (c : Config P) (x : Storage35.State P) (a : Storage32.Action P)
    (normal : c.mutant = "none") (nn : 0 ≤ x.g.vc) :
    execute c (ofLegacy x) (.prior a) = ofLegacy (Storage35.execute c x (.prior a)) := by
  simp only [execute, legacyView_ofLegacy, priorGates_legacy c x a normal nn]
  rfl

private theorem obligations_empty (x : State P) (ps : List (Slot P)) :
    obligations x [] ps = [] := by simp [obligations, union]

private theorem obligations_single (x : Storage35.State P) (d s : Slot P) (ps : List (Slot P)) :
    obligations (ofLegacy x) [⟨d, s⟩] ps =
      Storage32.makeObl (Storage35.project x) d s (exclude ps [d, s]) := by
  have flat (qs : List (Slot P)) :
      (qs.flatMap fun r => [(r, d, version (ofLegacy x) d, d)]) =
       qs.map (fun r => (r, d, version (ofLegacy x) d, d)) := by
    induction qs <;> simp [*]
  simp [obligations, endpoints, Storage32.makeObl, flat]
  rfl

private theorem execute_apply_legacy (c : Config P) (x : Storage35.State P)
    (eff plan : WriteMap P) (d s : Option (Slot P))
    (normal : c.mutant = "none") (nn : 0 ≤ x.g.vc)
    (en : Storage35.enabled c x (.opApply eff plan d s) = true) :
    execute c (ofLegacy x) (embedAction (.opApply eff plan d s)) =
      ofLegacy (Storage35.execute c x (.opApply eff plan d s)) := by
  cases d <;> cases s
  · simp only [embedAction, execute, opApply, legacyView_ofLegacy, install,
      List.isEmpty_nil, if_true, obligations_empty]
    rfl
  · simp [Storage35.enabled, Storage34.enabled, Storage34.enabledApply, Storage33.enabledApply] at en
  · simp [Storage35.enabled, Storage34.enabled, Storage34.enabledApply, Storage33.enabledApply] at en
  · rename_i dst src
    simp only [embedAction, execute, opApply, legacyView_ofLegacy, obligations_single]
    have gates : install (ofLegacy x) [⟨dst, src⟩] (Storage33.keys eff) =
        (Storage35.opApply c x eff plan (some dst) (some src)).s.gates.map Gate.ofLegacy := by
      have notMAM : (true && c.mutant != "MAM") = true := by simp [normal]
      simp only [install, Storage35.opApply, Storage34.opApply, Storage33.opApply, Storage32.commit,
        List.isEmpty_cons, Bool.false_eq_true, if_false, Option.isSome_some, Option.getD_some,
        notMAM, if_true, List.map_append, List.map_cons, List.map_nil]
      have make := makeGate_legacy x dst src (Storage33.keys eff) nn
      exact congrArg (fun g => x.s.gates.map Gate.ofLegacy ++ [g]) make.symm
    rw [show (Batch.mk eff plan [⟨dst, src⟩]).keys = Storage33.keys eff from rfl, gates]
    cases h : (x.s.disk src != absent)
    all_goals
      simp only [ofLegacy, State.fromCore, Storage35.execute, Storage35.opApply, Storage34.opApply,
        Storage33.opApply, Storage32.commit, Storage35.project, Storage34.project, Storage32.project,
        Option.isSome_some, Option.getD_some, List.filter_cons, List.filter_nil, h]
    all_goals rfl

private def PendingNodup (gates : List (Storage32.Gate P)) : Prop :=
  ∀ gt ∈ gates, gt.pending.Nodup

private theorem filtered_pending (gs : List (Storage32.Gate P))
    (f : Storage32.Gate P → Option (Storage32.Gate P))
    (sub : ∀ gt ht, f gt = some ht → List.Sublist ht.pending gt.pending)
    (nd : PendingNodup gs) : PendingNodup (gs.filterMap f) := by
  intro ht mem
  obtain ⟨gt, hg, eq⟩ := List.mem_filterMap.mp mem
  exact (nd gt hg).sublist (sub gt ht eq)

private theorem nodup_discharge (sys : Storage32.Sys P) (p : Slot P) (v : Int)
    (nd : PendingNodup sys.gates) : PendingNodup (Storage32.discharge sys p v).gates := by
  unfold Storage32.discharge
  apply filtered_pending _ _ _ nd
  intro gt ht eq
  split at eq <;> dsimp only at eq <;> split at eq <;>
    simp only [reduceCtorEq, Option.some.injEq] at eq
  · subst ht
    exact List.filter_sublist
  · subst ht
    exact List.Sublist.refl _

private theorem nodup_discard (sys : Storage32.Sys P) (p : Slot P)
    (nd : PendingNodup sys.gates) : PendingNodup (Storage32.discarded sys p).gates := by
  unfold Storage32.discarded
  apply filtered_pending _ _ _ nd
  intro gt ht eq
  split at eq <;> dsimp only at eq <;> split at eq <;>
    simp only [reduceCtorEq, Option.some.injEq] at eq
  · subst ht
    exact List.Sublist.refl _
  · subst ht
    exact List.filter_sublist

private theorem union_nodup (a b : List (Slot P)) (nd : a.Nodup) :
    (union a b).Nodup := by
  induction b generalizing a with
  | nil => exact nd
  | cons q qs ih =>
    simp only [union, List.foldl_cons]
    apply ih
    split
    · exact nd
    · rename_i no
      refine List.nodup_append.mpr ⟨nd, by simp, ?_⟩
      intro p hp r hr eq
      obtain rfl := List.mem_singleton.mp hr
      exact no (List.contains_iff_mem.mpr (eq ▸ hp))

private theorem finRange_nodup (n : Nat) : (List.finRange n).Nodup := by
  induction n with
  | zero => simp
  | succ n ih =>
    rw [List.finRange_succ, List.nodup_cons]
    constructor
    · simp [Fin.succ_ne_zero]
    · simpa [List.nodup_iff_pairwise_ne, List.pairwise_map] using ih

private theorem nodup_execute (c : Config P) (x : Storage35.State P) (a : Storage35.Action P)
    (normal : c.mutant = "none") (en : Storage35.enabled c x a = true)
    (nd : PendingNodup x.s.gates) : PendingNodup (Storage35.execute c x a).s.gates := by
  cases a with
  | extClaim => exact nd
  | opApply eff plan dst src =>
    change PendingNodup (Storage33.opApply c (Storage35.project x) eff plan dst src).s.gates
    simp only [Storage33.opApply, Storage32.commit]
    split
    · intro gt mem
      simp only [List.mem_append, List.mem_singleton] at mem
      rcases mem with old | rfl
      · exact nd gt old
      · exact (finRange_nodup P).sublist List.filter_sublist
    · exact nd
  | prior a =>
    change PendingNodup (Storage35.project x).s.gates at nd
    change PendingNodup (Storage35Base.execute c (Storage35.project x) a).s.gates
    apply Storage35Base.preserve10 _ _ _ (fun y => PendingNodup y.s.gates)
    · cases a with
      | opDelete | flushDel | load => exact nd
      | opRename src dst refs rt =>
        have raw : Storage32.enabled c (Storage35.project x) (.opRename src dst refs rt) = true := en
        have ne : src ≠ dst := by
          simp only [Storage32.enabled, Storage31.enabled, Storage3.enabled, Bool.and_eq_true, bne_iff_ne] at raw
          grind
        simp only [Storage32.execute, Storage32.opRename, Storage32.lift]
        split
        · intro gt mem
          simp only [List.mem_append, List.mem_singleton] at mem
          rcases mem with old | rfl
          · exact nd gt old
          · exact union_nodup [src, dst] refs (by simp [ne])
        · exact nd
      | old a =>
        cases a <;> try exact nd
        case rename =>
          simpa [Storage32.execute, Storage32.rename, normal, Storage32.lift] using nd
        case check =>
          simpa [Storage32.execute, Storage32.check, normal, Storage32.lift] using nd
        case saveFail =>
          simpa [Storage32.execute, Storage32.saveFail, normal, Storage32.lift] using nd
        case dirSync ok =>
          cases ok
          · simpa [Storage32.execute, Storage32.dirSync, normal, Storage32.lift] using nd
          · simp only [Storage32.execute, Storage32.dirSync, normal, Bool.true_and, bne]
            exact nodup_discharge _ _ _ nd
        case deliverUp ok =>
          simp only [Storage32.execute, Storage32.deliverUp]
          split
          · exact nd
          · split
            · simp only [Storage32.upDiscard]
              split
              · exact nd
              · split
                · exact nd
                · exact nodup_discard _ _ nd
            · exact nd
        all_goals simp [Storage32.execute, Storage32.stopped, Storage32.lift, PendingNodup]
    · intro ok m tail _ _
      simp only [Storage35Base.upDiscard]
      split
      · exact nd
      · split
        · exact nd
        · exact nodup_discard _ _ nd

private theorem reachable_pending_nodup (c : Config P) (disk : Slot P → Int) (x : Storage35.State P)
    (normal : c.mutant = "none") (reached : Storage35.Reachable c disk x) :
    PendingNodup x.s.gates := by
  induction reached with
  | initial => simp [PendingNodup, Storage35.initWith, Storage34.initWith, Storage34.embed,
      Storage33.initWith, Storage32.initWith, Storage32.embed]
  | @advance x y _ a eq ih =>
    unfold Storage35.step at eq
    split at eq
    · cases eq
      exact nodup_execute c x a normal (by assumption) ih
    · contradiction

theorem enabled_agrees (c : Config P) (disk : Slot P → Int) (x : Storage35.State P)
    (normal : c.mutant = "none") (reached : Storage35.Reachable c disk x) (a : Storage35.Action P) :
    enabled c (ofLegacy x) (embedAction a) = Storage35.enabled c x a := by
  cases a with
  | prior a => exact enabled_prior_legacy c x a normal (reachable_pending_nodup c disk x normal reached)
  | opApply eff plan d s => exact enabled_apply_legacy c x eff plan d s
  | extClaim => rfl

theorem step_agrees (c : Config P) (disk : Slot P → Int) (x : Storage35.State P)
    (normal : c.mutant = "none") (reached : Storage35.Reachable c disk x) (a : Storage35.Action P) :
    step c (ofLegacy x) (embedAction a) = (Storage35.step c x a).map ofLegacy := by
  have nn := (Storage35.aux_reachable10 c disk x normal reached).bounds.nonnegative
  unfold step Storage35.step
  rw [enabled_agrees c disk x normal reached a]
  split
  · rename_i en
    apply congrArg some
    cases a with
    | prior a => exact execute_prior_legacy c x a normal nn
    | opApply eff plan d s => exact execute_apply_legacy c x eff plan d s normal nn en
    | extClaim p present => simp only [embedAction, execute, legacyView_ofLegacy]; rfl
  · rfl

theorem reachable_ofLegacy (c : Config P) (disk : Slot P → Int) (x : Storage35.State P)
    (normal : c.mutant = "none") (valid : InitialValid c disk)
    (reached : Storage35.Reachable c disk x) :
    Reachable c disk (ofLegacy x) := by
  induction reached with
  | initial => exact .initial valid
  | @advance x y reached a eq ih =>
    exact .advance ih (embedAction a) (by rw [step_agrees c disk x normal reached a, eq]; rfl)

theorem legacy_targets (c : Config P) (disk : Slot P → Int) (x : Storage35.State P)
    (normal : c.mutant = "none") (valid : InitialValid c disk)
    (reached : Storage35.Reachable c disk x) :
    guarantee (ofLegacy x) = true ∧ orderHolds (ofLegacy x) = true ∧
      renameResolves (ofLegacy x) = true :=
  Storage35.all_targets_arbitrary_initial_disk c disk x normal valid.1 valid.2.1 valid.2.2.1 valid.2.2.2 reached

#print axioms embedAction_injective
#print axioms enabled_agrees
#print axioms step_agrees
#print axioms reachable_ofLegacy
#print axioms legacy_targets

end Stage6
