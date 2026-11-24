import CutRedundancy
namespace CM
namespace Cut
open Consolidated13
variable {K : Type} [DecidableEq K] (P : Inputs K) (D : Date13.Dates K)

def touchesFile (O : Observations) (old : String) (p : Plan) (f : File) : Bool :=
  decide (some f = p.source ∨ some f = p.survivor) ||
    decide (expectedPath P D O old p f ≠ f.path ∨ expectedContent P D O old p f ≠ semantic (O.content f))

/-- Named, finite checks on BEFORE and the intended path/identity images. -/
def preSafety (O : Observations) (G : Graph) (old : String) (p : Plan) : Option Refusal :=
  let names := touchedNames P D G old p
  let m := nameMap P D old p.destination
  if !(G.map File.path).Nodup then some .malformedGraph
  else if names.any (fun s => Date13.conflicted P D G (Date13.target P D G s)) then some .contested
  else if claimed P D G p.destination &&
      decide (P.key old ≠ P.key p.destination) && (nsMatch P.key P.urlHasOrigin old p.destination).isSome then some .ownChild
  else if names.any (fun s =>
    decide (Date13.target P D G (m s) ≠ Date13.target P D G s) &&
    captureClaimed P D G old p s &&
    !p.survivor.any (fun f => decide (Date13.claim P D f = Date13.target P D G (m s)))) then some .capture
  else if (match p.source,p.survivor with
    | some s,some t => decide (s.format ≠ t.format)
    | _,_ => false) then some .mixedFormats
  else if p.survivor.isSome && (p.source.toList ++ p.survivor.toList).any O.conflict then some .conflictPair
  else if leftoverUnkeepable P D O old p then some .leftoverReference
  else if G.any (fun f => touchesFile P D O old p f && !O.supported f) then some .unsupportedInput
  else if G.any (fun f => f.format == .org && decide (O.save f ≠ f) &&
    touchesFile P D O old p f) then some .unstableOrgInput
  else if G.any (fun f => (O.content f).header.any (fun h => h.text.any (fun a => match a with
    | .title whole parts =>
      let n := titlePrefix P D old p whole
      parts.any (fun part => match part with
        | .inl _ => false
        | .inr site => decide (site.start < whole.start + n) && decide (mappedName P D old p site.name ≠ site.name))
    | _ => false))) then some .titleIdentity
  else if headerChildrenWithoutLeftover P D O old p then some .headerChildren
  else if (retained G p).any (fun f => G.any (fun g => decide (f ≠ g) &&
    decide (expectedPath P D O old p f ≠ f.path) &&
    decide (O.foldedPath (expectedPath P D O old p f) = O.foldedPath g.path))) then some .pathTaken
  else none

/-- Safety over actual candidate files. Origin binding is the retained input
order, subsequently checked by P2; the writer supplies no origin permutation. -/
def postSafety (O : Observations) (G H : Graph) (old : String) (p : Plan) : Option Refusal :=
  let pairs := (retained G p).zip H
  if H.length ≠ (retained G p).length then some .observationsP2
  else if pairs.any (fun (f,h) => pairs.any (fun (g,k) => decide (f ≠ g) &&
      decide (O.foldedPath h.path = O.foldedPath k.path) &&
      decide (h.path ≠ f.path ∨ k.path ≠ g.path))) then some .pathTaken
  else if (touchedNames P D G old p).any (fun n => Date13.conflicted P D H
      (Date13.target P D H n)) then some .contested
  else if pairs.any (fun (f,h) => decide (Date13.claim P D h ≠
      intendedClaim P D old p f)) then some .titleIdentity
  else if pairs.any (fun (f,h) => O.conflict f && decide (h.path ≠ f.path) && decide (h.text ≠ f.text)) then some .conflictRewrite
  else none

def authorize (O : Observations) (G : Graph) (r : Request) (p : Plan) (question : Option Question) : Selection :=
  match preSafety P D O G r.source p with
  | some reason => .refuse reason
  | none =>
    match question with
    | none => .success p
    | some q => .ask q

