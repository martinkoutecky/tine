import CutPublish
namespace CM
namespace Cut
open Consolidated13
variable {K : Type} [DecidableEq K] (P : Inputs K) (D : Date13.Dates K)

/-- The O4 clauses about original files and the intended images. These are
semantic statements, independent of the executable guard's return value. -/
structure BeforeNameSafety (O : Observations) (G : Graph) (old : String) (p : Plan) : Prop where
  wellFormed : WF G
  clean : ∀ s ∈ touchedNames P D G old p,
    Date13.conflicted P D G (Date13.target P D G s) = false
  noCapture : ∀ s ∈ touchedNames P D G old p,
    Date13.target P D G (nameMap P D old p.destination s) ≠ Date13.target P D G s →
    captureClaimed P D G old p s = true →
    ∃ survivor ∈ p.survivor.toList,
      Date13.claim P D survivor = Date13.target P D G (nameMap P D old p.destination s)
  notOwnChild : claimed P D G p.destination = true → P.key old ≠ P.key p.destination →
    nsMatch P.key P.urlHasOrigin old p.destination = none
  oneFormat : ∀ s t, p.source = some s → p.survivor = some t → s.format = t.format
  mergeClean : p.survivor.isSome = true → ∀ f ∈ p.source.toList ++ p.survivor.toList, O.conflict f = false
  leftoverReferences : leftoverUnkeepable P D O old p = false
  supported : ∀ f ∈ G, touchesFile P D O old p f = true → O.supported f = true
  stableOrg : ∀ f ∈ G, f.format = .org → touchesFile P D O old p f = true → O.save f = f
  titlePrefix : ∀ f ∈ G, ∀ h ∈ (O.content f).header, ∀ whole parts,
    Atom.title whole parts ∈ h.text → ∀ site, Sum.inr site ∈ parts →
    site.start < whole.start + Cut.titlePrefix P D old p whole →
    mappedName P D old p site.name = site.name
  freePath : ∀ f ∈ retained G p, ∀ g ∈ G, f ≠ g →
    expectedPath P D O old p f ≠ f.path →
    O.foldedPath (expectedPath P D O old p f) ≠ O.foldedPath g.path

/-- The O4 clauses about actual output files, bound to original retained files
by their order. Existing stationary twins are allowed by the implications. -/
structure AfterNameSafety (O : Observations) (G H : Graph) (old : String) (p : Plan) : Prop where
  cardinality : H.length = (retained G p).length
  wellFormed : WF H
  foldedPaths : ∀ f h, (f,h) ∈ (retained G p).zip H →
    ∀ g k, (g,k) ∈ (retained G p).zip H → f ≠ g →
    h.path ≠ f.path ∨ k.path ≠ g.path → O.foldedPath h.path ≠ O.foldedPath k.path
  clean : ∀ n ∈ touchedNames P D G old p,
    Date13.conflicted P D H (Date13.target P D H n) = false
  identity : ∀ f h, (f,h) ∈ (retained G p).zip H →
    Date13.claim P D h = intendedClaim P D old p f
  noNewTwins : ∀ f h, (f,h) ∈ (retained G p).zip H →
    ∀ g k, (g,k) ∈ (retained G p).zip H → f ≠ g →
    Date13.claim P D f ≠ Date13.claim P D g → Date13.claim P D h ≠ Date13.claim P D k
  conflictedMove : ∀ f h, (f,h) ∈ (retained G p).zip H →
    O.conflict f = true → h.path ≠ f.path → h.text = f.text

private theorem refusal_guard {condition : Bool} {why : Refusal} {tail : Option Refusal}
    (h : (if condition then some why else tail) = none) : condition = false ∧ tail = none := by
  cases condition <;> simp_all

