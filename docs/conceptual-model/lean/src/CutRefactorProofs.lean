import CutSettings
/-! Historical algorithms in this module are proof comparators, not a second
operational model. The selected model never imports this module. -/
namespace CM
namespace Cut
namespace Refactor4
open Consolidated13
variable {K : Type} [DecidableEq K] (P : Inputs K) (D : Date13.Dates K)

-- The old three alternatives, encoded without adding a selected-model schema.
def toSum {α : Type} : Result α → Refusal ⊕ (Question ⊕ α)
  | .refuse r => .inl r | .ask q => .inr (.inl q) | .success x => .inr (.inr x)
def fromSum {α : Type} : Refusal ⊕ (Question ⊕ α) → Result α
  | .inl r => .refuse r | .inr (.inl q) => .ask q | .inr (.inr x) => .success x

theorem selection_result_equiv (x : Selection) : fromSum (toSum x) = x := by cases x <;> rfl
theorem selection_sum_equiv (x : Refusal ⊕ (Question ⊕ Plan)) : toSum (fromSum x) = x := by
  rcases x with r | q | p <;> rfl
theorem outcome_result_equiv (x : Outcome) : fromSum (toSum x) = x := by cases x <;> rfl
theorem outcome_sum_equiv (x : Refusal ⊕ (Question ⊕ Done)) : toSum (fromSum x) = x := by
  rcases x with r | q | p <;> rfl
structure LegacyCandidate where
  graph : Graph

theorem candidate_graph_equiv (x : LegacyCandidate) : LegacyCandidate.mk x.graph = x := by cases x; rfl
theorem graph_candidate_equiv (x : Graph) : (LegacyCandidate.mk x).graph = x := rfl

def legacyPost (O : Observations) (G H : Graph) (old : String) (p : Plan) : Option (Refusal ⊕ Unit) :=
  let pairs := (retained G p).zip H
  if H.length ≠ (retained G p).length then some (.inl Refusal.observationsP2)
  else if !(H.map File.path).Nodup then some (.inl Refusal.pathTaken)
  else if pairs.any (fun (f,h) => pairs.any (fun (g,k) => Decidable.decide (f ≠ g) &&
      Decidable.decide (O.foldedPath h.path = O.foldedPath k.path) &&
      Decidable.decide (h.path ≠ f.path ∨ k.path ≠ g.path))) then some (.inl Refusal.pathTaken)
  else if (touchedNames P D G old p).any (fun n => Date13.conflicted P D H
      (Date13.target P D H n)) then some (.inl Refusal.contested)
  else if pairs.any (fun (f,h) => Decidable.decide (Date13.claim P D h ≠
      intendedClaim P D old p f)) then some (.inl Refusal.titleIdentity)
  else if pairs.any (fun (f,h) => pairs.any (fun (g,k) => Decidable.decide (f ≠ g) &&
    Decidable.decide (Date13.claim P D h = Date13.claim P D k) &&
    Decidable.decide (Date13.claim P D f ≠ Date13.claim P D g))) then some (.inr ())
  else if pairs.any (fun (f,h) => O.conflict f && Decidable.decide (h.path ≠ f.path) && Decidable.decide (h.text ≠ f.text)) then some (.inl Refusal.conflictRewrite)
  else none
def legacySelectCore (O : Observations) (G : Graph) (r : Request) : Selection :=
  if r.answer = some .cancel then .refuse .cancelled
  else if !eligible P D G r.source then .refuse .absentSource
  else if (Date13.target P D G r.source).1 = .journal then .refuse .daySource
  else
    let s := sourceFile P D G r.source
    let holders := (Date13.claimants P D G (Date13.target P D G r.typed)).filter (fun f => some f != s)
    match holders with
    | _ :: _ :: _ => .refuse Refusal.contested
    | [t] =>
      match r.answer with
      | none => mergeSelection P D O G r s t (some .confirmMerge)
      | some .confirm => mergeSelection P D O G r s t none
      | _ => .refuse .unavailableAnswer
    | [] =>
      let declarers := (Date13.declarers P D G (Date13.target P D G r.typed)).filter (fun f => some f != s)
      let selfClaims := s.any (fun f => Decidable.decide (Date13.claim P D f = Date13.target P D G r.typed))
      if !selfClaims && !declarers.isEmpty then
        match minBy Date13.pathLe (Date13.declarers P D G (Date13.target P D G r.typed)) with
        | some owner => .refuse (.aliasHeld owner.path)
        | none => .refuse .absentSource
      else if r.answer.isSome then .refuse .unavailableAnswer
      else authorize P D O G r ⟨r.typed,s,none⟩ none