def survivorSpelling (G : Graph) (f : File) : Option String :=
  match P.title f.format f.text with
  | some title => if Date13.target P D G title = Date13.claim P D f then some title
      else if Date13.target P D G (P.stem f) = Date13.claim P D f then some (P.stem f) else none
  | none => if Date13.target P D G (P.stem f) = Date13.claim P D f then some (P.stem f) else none

def mergeSelection (O : Observations) (G : Graph) (r : Request) (s : Option File) (t : File)
    (question : Option Question) : Selection :=
  match survivorSpelling P D G t with
  | none => .refuse .unknownSurvivorSpelling
  | some n => authorize P D O G r ⟨n,s,some t⟩ question

theorem alias_held_min_exists (xs : Graph) (source : Option File)
    (h : (xs.filter (fun f => some f != source)).isEmpty = false) :
    (minBy Date13.pathLe xs).isSome = true := by
  apply Option.isSome_iff_ne_none.mpr
  intro hn
  have he := minBy_eq_none.mp hn
  simp [he] at h

def selectCore (O : Observations) (G : Graph) (r : Request) : Selection :=
  if r.answer = some .cancel then .refuse .cancelled
  else if !eligible P D G r.source then .refuse .absentSource
  else if (Date13.target P D G r.source).1 = .journal then .refuse .daySource
  else
    let s := sourceFile P D G r.source
    let holders := (Date13.claimants P D G (Date13.target P D G r.typed)).filter (fun f => some f != s)
    match holders with
    | _ :: _ :: _ => .refuse .contested
    | [t] =>
      match r.answer with
      | none => mergeSelection P D O G r s t (some .confirmMerge)
      | some _ => mergeSelection P D O G r s t none
    | [] =>
      let declarers := (Date13.declarers P D G (Date13.target P D G r.typed)).filter (fun f => some f != s)
      let selfClaims := s.any (fun f => decide (Date13.claim P D f = Date13.target P D G r.typed))
      if ha : !selfClaims && !declarers.isEmpty then
        let owners := Date13.declarers P D G (Date13.target P D G r.typed)
        let owner := (minBy Date13.pathLe owners).get
          (alias_held_min_exists owners s (by cases he : declarers.isEmpty <;> simp_all))
        .refuse (.aliasHeld owner.path)
      else if r.answer.isSome then .refuse .unavailableAnswer
      else authorize P D O G r ⟨r.typed,s,none⟩ none

/-- The sole attendance gate applies equally to supplied answers. -/
def attendance (attended : Bool) : Selection → Selection
  | .ask q => if attended then .ask q else .refuse .unattended
  | .success p => if !attended && p.survivor.isSome then .refuse .unattended else .success p
  | .refuse why => .refuse why

def select (O : Observations) (G : Graph) (r : Request) : Selection :=
  attendance r.attended (selectCore P D O G r)

theorem selection_total (O : Observations) (G : Graph) (r : Request) :
    ∃ x, select P D O G r = x ∧ ∀ y, select P D O G r = y → y = x := by
  exact ⟨select P D O G r,rfl,fun _ h => h.symm⟩

theorem selection_deterministic (O : Observations) (G : Graph) (r : Request) {x y : Selection}
    (hx : select P D O G r = x) (hy : select P D O G r = y) : x = y := hx.symm.trans hy

