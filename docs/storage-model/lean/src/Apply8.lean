import Storage33
import Shape4
import Lineage4
namespace Storage33
open Storage21
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false
theorem keys_mem8 (eff : Map P) (p : Fin P) : p ∈ keys eff ↔ (eff p).isSome = true := by
  simp [keys,ids]
theorem apply_admission8 (c : Config P) (x : State P) (eff plan : Map P) (dst src : Option (Fin P))
    (mutant : c.mutant = "none") (en : enabledApply c x eff plan dst src = true) :
    ∀ p ∈ keys eff, (x.s.pages p).held = true ∧ clean (x.s.pages p) = true ∧
      plan p = some (Storage3.opCur x.s.toBase p) := by
  have raw := en
  simp only [enabledApply,Bool.and_eq_true] at raw
  have held : (keys eff).all (fun p => (x.s.pages p).held) = true := by
    simpa [Storage31.opHeld,Storage32.project,mutant] using raw.1.1.1.1.1.2
  have hc : (keys eff).all (Storage3.opClean x.s.toBase) = true := by
    simpa [mutant] using raw.1.1.1.2
  have hs : stale x eff plan = false := by
    simpa [mutant] using raw.1.1.2
  intro p mem
  have hp := List.all_eq_true.mp held p mem
  have cp : clean (x.s.pages p) = true := by
    simpa [Storage3.opClean,hp] using List.all_eq_true.mp hc p mem
  have notstale := List.any_eq_false.mp hs p mem
  simp only [stale] at hs
  have ps : plan p ≠ none ∧ Storage3.opCur x.s.toBase p = get plan p := by
    simpa [stale] using notstale
  refine ⟨hp,cp,?_⟩
  cases eq : plan p with
  | none => simp [eq] at ps
  | some b => simpa [get,eq] using ps.2.symm

theorem apply_effects_drafted8 (c : Config P) (x : State P) (eff plan : Map P) (dst src : Option (Fin P))
    (mutant : c.mutant = "none") (p : Fin P) (mem : p ∈ keys eff) :
    let y := opApply c x eff plan dst src
    (y.s.pages p).held = true ∧ (y.s.pages p).buf = get eff p ∧
      (y.s.pages p).ver = x.g.vc + 1 + p.val ∧ (y.s.pages p).risk = true ∧
      (y.s.drafts p).bytes = get eff p ∧ (y.s.drafts p).ver = x.g.vc + 1 + p.val := by
  have hm : (keys eff).contains p = true := by simpa using mem
  simp [opApply,Storage32.commit,Storage3.commit,Storage21.commit,Storage3.fold_put_if,mem,hm,mutant,draftEntry,Storage3.opEdit]
  split <;> simp_all
theorem commit_named_bad8 (c : Config P) (x : State P) (ns : Storage32.Sys P) (ng : Storage32.Ghost P)
    (ps : List (Fin P)) (unchanged : ∀ p, p ∉ ps → ns.pages p = x.s.pages p)
    (named : ∀ p ∈ ps, p ∈ ng.mine) (windows : ns.w = x.s.w) (bad : ng.bad = x.g.bad) :
    (Storage32.commit c x ns ng "opApply").g.bad = x.g.bad := by
  have b : ∀ np : Fin P → Promise, (ids P).all (fun p =>
      keptB (Storage3.project (Storage32.project x)) p ns.toBase.toSys np ng.mine "opApply") = true := by
    intro np
    apply List.all_eq_true.mpr
    intro p _
    by_cases mem : p ∈ ps
    · simp [keptB,List.contains_iff_mem,named p mem]
    · simp [keptB,Storage32.project,Storage3.project,unchanged p mem]
  have w : (ids P).all (fun p => keptW (Storage3.project (Storage32.project x)) p ns.toBase.toSys ng.owed "opApply") = true := by
    simp [keptW,Storage32.project,Storage3.project,windows]
  simp [Storage32.commit,Storage3.commit,Storage21.commit,b,w,bad,tags,union]

theorem apply_bad8 (c : Config P) (x : State P) (eff plan : Map P) (dst src : Option (Fin P))
    (mutant : c.mutant = "none") (en : enabledApply c x eff plan dst src = true) :
    (opApply c x eff plan dst src).g.bad = x.g.bad := by
  have raw := en
  simp only [enabledApply,Bool.and_eq_true] at raw
  have hc : (keys eff).all (Storage3.opClean x.s.toBase) = true := by
    simpa [mutant] using raw.1.1.1.2
  have hs : stale x eff plan = false := by simpa [mutant] using raw.1.1.2
  dsimp only [opApply]
  apply commit_named_bad8 c x _ _ (keys eff)
  · intro p mem
    simp [Storage3.fold_put_if,mem]
  · intro p mem
    simp [mem,List.all_eq_true.mp hc p mem]
  · rfl
  · simp [hc,hs,tags,union]
