import CutAliasProofs
namespace CM
namespace Cut
open Consolidated13
variable {K : Type} [DecidableEq K] (P : Inputs K) (D : Date13.Dates K)

/-- Index/order pairing, without an output-origin certificate from the writer. -/
theorem mapped_zip_equal {A B C : Type} (a : A → C) (b : B → C)
    {xs : List A} {ys : List B} (he : xs.map a = ys.map b) :
    ∀ x y, (x,y) ∈ xs.zip ys → a x = b y := by
  induction xs generalizing ys with
  | nil => simp
  | cons x xs ih =>
    cases ys with
    | nil => simp at he
    | cons y ys =>
      simp only [List.map_cons,List.cons.injEq] at he
      intro u v hz
      simp only [List.zip_cons_cons,List.mem_cons] at hz
      rcases hz with hz | hz
      · cases hz; exact he.1
      · exact ih he.2 u v hz

theorem done_pair_observations {O : Observations} {E : Engine} {G : Graph}
    {r : Request} {d : Done} (h : decide P D O E G r = .success d)
    {f k : File} (hz : (f,k) ∈ (retained G d.plan).zip d.candidate.graph) :
    k.path = expectedPath P D O r.source d.plan f ∧ k.dir = f.dir ∧ k.format = f.format ∧
    ContentMatches (expectedContent P D O r.source d.plan f) (semantic (O.content k))
      (headerFlexible d.plan f) := by
  exact (done_reading P D h).2 f k hz

theorem done_p1 {O : Observations} {E : Engine} {G : Graph}
    {r : Request} {d : Done} (h : decide P D O E G r = .success d)
    {f k : File} (hz : (f,k) ∈ (retained G d.plan).zip d.candidate.graph)
    (hn : inPair d.plan f = false) :
    editGeometry f.text 0 (edits P D O r.source d.plan f) = true ∧
    bytes k.text = splice (bytes f.text) 0 (edits P D O r.source d.plan f) ∧
    (edits P D O r.source d.plan f = [] → k.text = f.text) := by
  obtain ⟨_,_,_,hc,_⟩ := done_checked P D h
  have hp := (accepted_properties P D hc).2.2.2.2.1
  simp only [p1Check] at hp
  have hf := List.all_eq_true.mp hp (f,k) hz
  exact ⟨(p1_splice P D hn hf).1,(p1_splice P D hn hf).2,
    fun he => p1_unrewritten P D hn he hf⟩

theorem unchanged_child_touched (G : Graph) (old : String) (p : Plan) (name : String)
    (hn : name ∈ allNames P G) (hd : (Date13.target P D [] name).1 ≠ .journal) (hc : (nsMatch P.key P.urlHasOrigin old name).isSome = true) :
    name ∈ touchedNames P D G old p := by
  simp only [touchedNames,List.mem_append,List.mem_cons,List.mem_filter]
  exact Or.inl (Or.inl (Or.inr ⟨Or.inr hn,by simp [hc,hd]⟩))

theorem declared_alias_touched (G : Graph) (old : String) (p : Plan) (source : File)
    (hs : p.source = some source) (alias : String) (hd : (Date13.target P D [] alias).1 ≠ .journal) (ha : alias ∈ P.aliasDecl source.format source.text) :
    alias ∈ touchedNames P D G old p := by
  simp [touchedNames,hs,ha,hd]

