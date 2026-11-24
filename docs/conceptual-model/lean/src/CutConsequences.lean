import CutTwins
namespace CM
namespace Cut
open Consolidated13
variable {K : Type} [DecidableEq K] (P : Inputs K) (D : Date13.Dates K)

theorem erase_text (t : Text) : textString (t.map eraseAtom) = textString t := by
  induction t with
  | nil => rfl
  | cons a rest ih => cases a <;> simp [eraseAtom,textString,atomText,eraseSite,ih]

theorem semantic_block_content (c : Content) :
    (semantic c).blocks.map (fun b => textString b.body) = c.blocks.map (fun b => textString b.body) := by
  simp [semantic,List.map_map,Function.comp_def,eraseBlock,normalizeBody,mapBody,compact_text,erase_text]

/-- Exact multiplicity on the actual reader's ordinary block-content strings,
including repetition and the ordinary leftover framing. -/
theorem done_block_content_count {O : Observations} {E : Engine} {G : Graph}
    {r : Request} {d : Done} (h : decide P D O E G r = .success d)
    {s t out : File} (hs : d.plan.source = some s) (ht : d.plan.survivor = some t)
    (hz : (t,out) ∈ (retained G d.plan).zip d.candidate.graph) (content : String) :
    let source := semantic (transformed P D O r.source d.plan s)
    let target := semantic (transformed P D O r.source d.plan t)
    ((O.content out).blocks.map (fun b => textString b.body)).count content =
      ((target.blocks).map (fun b => textString b.body)).count content +
      ((middle t.format source target).map (fun b => textString b.body)).count content +
      ((source.blocks).map (fun b => textString b.body)).count content := by
  have hc := (done_p4 P D h hs ht hz).blocks
  have raw := congrArg (List.map (fun b => textString b.body)) hc
  rw [← semantic_block_content (O.content out),raw]
  simp [List.map_append,List.map_map,Function.comp_def,List.count_append,Nat.add_assoc]

theorem done_alias_content_count {O : Observations} {E : Engine} {G : Graph}
    {r : Request} {d : Done} (h : decide P D O E G r = .success d)
    {s t out : File} (hs : d.plan.source = some s) (ht : d.plan.survivor = some t)
    (hz : (t,out) ∈ (retained G d.plan).zip d.candidate.graph) (key : Page K) :
    let names := (semantic (transformed P D O r.source d.plan t)).aliasNames ++
      (semantic (transformed P D O r.source d.plan s)).aliasNames
    (((semantic (O.content out)).aliasNames).map (Date13.target P D [])).count key =
      if key ∈ names.map (Date13.target P D []) then 1 else 0 := by
  rw [(done_p4 P D h hs ht hz).aliases]
  exact alias_union_count P D _ key

/-- The enclosing title rebind: one replacement prefix followed by the kept
suffix, with its nested sites respelled. This is the construction used by P2. -/
theorem title_rebinding (O : Observations) (fmt : Format) (old : String) (p : Plan)
    (skip : Bool) (whole : Site) (parts : List (String ⊕ Site)) :
    mapAtom P D O fmt old p skip (.title whole parts) =
      let rest := if skip then parts else
        (if titlePrefix P D old p whole = 0 then [] else [.inl p.destination]) ++
        (dropParts (titlePrefix P D old p whole) parts).map (fun part => match part with
          | .inl text => .inl text
          | .inr site => .inr (siteResult P D O fmt old p false site).1)
      let ps := compactParts rest
      let raw := String.join (ps.map partText)
      .title {whole with name := raw,native := raw} ps := rfl

/-- Every O4 safety clause and the actual content binding that includes the
constructive title rule. No checker-success or preservation premise is added. -/
theorem done_o4 {O : Observations} {E : Engine} {G : Graph}
    {r : Request} {d : Done} (h : decide P D O E G r = .success d) :
    BeforeNameSafety P D O G r.source d.plan ∧
    AfterNameSafety P D O G d.candidate.graph r.source d.plan ∧
    ∀ f k, (f,k) ∈ (retained G d.plan).zip d.candidate.graph →
      ContentMatches (expectedContent P D O r.source d.plan f) (semantic (O.content k))
        (headerFlexible d.plan f) := by
  refine ⟨(done_name_safety P D h).1,(done_name_safety P D h).2,?_⟩
  intro f k hz
  exact (done_pair_observations P D h hz).2.2.2

end Cut
end CM