theorem apply_guarantee8 (c : Config P) (x : State P) (eff plan : Map P) (dst src : Option (Fin P))
    (mutant : c.mutant = "none") (en : enabledApply c x eff plan dst src = true)
    (before : guarantee x = true) : guarantee (opApply c x eff plan dst src) = true := by
  have hb := apply_bad8 c x eff plan dst src mutant en
  have raw := en
  simp only [enabledApply,Bool.and_eq_true] at raw
  have alive : x.s.alive = true := raw.1.1.1.1.1.1.1.1
  have ac : Storage3.accepted (Storage32.project (opApply c x eff plan dst src)) = Storage3.accepted (Storage32.project x) := rfl
  have tr : Storage3.trashed (Storage32.project (opApply c x eff plan dst src)) = Storage3.trashed (Storage32.project x) := rfl
  have nl : Storage3.noLoss (Storage32.project (opApply c x eff plan dst src)) = true := by
    simp [Storage3.noLoss,Storage21.noLoss,Storage3.project,Storage32.project,opApply,Storage32.commit,Storage3.commit,Storage21.commit,alive]
  change Storage3.guarantee (Storage32.project (opApply c x eff plan dst src)) = true
  change Storage3.guarantee (Storage32.project x) = true at before
  simp only [Storage3.guarantee,Bool.and_eq_true] at before ⊢
  exact ⟨⟨⟨nl,by simpa [Storage3.transitions,Storage21.transitions,Storage3.project,Storage32.project,hb] using before.1.1.2⟩,by rw [ac]; exact before.1.2⟩,by rw [tr]; exact before.2⟩
theorem fold_ack_page8 (ps : List (Fin P)) (pr : Fin P → Promise) (bytes vers : Fin P → Int) (p : Fin P) :
    ps.foldl (fun acc q => put acc q (ack (acc q) (bytes q) (vers q) false 0)) pr p =
      if p ∈ ps then ack (pr p) (bytes p) (vers p) false 0 else pr p := by
  induction ps generalizing pr with
  | nil => simp
  | cons q tail ih =>
    rw [List.foldl_cons,ih]
    by_cases he : p = q
    · subst q
      simp [put,Storage3.ack_idem4]
    · by_cases ht : p ∈ tail <;> simp [put,he,ht]
theorem apply_promise8 (c : Config P) (x : State P) (eff plan : Map P) (dst src : Option (Fin P))
    (p : Fin P) (mem : p ∈ keys eff) :
    (opApply c x eff plan dst src).g.promise p = ack (x.g.promise p) (get eff p) (x.g.vc+1+p.val) false 0 := by
  have hm : (keys eff).contains p = true := by simpa using mem
  simp [opApply,Storage32.commit,Storage3.commit,Storage21.commit,Storage3.fold_put_if,fold_ack_page8,mem,hm,clean]
theorem apply_new_custody8 (c : Config P) (x : State P) (eff plan : Map P) (dst src : Option (Fin P))
    (mutant : c.mutant = "none") (p : Fin P) (mem : p ∈ keys eff)
    (bound : (x.g.promise p).ver ≤ x.g.vc) :
    let y := opApply c x eff plan dst src
    (y.g.promise p).on = true ∧ (y.g.promise p).bytes = get eff p ∧
      (y.g.promise p).ver = x.g.vc+1+p.val ∧ (y.g.promise p).saved = false ∧
      (y.s.drafts p).bytes = (y.g.promise p).bytes ∧ (y.s.drafts p).ver = (y.g.promise p).ver := by
  have later : x.g.vc+1+p.val > (x.g.promise p).ver := by omega
  have promised := apply_promise8 c x eff plan dst src p mem
  simp only [ack,later,decide_true,Bool.true_or,ite_true] at promised
  have d := apply_effects_drafted8 c x eff plan dst src mutant p mem
  simp only [promised]
  exact ⟨True.intro,True.intro,True.intro,True.intro,d.2.2.2.2.1,d.2.2.2.2.2⟩
#print axioms apply_effects_drafted8
#print axioms apply_bad8
#print axioms apply_guarantee8
#print axioms apply_new_custody8
end Storage33