theorem done_exception_locations {O : Observations} {E : Engine} {G : Graph}
    {r : Request} {d : Done} (h : decide P D O E G r = .success d) :
    (d.exceptions.map (fun report => (report.path,report.start,report.stop,report.kind))).Nodup ∧
    ∀ f ∈ G, ∀ site ∈ contentSites (O.content f),
      let disposition := (siteResult P D O f.format r.source d.plan (skipFile P D O r.source d.plan f) site).2
      disposition = .skippedConflict ∨ disposition = .unspellable ∨ disposition = .normalizedKey →
      ⟨f.path,site.start,site.stop,disposition,site.kind⟩ ∈ d.exceptions := by
  obtain ⟨p,_,he,hc,hr⟩ := done_checked P D h
  have hd := accepted_reports_distinct P D hc
  have hr' : d.exceptions = exceptionReports P D O G r.source d.plan := by simpa [← he] using hr
  rw [hr']
  refine ⟨hd,?_⟩
  intro f hf site hs
  dsimp only
  intro hh
  unfold exceptionReports
  apply List.mem_flatMap.mpr
  refine ⟨f,hf,?_⟩
  apply List.mem_filterMap.mpr
  refine ⟨site,hs,?_⟩
  simp only
  rw [if_pos hh]

/-- This is the native header-line sequence, not a set of line meanings. -/
def headerLines (hs : List Header) : List String := hs.map (fun h => textString h.text)
def faithful (t : Text) : Prop := ∀ s ps, Atom.title s ps ∈ t → String.join (ps.map partText) = s.native

theorem parts_text (ps : List (String ⊕ Site)) :
    textString (ps.map partAtom) = String.join (ps.map partText) := by
  induction ps with
  | nil => rfl
  | cons p ps ih => cases p <;> simp [partAtom,partText,textString,atomText,String.join_cons,ih]

theorem textString_append (a b : Text) : textString (a ++ b) = textString a ++ textString b := by
  induction a with
  | nil => simp [textString]
  | cons h rest ih => simp [textString,ih,String.append_assoc]

theorem demote_text (t : Text) (hf : faithful t) : textString (t.flatMap demoteAtom) = textString t := by
  induction t with
  | nil => rfl
  | cons a rest ih =>
    have hr : faithful rest := fun s ps hm => hf s ps (List.mem_cons_of_mem a hm)
    simp only [List.flatMap_cons,textString_append,textString,ih hr]
    congr 1
    cases a with
    | text v => simp [demoteAtom,textString,atomText]
    | site v => simp only [demoteAtom]; split <;> simp [textString,atomText]
    | title v ps => exact (parts_text ps).trans (hf v ps (List.mem_cons_self ..))

theorem textString_intercalate (ts : List Text) :
    textString (List.intercalate [.text "\n"] ts) = "\n".intercalate (ts.map textString) := by
  induction ts with
  | nil => rfl
  | cons t ts ih =>
    cases ts with
    | nil => simp [List.intercalate]
    | cons u us =>
      simp only [List.intercalate_cons_cons,textString_append,ih]
      simp [String.intercalate_cons_cons,textString,atomText,String.append_assoc]

theorem leftover_exact_content (fmt : Format) (hs : List Header)
    (hf : ∀ h ∈ hs, faithful h.text) :
    textString (leftoverBody fmt hs) =
      (if fmt = .org then "\n#+BEGIN_QUOTE\n" else "\n") ++
      "\n".intercalate (headerLines hs) ++ (if fmt = .org then "\n#+END_QUOTE" else "") := by
  simp only [leftoverBody,compact_text,textString_append,leftoverPayload,textString_intercalate,List.map_map,Function.comp_def]
  have he : hs.map (fun h => textString (h.text.flatMap demoteAtom)) = headerLines hs := by
    apply List.map_congr_left
    intro h hh
    exact demote_text h.text (hf h hh)
  rw [he]
  cases fmt <;> simp [textString,atomText,String.append_assoc]

theorem mapped_faithful (O : Observations) (fmt : Format) (old : String) (p : Plan) (skip : Bool) (t : Text) :
    faithful (t.map (mapAtom P D O fmt old p skip)) := by
  intro whole parts hm
  obtain ⟨a,_,ha⟩ := List.mem_map.mp hm
  cases a with
  | text v => cases ha
  | site v => cases ha
  | title v ps => cases ha; rfl

theorem erased_faithful (t : Text) (hf : faithful t) : faithful (t.map eraseAtom) := by
  intro whole parts hm
  obtain ⟨a,ha,he⟩ := List.mem_map.mp hm
  cases a with
  | text v => cases he
  | site v => cases he
  | title v ps =>
    cases he
    have heq : (ps.map (fun a => match a with | .inl t => .inl t | .inr t => .inr (eraseSite t))).map partText = ps.map partText := by
      rw [List.map_map]
      apply List.map_congr_left
      intro x _
      cases x <;> rfl
    change String.join (_) = v.native
    exact (congrArg String.join heq).trans (hf v ps ha)

theorem compact_title_mem {whole : Site} {parts : List (String ⊕ Site)} {t : Text}
    (hm : Atom.title whole parts ∈ compact t) : Atom.title whole parts ∈ t := by
  induction t with
  | nil => simp [compact] at hm
  | cons a rest ih =>
    cases a with
    | title v ps =>
      simp only [compact,List.mem_cons] at hm
      rcases hm with he | hh
      · exact List.mem_cons.mpr (Or.inl he)
      · exact List.mem_cons_of_mem _ (ih hh)
    | site v =>
      exact List.mem_cons_of_mem _ (ih (by simpa [compact] using hm))
    | text v =>
      apply List.mem_cons_of_mem
      apply ih
      cases he : compact rest with
      | nil => simpa [compact,he] using hm
      | cons b bs =>
        cases b with
        | text u =>
          have hh : Atom.title whole parts ∈ bs := by simpa [compact,he] using hm
          simp [he,hh]
        | site site =>
          by_cases hv : v = "" <;> simpa [compact,he,hv] using hm
        | title site ps =>
          by_cases hv : v = "" <;> simpa [compact,he,hv] using hm

theorem compact_faithful (t : Text) (hf : faithful t) : faithful (compact t) :=
  fun s ps hm => hf s ps (compact_title_mem hm)

theorem expected_header_faithful (O : Observations) (old : String) (p : Plan) (f : File) :
    ∀ h ∈ (semantic (transformed P D O old p f)).header, faithful h.text := by
  intro h hh
  obtain ⟨original,ho,he⟩ := List.mem_map.mp hh
  cases he
  have ho' := (List.mem_filter.mp ho).1
  obtain ⟨raw,_,hr⟩ := List.mem_map.mp ho'
  cases hr
  exact compact_faithful _ (erased_faithful _ (mapped_faithful P D O f.format old p _ raw.text))

/-- Conservation is a sequence equation plus an ordered, multiplicity-preserving
partition. The only novel block is the ordinary O6 leftover content. -/
structure MergeConservation (fmt : Format) (s t out : Semantic) : Prop where
  header : out.header.Perm (t.header ++ (allocation s t).1)
  sourcePartition : List.Perm ((allocation s t).1 ++ (allocation s t).2) s.header
  sourceOrder : List.Sublist (allocation s t).1 s.header ∧ List.Sublist (allocation s t).2 s.header
  headerMultiplicity : ∀ h, (allocation s t).1.count h + (allocation s t).2.count h = s.header.count h
  aliases : out.aliasNames = uniqueNames P D (t.aliasNames ++ s.aliasNames)
  aliasDeclarations : out.aliasDeclarations = if out.aliasNames.isEmpty then 0 else 1
  blocks : out.blocks =
    (t.blocks ++ middle fmt s t ++ s.blocks.map (fun b => {b with underHeader := false}))
  leftover : middle fmt s t = if (allocation s t).2.isEmpty then []
    else [⟨0,leftoverBody fmt (allocation s t).2,false⟩]

theorem expected_merge_conserves (fmt : Format) (s t : Semantic) :
    MergeConservation P D fmt s t (mergeContent P D fmt s t) := by
  constructor
  · exact List.Perm.refl _
  · exact allocation_multiset _ _
  · exact allocation_order _ _
  · exact allocation_count_each _ _
  · rfl
  · rfl
  · rfl
  · simp only [middle,needsLeftover]; cases (allocation s t).2 <;> rfl

theorem block_content_multiplicity (fmt : Format) (s t : Semantic) (content : String) :
    ((mergeContent P D fmt s t).blocks.map (fun b => textString b.body)).count content =
      ((t.blocks).map (fun b => textString b.body)).count content +
      ((middle fmt s t).map (fun b => textString b.body)).count content +
      ((s.blocks).map (fun b => textString b.body)).count content := by
  simp [mergeContent,List.map_append,List.count_append,List.map_map,Function.comp_def,Nat.add_assoc]

theorem done_p4 {O : Observations} {E : Engine} {G : Graph}
    {r : Request} {d : Done} (h : decide P D O E G r = .success d)
    {s t k : File} (hs : d.plan.source = some s) (ht : d.plan.survivor = some t)
    (hz : (t,k) ∈ (retained G d.plan).zip d.candidate.graph) :
    MergeConservation P D t.format
      (semantic (transformed P D O r.source d.plan s))
      (semantic (transformed P D O r.source d.plan t)) (semantic (O.content k)) := by
  have hc := (done_pair_observations P D h hz).2.2.2
  have he := hc.1
  have hh := hc.2
  simp only [expectedContent,headerFlexible,hs,ht,beq_self_eq_true,ite_true] at he hh
  have base := expected_merge_conserves P D t.format
    (semantic (transformed P D O r.source d.plan s)) (semantic (transformed P D O r.source d.plan t))
  constructor
  · exact hh
  · exact base.sourcePartition
  · exact base.sourceOrder
  · exact base.headerMultiplicity
  · exact (congrArg Semantic.aliasNames he).trans base.aliases
  · have hn := congrArg Semantic.aliasNames he
    exact (congrArg Semantic.aliasDeclarations he).trans (base.aliasDeclarations.trans (by rw [hn]))
  · exact congrArg Semantic.blocks he
  · exact base.leftover

theorem done_p4_leftover_content {O : Observations} {E : Engine} {G : Graph}
    {r : Request} {d : Done} (h : decide P D O E G r = .success d)
    {source target : File} (hs : d.plan.source = some source) (ht : d.plan.survivor = some target) :
    let s := semantic (transformed P D O r.source d.plan source)
    let t := semantic (transformed P D O r.source d.plan target)
    textString (leftoverBody target.format (allocation s t).2) =
      (if target.format = .org then "\n#+BEGIN_QUOTE\n" else "\n") ++
      "\n".intercalate (headerLines (allocation s t).2) ++
      (if target.format = .org then "\n#+END_QUOTE" else "") := by
  apply leftover_exact_content
  intro header hh
  apply expected_header_faithful P D O r.source d.plan source header
  exact (allocation_order _ _).2.subset hh

theorem fileless_no_creation {O : Observations} {E : Engine} {G : Graph}
    {r : Request} {d : Done} (h : decide P D O E G r = .success d)
    (hs : d.plan.source = none) :
    d.candidate.graph.length = G.length := by
  have he := (done_reading P D h).1
  simpa [fileless_cardinality G d.plan hs] using he

theorem done_day_files {O : Observations} {E : Engine} {G : Graph}
    {r : Request} {d : Done} (h : decide P D O E G r = .success d)
    {f k : File} (hz : (f,k) ∈ (retained G d.plan).zip d.candidate.graph)
    (hd : (Date13.claim P D f).1 = .journal) :
    Date13.claim P D k = Date13.claim P D f ∧ k.path = f.path := by
  have ha := (done_name_safety P D h).2.identity f k hz
  exact ⟨by simpa [intendedClaim,hd] using ha,
    by simpa [expectedPath,hd] using (done_pair_observations P D h hz).1⟩

end Cut
end CM