theorem authorized_safe {O : Observations} {G : Graph} {r : Request} {p p' : Plan} {q : Option Question}
    (h : authorize P D O G r p q = .success p') : preSafety P D O G r.source p' = none := by
  unfold authorize at h
  split at h
  · contradiction
  · split at h
    · cases h; assumption
    · contradiction

theorem unattended_no_question (O : Observations) (G : Graph) (r : Request) (ha : r.attended = false) :
    ∀ q, select P D O G r ≠ .ask q := by
  intro q
  unfold select
  cases selectCore P D O G r <;> simp [attendance,ha]
  split <;> simp

theorem unattended_no_merge (O : Observations) (G : Graph) (r : Request) (ha : r.attended = false)
    {p : Plan} (h : select P D O G r = .success p) : p.survivor = none := by
  unfold select at h
  cases he : selectCore P D O G r with
  | refuse why => simp [he,attendance] at h
  | ask q => simp [he,attendance,ha] at h
  | success plan =>
    simp only [he,attendance,ha,Bool.not_false,Bool.true_and] at h
    split at h
    · contradiction
    · next hn => cases h; simpa using hn

theorem attendance_applied {a : Bool} {x : Selection} {p : Plan}
    (h : attendance a x = .success p) : x = .success p := by
  cases x with
  | refuse why => simp [attendance] at h
  | ask q => simp only [attendance] at h; split at h <;> contradiction
  | success plan =>
    simp only [attendance] at h
    split at h
    · contradiction
    · exact h

theorem selected_pre_safe {O : Observations} {G : Graph} {r : Request} {p : Plan}
    (h : select P D O G r = .success p) : preSafety P D O G r.source p = none := by
  have hc := attendance_applied h
  simp only [selectCore] at hc
  split at hc
  · contradiction
  · split at hc
    · contradiction
    · split at hc
      · contradiction
      · try dsimp only at hc
        split at hc
        · contradiction
        · split at hc
          · unfold mergeSelection at hc
            split at hc
            · contradiction
            · exact authorized_safe P D hc
          · unfold mergeSelection at hc
            split at hc
            · contradiction
            · exact authorized_safe P D hc
        · try dsimp only at hc
          split at hc
          · contradiction
          · split at hc
            · contradiction
            · exact authorized_safe P D hc

theorem selected_source {O : Observations} {G : Graph} {r : Request} {p : Plan}
    (h : select P D O G r = .success p) :
    eligible P D G r.source = true ∧ (Date13.target P D G r.source).1 ≠ .journal := by
  have hc := attendance_applied h
  by_cases he : eligible P D G r.source = true
  · by_cases hd : (Date13.target P D G r.source).1 = .journal
    · simp [selectCore,he,hd] at hc
      split at hc <;> contradiction
    · exact ⟨he,hd⟩
  · simp [selectCore,he] at hc
    split at hc <;> contradiction


theorem pre_wf {O : Observations} {G : Graph} {old : String} {p : Plan}
    (h : preSafety P D O G old p = none) : WF G := by
  apply Classical.byContradiction
  intro hw
  have hn : ¬(G.map File.path).Nodup := hw
  simp [preSafety,hn] at h

theorem pre_touched_clean {O : Observations} {G : Graph} {old : String} {p : Plan}
    (h : preSafety P D O G old p = none) :
    ∀ s ∈ touchedNames P D G old p, Date13.conflicted P D G (Date13.target P D G s) = false := by
  have hw := pre_wf P D h
  intro s hs
  cases he : Date13.conflicted P D G (Date13.target P D G s) with
  | false => rfl
  | true =>
    have ha : (touchedNames P D G old p).any (fun n => Date13.conflicted P D G (Date13.target P D G n)) = true := List.any_eq_true.mpr ⟨s,hs,he⟩
    simp [preSafety,show (G.map File.path).Nodup from hw,ha] at h

theorem post_cardinality {O : Observations} {G H : Graph} {old : String} {p : Plan}
    (h : postSafety P D O G H old p = none) : H.length = (retained G p).length := by
  unfold postSafety at h
  dsimp only at h
  split at h
  · contradiction
  · exact Classical.not_not.mp (by assumption)

theorem post_folded {O : Observations} {G H : Graph} {old : String} {p : Plan}
    (h : postSafety P D O G H old p = none) :
    ∀ f a, (f,a) ∈ (retained G p).zip H → ∀ g b, (g,b) ∈ (retained G p).zip H → f ≠ g →
      a.path ≠ f.path ∨ b.path ≠ g.path → O.foldedPath a.path ≠ O.foldedPath b.path := by
  unfold postSafety at h
  dsimp only at h
  split at h <;> try contradiction
  split at h
  · contradiction
  · next hn =>
    intro f a hfa g b hgb hfg hp he
    apply hn
    exact List.any_eq_true.mpr ⟨(f,a),hfa,List.any_eq_true.mpr ⟨(g,b),hgb,by simp [hfg,hp,he]⟩⟩

theorem post_wf {O : Observations} {G H : Graph} {old : String} {p : Plan}
    (before : WF G) (h : postSafety P D O G H old p = none) : WF H :=
  post_paths_nodup (retained_wf before p) (post_cardinality P D h).symm (post_folded P D h)

end Cut
end CM