private theorem pre_semantic {O : Observations} {G : Graph} {old : String} {p : Plan}
    (h : preSafety P D O G old p = none) : BeforeNameSafety P D O G old p := by
  have hw := pre_wf P D h
  have hc := pre_touched_clean P D h
  unfold preSafety at h
  dsimp only at h
  obtain ⟨guard1,h⟩ := refusal_guard h
  obtain ⟨guard2,h⟩ := refusal_guard h
  obtain ⟨guard3,h⟩ := refusal_guard h
  obtain ⟨guard4,h⟩ := refusal_guard h
  obtain ⟨guard5,h⟩ := refusal_guard h
  obtain ⟨guard6,h⟩ := refusal_guard h
  obtain ⟨guard7,h⟩ := refusal_guard h
  obtain ⟨guard8,h⟩ := refusal_guard h
  obtain ⟨guard9,h⟩ := refusal_guard h
  obtain ⟨guard10,h⟩ := refusal_guard h
  obtain ⟨guard11,h⟩ := refusal_guard h
  obtain ⟨guard12,h⟩ := refusal_guard h
  all_goals
    constructor
    · exact hw
    · exact hc
    · intro s hs hm hp
      apply Classical.byContradiction
      intro hn
      have hn' : p.survivor.any (fun f => Decidable.decide (Date13.claim P D f =
          Date13.target P D G (nameMap P D old p.destination s))) = false := by
        cases he : p.survivor with
        | none => rfl
        | some t =>
          have ht : Date13.claim P D t ≠ Date13.target P D G (nameMap P D old p.destination s) := by
            intro ht
            exact hn ⟨t,by simp [he],ht⟩
          simp only [Option.any_some]
          simp only [ht,decide_false]
      have : (touchedNames P D G old p).any (fun s =>
          Decidable.decide (Date13.target P D G (nameMap P D old p.destination s) ≠ Date13.target P D G s) &&
          captureClaimed P D G old p s &&
          !p.survivor.any (fun f => Decidable.decide (Date13.claim P D f =
            Date13.target P D G (nameMap P D old p.destination s)))) = true := by
        exact List.any_eq_true.mpr ⟨s,hs,by simp [hm,hp,hn']⟩
      exact False.elim (Bool.noConfusion (guard4.symm.trans this))
    · intro he hk
      cases hn : nsMatch P.key P.urlHasOrigin old p.destination with
      | none => rfl
      | some n =>
        have : (claimed P D G p.destination && Decidable.decide (P.key old ≠ P.key p.destination) &&
            (nsMatch P.key P.urlHasOrigin old p.destination).isSome) = true := by simp [he,hk,hn]
        exact False.elim (Bool.noConfusion (guard3.symm.trans this))
    · intro s t hs ht
      apply Classical.byContradiction
      intro hf
      have hf' : (match p.source,p.survivor with
          | some s,some t => Decidable.decide (s.format ≠ t.format)
          | _,_ => false) = true := by simp [hs,ht,hf]
      have : False := by simpa [hs,ht,hf] using guard5
      exact this
    · intro hm f hf
      cases ho : O.conflict f with
      | false => rfl
      | true =>
        have : (p.survivor.isSome && (p.source.toList ++ p.survivor.toList).any O.conflict) = true := by
          simp only [hm,Bool.true_and]; exact List.any_eq_true.mpr ⟨f,hf,ho⟩
        exact False.elim (Bool.noConfusion (guard6.symm.trans this))
    · cases ho : leftoverUnkeepable P D O old p with
      | false => rfl
      | true => simp [ho] at guard7
    · intro f hf ht
      cases ho : O.supported f with
      | true => rfl
      | false =>
        have : G.any (fun f => touchesFile P D O old p f && !O.supported f) = true :=
          List.any_eq_true.mpr ⟨f,hf,by simp [ht,ho]⟩
        exact False.elim (Bool.noConfusion (guard8.symm.trans this))
    · intro f hf ho ht
      apply Classical.byContradiction
      intro hn
      have : G.any (fun f => f.format == .org && Decidable.decide (O.save f ≠ f) &&
          touchesFile P D O old p f) = true :=
        List.any_eq_true.mpr ⟨f,hf,by simp [ho,hn,ht]⟩
      exact False.elim (Bool.noConfusion (guard9.symm.trans this))
    · intro f hf header hh whole parts ha site hs hp
      apply Classical.byContradiction
      intro hn
      have hs' : parts.any (fun part => match part with
          | .inl _ => false
          | .inr site => Decidable.decide (site.start < whole.start + Cut.titlePrefix P D old p whole) &&
              Decidable.decide (mappedName P D old p site.name ≠ site.name)) = true :=
        List.any_eq_true.mpr ⟨Sum.inr site,hs,by simp [hp,hn]⟩
      have ha' : header.text.any (fun a => match a with
          | .title whole parts => parts.any (fun part => match part with
            | .inl _ => false
            | .inr site => Decidable.decide (site.start < whole.start + Cut.titlePrefix P D old p whole) &&
                Decidable.decide (mappedName P D old p site.name ≠ site.name))
          | _ => false) = true := List.any_eq_true.mpr ⟨Atom.title whole parts,ha,hs'⟩
      have hh' : (O.content f).header.any (fun header => header.text.any (fun a => match a with
          | .title whole parts => parts.any (fun part => match part with
            | .inl _ => false
            | .inr site => Decidable.decide (site.start < whole.start + Cut.titlePrefix P D old p whole) &&
                Decidable.decide (mappedName P D old p site.name ≠ site.name))
          | _ => false)) = true := List.any_eq_true.mpr ⟨header,hh,ha'⟩
      have : G.any (fun f => (O.content f).header.any (fun header => header.text.any (fun a => match a with
          | .title whole parts => parts.any (fun part => match part with
            | .inl _ => false
            | .inr site => Decidable.decide (site.start < whole.start + Cut.titlePrefix P D old p whole) &&
                Decidable.decide (mappedName P D old p site.name ≠ site.name))
          | _ => false))) = true := List.any_eq_true.mpr ⟨f,hf,hh'⟩
      exact False.elim (Bool.noConfusion (guard10.symm.trans this))
    · intro f hf g hg hfg hp
      intro he
      have hg' : G.any (fun g => Decidable.decide (f ≠ g) && Decidable.decide (expectedPath P D O old p f ≠ f.path) &&
          Decidable.decide (O.foldedPath (expectedPath P D O old p f) = O.foldedPath g.path)) = true :=
        List.any_eq_true.mpr ⟨g,hg,by simp [hfg,hp,he]⟩
      have : (retained G p).any (fun f => G.any (fun g => Decidable.decide (f ≠ g) &&
          Decidable.decide (expectedPath P D O old p f ≠ f.path) &&
          Decidable.decide (O.foldedPath (expectedPath P D O old p f) = O.foldedPath g.path))) = true :=
        List.any_eq_true.mpr ⟨f,hf,hg'⟩
      exact False.elim (Bool.noConfusion (guard12.symm.trans this))