def legacyP1File (O : Observations) (old : String) (p : Plan) (f h : File) : Bool :=
  if inPair p f then true else
    let es := edits P D O old p f
    editGeometry f.text 0 es && Decidable.decide (bytes h.text = splice (bytes f.text) 0 es) &&
      (if es.isEmpty then Decidable.decide (h.text = f.text) else true)

def legacyP1Check (O : Observations) (G H : Graph) (old : String) (p : Plan) : Bool :=
  Decidable.decide (H.length = (retained G p).length) &&
    ((retained G p).zip H).all (fun (f,h) => legacyP1File P D O old p f h)


theorem p1_file_equiv (O : Observations) (old : String) (p : Plan) (f h : File) :
    legacyP1File P D O old p f h = p1File P D O old p f h := by
  by_cases hn : inPair p f = true
  · simp [legacyP1File,p1File,hn]
  · have hn' : inPair p f = false := Bool.eq_false_iff.mpr hn
    by_cases hc : p1File P D O old p f h = true
    · have he : (if (edits P D O old p f).isEmpty then Decidable.decide (h.text = f.text) else true) = true := by
        split
        · next hh =>
          have empty : edits P D O old p f = [] := List.isEmpty_iff.mp hh
          simp [p1_unrewritten P D hn' empty hc]
        · rfl
      simpa only [legacyP1File,p1File,hn',Bool.false_eq_true,if_false,he,Bool.and_true] using hc
    · have hc' : p1File P D O old p f h = false := Bool.eq_false_iff.mpr hc
      simp only [legacyP1File,p1File,hn',Bool.false_eq_true,if_false] at hc' ⊢
      simp only [Bool.and_assoc,← Bool.and_assoc,hc',Bool.false_and]

theorem p1_check_equiv (O : Observations) (G H : Graph) (old : String) (p : Plan)
    (hl : H.length = (retained G p).length) :
    legacyP1Check P D O G H old p = p1Check P D O G H old p := by
  simp only [legacyP1Check,p1Check,hl,decide_true,Bool.true_and,p1_file_equiv]

private theorem folded_of_false {O : Observations} {xs H : Graph}
    (h : (xs.zip H).any (fun (f,h) => (xs.zip H).any (fun (g,k) => Decidable.decide (f ≠ g) &&
      Decidable.decide (O.foldedPath h.path = O.foldedPath k.path) && Decidable.decide (h.path ≠ f.path ∨ k.path ≠ g.path))) = false) :
    ∀ f a, (f,a) ∈ xs.zip H → ∀ g b, (g,b) ∈ xs.zip H → f ≠ g →
      a.path ≠ f.path ∨ b.path ≠ g.path → O.foldedPath a.path ≠ O.foldedPath b.path := by
  intro f a hfa g b hgb hfg hp he
  have bad : (xs.zip H).any (fun (f,h) => (xs.zip H).any (fun (g,k) => Decidable.decide (f ≠ g) &&
      Decidable.decide (O.foldedPath h.path = O.foldedPath k.path) && Decidable.decide (h.path ≠ f.path ∨ k.path ≠ g.path))) = true :=
    List.any_eq_true.mpr ⟨(f,a),hfa,List.any_eq_true.mpr ⟨(g,b),hgb,by simp [hfg,hp,he]⟩⟩
  exact Bool.noConfusion (h.symm.trans bad)

theorem post_equiv (O : Observations) (G H : Graph) (old : String) (p : Plan) (before : WF G) :
    legacyPost P D O G H old p = (postSafety P D O G H old p).map Sum.inl := by
  let pairs := (retained G p).zip H
  let folded := pairs.any (fun (f,h) => pairs.any (fun (g,k) => Decidable.decide (f ≠ g) &&
    Decidable.decide (O.foldedPath h.path = O.foldedPath k.path) && Decidable.decide (h.path ≠ f.path ∨ k.path ≠ g.path)))
  let clean := (touchedNames P D G old p).any (fun n => Date13.conflicted P D H (Date13.target P D H n))
  let identity := pairs.any (fun (f,h) => Decidable.decide (Date13.claim P D h ≠ intendedClaim P D old p f))
  let collision := pairs.any (fun (f,h) => pairs.any (fun (g,k) => Decidable.decide (f ≠ g) &&
    Decidable.decide (Date13.claim P D h = Date13.claim P D k) && Decidable.decide (Date13.claim P D f ≠ Date13.claim P D g)))
  let conflict := pairs.any (fun (f,h) => O.conflict f && Decidable.decide (h.path ≠ f.path) && Decidable.decide (h.text ≠ f.text))
  change (if H.length ≠ (retained G p).length then some (.inl Refusal.observationsP2)
    else if !(H.map File.path).Nodup then some (.inl Refusal.pathTaken)
    else if folded then some (.inl Refusal.pathTaken)
    else if clean then some (.inl Refusal.contested)
    else if identity then some (.inl Refusal.titleIdentity)
    else if collision then some (.inr ())
    else if conflict then some (.inl Refusal.conflictRewrite) else none) =
    (if H.length ≠ (retained G p).length then some Refusal.observationsP2
    else if folded then some Refusal.pathTaken
    else if clean then some Refusal.contested
    else if identity then some Refusal.titleIdentity
    else if conflict then some Refusal.conflictRewrite else none).map (fun r : Refusal => (Sum.inl r : Refusal ⊕ Unit))
  by_cases hl : H.length = (retained G p).length
  · by_cases hf : folded = true
    · simp only [hl,ne_eq,not_true_eq_false,if_false]
      by_cases hw : (H.map File.path).Nodup <;> simp only [hw,decide_true,decide_false,Bool.not_true,Bool.not_false,Bool.true_eq,if_true,Bool.false_eq_true,if_false,hf,Option.map_some]
    · have hf' : folded = false := Bool.eq_false_iff.mpr hf
      have hw := post_paths_nodup (retained_wf before p) hl.symm (folded_of_false hf')
      by_cases hc : clean = true
      · simp only [hl,ne_eq,not_true_eq_false,if_false,show (H.map File.path).Nodup from hw,decide_true,Bool.not_true,Bool.false_eq_true,hf',hc,Bool.true_eq,if_true,Option.map_some]
      · have hc' : clean = false := Bool.eq_false_iff.mpr hc
        by_cases hi : identity = true
        · simp only [hl,ne_eq,not_true_eq_false,if_false,show (H.map File.path).Nodup from hw,decide_true,Bool.not_true,Bool.false_eq_true,hf',hc',hi,Bool.true_eq,if_true,Option.map_some]
        · have hi' : identity = false := Bool.eq_false_iff.mpr hi
          have noNew := post_no_new_twins P D (O := O) hw (by
            intro n hn
            cases he : Date13.conflicted P D H (Date13.target P D H n) with
            | false => rfl
            | true =>
              have : clean = true := List.any_eq_true.mpr ⟨n,hn,he⟩
              exact False.elim (hc this)) (by
            intro f h hz
            apply Classical.byContradiction
            intro hn
            have : identity = true := List.any_eq_true.mpr ⟨(f,h),hz,by simp [hn]⟩
            exact hi this)
          have noBad : pairs.any (fun (f,h) => pairs.any (fun (g,k) => Decidable.decide (f ≠ g) &&
              Decidable.decide (Date13.claim P D h = Date13.claim P D k) && Decidable.decide (Date13.claim P D f ≠ Date13.claim P D g))) = false := by
            apply Bool.eq_false_iff.mpr
            intro bad
            obtain ⟨⟨f,h⟩,hfh,bad⟩ := List.any_eq_true.mp bad
            obtain ⟨⟨g,k⟩,hgk,bad⟩ := List.any_eq_true.mp bad
            simp only [Bool.and_eq_true,decide_eq_true_eq] at bad
            exact noNew f h hfh g k hgk bad.1.1 bad.2 bad.1.2
          simp only [hl,ne_eq,not_true_eq_false,if_false]
          simp only [show (H.map File.path).Nodup from hw,decide_true,Bool.not_true,Bool.false_eq_true,if_false]
          have nc : collision = false := noBad
          simp only [hf',hc',hi',Bool.false_eq_true,if_false,nc]
          split <;> rfl
  · simp only [hl,ne_eq,not_false_eq_true,if_true,Option.map_some]

theorem cancel_dominates_holder_dispatch (O : Observations) (G : Graph) (r : Request)
    (h : r.answer = some .cancel) : selectCore P D O G r = .refuse .cancelled := by
  simp [selectCore,h]

private theorem minimum_dispatch (m : Option File) (hm : m.isSome = true) :
    (match m with
    | some owner => (Result.refuse (.aliasHeld owner.path) : Selection)
    | none => .refuse .absentSource) = .refuse (.aliasHeld (m.get hm).path) := by
  cases m with
  | none => simp at hm
  | some f => rfl

theorem selection_core_equiv (O : Observations) (G : Graph) (r : Request) :
    legacySelectCore P D O G r = selectCore P D O G r := by
  unfold legacySelectCore selectCore
  by_cases cancel : r.answer = some .cancel
  · simp only [cancel,if_true]
  · simp only [cancel,if_false]
    split
    · rfl
    · split
      · rfl
      · try dsimp only
        split
        · simp_all only
        · simp_all only
          cases ha : r.answer with
          | none => rfl
          | some answer => cases answer with
            | confirm => rfl
            | cancel => exact False.elim (cancel ha)
        · simp_all only
          try dsimp only
          split
          · next held =>
            try dsimp only
            apply minimum_dispatch
          · rfl

theorem selection_commutes (O : Observations) (G : Graph) (r : Request) :
    attendance r.attended (legacySelectCore P D O G r) = select P D O G r := by
  rw [selection_core_equiv]; rfl


/-- Exact round-4 check order, with its deleted reason distinguished by inr (). -/
def legacyCheck (O : Observations) (G : Graph) (old : String) (p : Plan) (c : Candidate) : Option (Refusal ⊕ Unit) :=
  match preSafety P D O G old p with
  | some reason => some (.inl reason)
  | none => match legacyPost P D O G c.graph old p with
    | some reason => some reason
    | none =>
      if !(G ++ c.graph).all (validSites O) || !reportsDistinct (exceptionReports P D O G old p) then some (.inl .invalidSites)
      else if !legacyP1Check P D O G c.graph old p then some (.inl .textP1)
      else if !p2Check P D O G c.graph old p then some (.inl .observationsP2)
      else if !p3Check O G c.graph then some (.inl .saveP3)
      else none

theorem candidate_check_equiv (O : Observations) (G : Graph) (old : String) (p : Plan) (c : Candidate)
    (before : preSafety P D O G old p = none) :
    legacyCheck P D O G old p c = (candidateCheck P D O G old p c).map Sum.inl := by
  unfold legacyCheck
  rw [before,post_equiv P D O G c.graph old p (pre_wf P D before)]
  cases ha : postSafety P D O G c.graph old p with
  | some reason => simp [candidateCheck,ha]
  | none =>
    simp only [Option.map_none]
    rw [p1_check_equiv P D O G c.graph old p (post_cardinality P D ha)]
    unfold candidateCheck
    rw [ha]
    split <;> try rfl
    split <;> try rfl
    split <;> try rfl
    split <;> rfl

def liftResult {α : Type} : Result α → (Refusal ⊕ Unit) ⊕ (Question ⊕ α)
  | .refuse r => .inl (.inl r) | .ask q => .inr (.inl q) | .success x => .inr (.inr x)

structure LegacyDone where
  plan : Plan
  candidate : Candidate
  settings : Settings
  settingsStatus : SettingsStatus
  exceptions : List Report
  deriving DecidableEq, Repr

/-- Assemble the unchanged legacy presentation of a completed result. -/
def completePages (succeeded : Settings → Bool) (cfg : Settings) (old : String) : Outcome → Result LegacyDone
  | .refuse why => .refuse why
  | .ask q => .ask q
  | .success pages =>
    let step := finishSettings P D succeeded cfg old pages
    .success ⟨pages.plan,pages.candidate,step.settings,step.status,pages.exceptions⟩

def projectPages : ((Refusal ⊕ Unit) ⊕ (Question ⊕ LegacyDone)) → ((Refusal ⊕ Unit) ⊕ (Question ⊕ Done))
  | .inl reason => .inl reason
  | .inr (.inl q) => .inr (.inl q)
  | .inr (.inr d) => .inr (.inr ⟨d.plan,d.candidate,d.exceptions⟩)

theorem completion_projection (succeeded : Settings → Bool) (cfg : Settings) (old : String) (out : Outcome) :
    projectPages (liftResult (completePages P D succeeded cfg old out)) = liftResult out := by
  cases out with
  | refuse r => rfl
  | ask q => rfl
  | success d => rfl

def legacyPublish (O : Observations) (E : Engine) (succeeded : Settings → Bool) (G : Graph) (cfg : Settings) (old : String) (p : Plan) :
    (Refusal ⊕ Unit) ⊕ (Question ⊕ LegacyDone) :=
  match E.propose p with
  | none => .inl (.inl .writerFailed)
  | some c => match legacyCheck P D O G old p c with
    | some reason => .inl reason
    | none =>
      let intended := settingsAfter P D cfg old p
      let reports := exceptionReports P D O G old p
      if succeeded intended then .inr (.inr ⟨p,c,intended,.updated,reports⟩)
      else .inr (.inr ⟨p,c,cfg,.pagesDoneSettingsNotUpdated,reports⟩)

theorem publication_commutes (O : Observations) (E : Engine) (succeeded : Settings → Bool) (G : Graph) (cfg : Settings) (old : String) (p : Plan)
    (before : preSafety P D O G old p = none) :
    legacyPublish P D O E succeeded G cfg old p = liftResult (completePages P D succeeded cfg old (publish P D O E G old p)) := by
  unfold legacyPublish publish
  cases hp : E.propose p with
  | none => rfl
  | some c =>
    dsimp only
    rw [candidate_check_equiv P D O G old p c before]
    cases hc : candidateCheck P D O G old p c with
    | some reason => rfl
    | none =>
      simp only [Option.map_none]
      try dsimp only
      unfold completePages finishSettings
      try dsimp only
      split <;> rfl

def legacyDecide (O : Observations) (E : Engine) (succeeded : Settings → Bool) (G : Graph) (cfg : Settings) (r : Request) :
    (Refusal ⊕ Unit) ⊕ (Question ⊕ LegacyDone) :=
  match attendance r.attended (legacySelectCore P D O G r) with
  | .refuse reason => .inl (.inl reason)
  | .ask question => .inr (.inl question)
  | .success plan => legacyPublish P D O E succeeded G cfg r.source plan

/-- Every request has the same files, reports, question or distinguished refusal.
There is no caller safety/reader/writer hypothesis. -/
theorem decide_chapter_equiv (O : Observations) (E : Engine) (succeeded : Settings → Bool) (G : Graph) (cfg : Settings) (r : Request) :
    legacyDecide P D O E succeeded G cfg r = liftResult (completePages P D succeeded cfg r.source (Cut.decide P D O E G r)) := by
  unfold legacyDecide Cut.decide
  rw [selection_commutes]
  cases hs : select P D O G r with
  | refuse reason => rfl
  | ask question => rfl
  | success plan => exact publication_commutes P D O E succeeded G cfg r.source plan (selected_pre_safe P D hs)

/-- Every page outcome is independent of the settings record and its write result.
The historical result includes the old combined O8 completion, projected away. -/
theorem decide_pages_projection (O : Observations) (E : Engine) (succeeded : Settings → Bool)
    (G : Graph) (cfg : Settings) (r : Request) :
    projectPages (legacyDecide P D O E succeeded G cfg r) = liftResult (Cut.decide P D O E G r) := by
  rw [decide_chapter_equiv,completion_projection]


end Refactor4
end Cut
end CM