private theorem post_semantic {O : Observations} {G H : Graph} {old : String} {p : Plan}
    (before : WF G) (h : postSafety P D O G H old p = none) : AfterNameSafety P D O G H old p := by
  have hw := post_wf P D before h
  unfold postSafety at h
  dsimp only at h
  repeat (split at h <;> try contradiction)
  all_goals try simp only [Bool.false_eq_true,if_false] at h
  all_goals repeat (split at h <;> try contradiction)
  all_goals try simp only [Bool.false_eq_true,if_false] at h
  all_goals repeat (split at h <;> try contradiction)
  all_goals
    constructor
    · exact Classical.not_not.mp (by assumption)
    · exact hw
    · intro f a hfa g b hgb hfg hp he
      have hb : ((retained G p).zip H).any (fun (g,k) => Decidable.decide (f ≠ g) &&
          Decidable.decide (O.foldedPath a.path = O.foldedPath k.path) &&
          Decidable.decide (a.path ≠ f.path ∨ k.path ≠ g.path)) = true :=
        List.any_eq_true.mpr ⟨(g,b),hgb,by simp [hfg,hp,he]⟩
      have : ((retained G p).zip H).any (fun (f,h) => ((retained G p).zip H).any (fun (g,k) =>
          Decidable.decide (f ≠ g) && Decidable.decide (O.foldedPath h.path = O.foldedPath k.path) &&
          Decidable.decide (h.path ≠ f.path ∨ k.path ≠ g.path))) = true :=
        List.any_eq_true.mpr ⟨(f,a),hfa,hb⟩
      contradiction
    · intro n hn
      cases he : Date13.conflicted P D H (Date13.target P D H n) with
      | false => rfl
      | true =>
        have : (touchedNames P D G old p).any (fun n => Date13.conflicted P D H (Date13.target P D H n)) = true :=
          List.any_eq_true.mpr ⟨n,hn,he⟩
        contradiction
    · intro f a hfa
      apply Classical.byContradiction
      intro hn
      have : ((retained G p).zip H).any (fun (f,h) => Decidable.decide (Date13.claim P D h ≠
          intendedClaim P D old p f)) = true :=
        List.any_eq_true.mpr ⟨(f,a),hfa,by simp [hn]⟩
      contradiction
    · apply post_no_new_twins P D (O := O) hw
      · intro n hn
        cases he : Date13.conflicted P D H (Date13.target P D H n) with
        | false => rfl
        | true =>
          have : (touchedNames P D G old p).any (fun n => Date13.conflicted P D H (Date13.target P D H n)) = true :=
            List.any_eq_true.mpr ⟨n,hn,he⟩
          contradiction
      · intro f a hfa
        apply Classical.byContradiction
        intro hn
        have : ((retained G p).zip H).any (fun (f,h) => Decidable.decide (Date13.claim P D h ≠ intendedClaim P D old p f)) = true :=
          List.any_eq_true.mpr ⟨(f,a),hfa,by simp [hn]⟩
        contradiction
    · intro f a hfa hc hp
      apply Classical.byContradiction
      intro ht
      have : ((retained G p).zip H).any (fun (f,h) => O.conflict f && Decidable.decide (h.path ≠ f.path) &&
          Decidable.decide (h.text ≠ f.text)) = true := List.any_eq_true.mpr ⟨(f,a),hfa,by simp [hc,hp,ht]⟩
      contradiction
theorem done_name_safety {O : Observations} {E : Engine} {G : Graph}
    {r : Request} {d : Done} (h : decide P D O E G r = .success d) :
    BeforeNameSafety P D O G r.source d.plan ∧
    AfterNameSafety P D O G d.candidate.graph r.source d.plan := by
  obtain ⟨p,hp,he,hc,_⟩ := done_checked P D h
  have hb : preSafety P D O G r.source d.plan = none := by
    exact (accepted_properties P D hc).1
  have ha : postSafety P D O G d.candidate.graph r.source d.plan = none := by
    exact (accepted_properties P D hc).2.1
  exact ⟨pre_semantic P D hb,post_semantic P D (pre_wf P D hb) ha⟩
end Cut
end CM
