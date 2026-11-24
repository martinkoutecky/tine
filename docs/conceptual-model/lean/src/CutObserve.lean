import CutTypes
namespace CM
namespace Cut
open Consolidated13
variable {K : Type} [DecidableEq K] (P : Inputs K) (D : Date13.Dates K)

def nameMap (old destination s : String) : String :=
  if (Date13.target P D [] s).1 = .journal then s else nsSubst P.key P.urlHasOrigin old destination s

def mappedName (old : String) (p : Plan) (s : String) :=
  nameMap P D old p.destination s

def siteResult (O : Observations) (fmt : Format) (old : String) (p : Plan)
    (skip : Bool) (s : Site) : Site × Disposition :=
  let n := mappedName P D old p s.name
  if s.kind = .task ∨ s.kind = .priority ∨ (Date13.target P D [] s.name).1 = .journal then (s,.fixed)
  else if n = s.name then (s,.unchanged)
  else if skip then (s,.skippedConflict)
  else if s.context = .splitLine then (s,.unspellable)
  else
    let k := if s.kind = .propertyKey then O.keySpelling n else some n
    match k with
    | none => (s,.unspellable)
    | some k => match O.spell fmt s k with
      | none => (s,.unspellable)
      | some raw => ({s with name := k,native := raw},if k = n then .substituted else .normalizedKey)

def compact : Text → Text
  | [] => []
  | .site s :: rest => .site s :: compact rest
  | .title s ps :: rest => .title s ps :: compact rest
  | .text t :: rest => match compact rest with
    | .text u :: tail => .text (t ++ u) :: tail
    | other => if t = "" then other else .text t :: other

def compactParts (ps : List (String ⊕ Site)) : List (String ⊕ Site) :=
  (compact (ps.map partAtom)).map (fun a => match a with
    | .text t => .inl t | .site t => .inr t | .title t _ => .inl t.native)

/-- The replaced title prefix is measured on the original spelling. References
in that prefix are a separate refusal; references in the suffix are transformed. -/
def titlePrefix (old : String) (p : Plan) (s : Site) : Nat :=
  if (Date13.target P D [] s.name).1 == .journal then 0 else
  match nsMatch P.key P.urlHasOrigin old s.name with
  | none => 0
  | some i => bytes ("/".intercalate (((Params.splitSlash s.name.toList).map String.ofList).take (i+1))) |>.length

def dropParts : Nat → List (String ⊕ Site) → List (String ⊕ Site)
  | _, [] => []
  | n, a :: rest =>
    if n = 0 then a :: rest else
    let len := (bytes (partText a)).length
    if len ≤ n then dropParts (n-len) rest
    else .inl (byteString ((bytes (partText a)).drop n)) :: rest

def titleParts (O : Observations) (fmt : Format) (old : String) (p : Plan)
    (skip : Bool) (s : Site) (parts : List (String ⊕ Site)) : List (String ⊕ Site) :=
  if skip then parts else
  let n := titlePrefix P D old p s
  (if n = 0 then [] else [.inl p.destination]) ++
    (dropParts n parts).map (fun a => match a with
      | .inl t => .inl t | .inr t => .inr (siteResult P D O fmt old p false t).1)

def mapAtom (O : Observations) (fmt : Format) (old : String) (p : Plan) (skip : Bool) : Atom → Atom
  | .text t => .text t
  | .site s => .site (siteResult P D O fmt old p skip s).1
  | .title s parts =>
    let ps := compactParts (titleParts P D O fmt old p skip s parts)
    let raw := String.join (ps.map partText)
    .title {s with name := raw,native := raw} ps

def mapHeader (O : Observations) (fmt : Format) (old : String) (p : Plan) (skip : Bool) (h : Header) : Header :=
  {h with
    text := h.text.map (mapAtom P D O fmt old p skip)
    key := match (textSites h.text).find? (fun s => s.kind == .propertyKey) with
      | some s => some (siteResult P D O fmt old p skip s).1.name
      | none => h.key}

/-- A key site determines the parser key after the same O1/O7 transformation. -/
theorem mapHeader_key_site (O : Observations) (fmt : Format) (old : String)
    (p : Plan) (skip : Bool) (h : Header) (s : Site)
    (hs : (textSites h.text).find? (fun t => t.kind == .propertyKey) = some s) :
    (mapHeader P D O fmt old p skip h).key =
      some (siteResult P D O fmt old p skip s).1.name := by
  simp [mapHeader, hs]

/-- No key site means no key rename, including built-in keys and non-properties. -/
theorem mapHeader_key_no_site (O : Observations) (fmt : Format) (old : String)
    (p : Plan) (skip : Bool) (h : Header)
    (hs : (textSites h.text).find? (fun t => t.kind == .propertyKey) = none) :
    (mapHeader P D O fmt old p skip h).key = h.key := by
  simp [mapHeader, hs]

def mapBody (f : Atom → Atom) (t : Text) : Text := t.map f
def mapContent (O : Observations) (fmt : Format) (old : String) (p : Plan) (skip : Bool) (c : Content) : Content :=
  ⟨c.header.map (mapHeader P D O fmt old p skip),
    c.blocks.map (fun b => {b with body := mapBody (mapAtom P D O fmt old p skip) b.body})⟩

def normalizeBody (t : Text) : Text := compact t

def eraseSite (s : Site) : Site := {s with start := 0,stop := 0}
def eraseAtom : Atom → Atom
  | .text t => .text t
  | .site s => .site (eraseSite s)
  | .title s ps => .title (eraseSite s) (ps.map (fun a => match a with | .inl t => .inl t | .inr t => .inr (eraseSite t)))
def eraseHeader (h : Header) : Header := {h with text := compact (h.text.map eraseAtom)}
def eraseBlock (b : Block) : Block := {b with body := normalizeBody (mapBody eraseAtom b.body)}

/-- Every block has the same raw outline-content observation. -/
structure Semantic where
  header : List Header
  aliasNames : List String
  aliasDeclarations : Nat
  blocks : List Block
  deriving DecidableEq, Repr

def meaningfulHeader (h : Header) : Bool :=
  let text := (textString h.text).toList.filter (fun c => !c.isWhitespace)
  !text.isEmpty && text != ['-'] && text != ['*']

def semantic (c : Content) : Semantic :=
  ⟨(c.header.filter (fun h => meaningfulHeader h && (h.role != .aliases || (textSites h.text).all (fun s => s.kind != .aliasName)))).map eraseHeader,aliases c,
    (c.header.filter (fun h => h.role == .aliases && (textSites h.text).any (fun s => s.kind == .aliasName))).length,
    c.blocks.map eraseBlock⟩

def uniqueNames (xs : List String) : List String :=
  xs.foldl (fun kept n => if kept.any (fun k => Date13.target P D [] k == Date13.target P D [] n) then kept else kept ++ [n]) []

/-- Split source headers in input order, reserving each joined key immediately. -/
def allocate : List String → List Header → List Header × List Header
  | _, [] => ([],[])
  | keys, h :: hs =>
    if h.role == .property && !keys.contains (headerKey h) then
      let rest := allocate (keys ++ [headerKey h]) hs
      (h :: rest.1,rest.2)
    else
      let rest := allocate keys hs
      (rest.1,h :: rest.2)

def demoteAtom : Atom → Text
  | .site s => if s.kind = .title then [.text s.native] else [.site s]
  | .title _ ps => ps.map partAtom
  | .text t => [.text t]

def leftoverPayload (hs : List Header) : Text :=
  List.intercalate [.text "\n"] (hs.map (fun h => h.text.flatMap demoteAtom))

def leftoverBody (fmt : Format) (hs : List Header) : Text :=
  compact ([.text (if fmt = .org then "\n#+BEGIN_QUOTE\n" else "\n")] ++
    leftoverPayload hs ++ (if fmt = .org then [.text "\n#+END_QUOTE"] else []))

def needsLeftover (hs : List Header) : Bool := !hs.isEmpty

def allocation (source target : Semantic) :=
  allocate (target.header.filterMap (fun h => if h.role == .property then some (headerKey h) else none)) source.header

def middle (fmt : Format) (s t : Semantic) : List Block :=
  if needsLeftover (allocation s t).2 then [⟨0,leftoverBody fmt (allocation s t).2,false⟩] else []


def mergeContent (fmt : Format) (source target : Semantic) : Semantic :=
  let a := allocation source target
  let names := uniqueNames P D (target.aliasNames ++ source.aliasNames)
  ⟨target.header ++ a.1,names,if names.isEmpty then 0 else 1,
    target.blocks ++ middle fmt source target ++
      source.blocks.map (fun b => {b with underHeader := false})⟩

def retained (G : Graph) (p : Plan) : Graph :=
  if p.survivor.isSome then G.filter (fun f => some f != p.source) else G

def movedPath (O : Observations) (f : File) (newName : String) := O.parent f ++ O.encode newName ++ O.extension f

def expectedPath (O : Observations) (old : String) (p : Plan) (f : File) : String :=
  let n := mappedName P D old p (P.name f)
  if (Date13.claim P D f).1 = .journal ∨ some f = p.survivor ∨ n = P.name f then f.path else movedPath O f n

def intendedClaim (old : String) (p : Plan) (f : File) :=
  if (Date13.claim P D f).1 = .journal ∨ some f = p.survivor then Date13.claim P D f
  else Date13.target P D [] (nameMap P D old p.destination (P.name f))

def skipFile (O : Observations) (old : String) (p : Plan) (f : File) : Bool :=
  O.conflict f && decide (some f ≠ p.source ∧ some f ≠ p.survivor) && decide (expectedPath P D O old p f = f.path)

def transformed (O : Observations) (old : String) (p : Plan) (f : File) : Content :=
  mapContent P D O f.format old p (skipFile P D O old p f) (O.content f)

def headerChildrenWithoutLeftover (O : Observations) (old : String) (p : Plan) : Bool :=
  match p.source,p.survivor with
  | some s,some t =>
    let a := allocation (semantic (transformed P D O old p s)) (semantic (transformed P D O old p t))
    (O.content s).blocks.any Block.underHeader && !needsLeftover a.2
  | _,_ => false

def expectedContent (O : Observations) (old : String) (p : Plan) (f : File) : Semantic :=
  let t := fun x => semantic (transformed P D O old p x)
  match p.source,p.survivor with
  | some s,some target => if f = target then mergeContent P D target.format (t s) (t target) else t f
  | _,_ => t f

structure ExpectedFile where
  path : String
  dir : Dir
  format : Format
  content : Semantic
  deriving DecidableEq, Repr

def expected (O : Observations) (G : Graph) (old : String) (p : Plan) : List ExpectedFile :=
  (retained G p).map (fun f => ⟨expectedPath P D O old p f,f.dir,f.format,expectedContent P D O old p f⟩)
def actual (O : Observations) (H : Graph) : List ExpectedFile :=
  H.map (fun f => ⟨f.path,f.dir,f.format,semantic (O.content f)⟩)
def headerFlexible (p : Plan) (f : File) : Bool :=
  match p.source,p.survivor with
  | some _,some t => f == t
  | _,_ => false

def ContentMatches (intended observed : Semantic) (flexible : Bool) : Prop :=
  {observed with header := intended.header} = intended ∧
    (if flexible then observed.header.Perm intended.header else observed.header = intended.header)
instance (a b : Semantic) (flexible : Bool) : Decidable (ContentMatches a b flexible) :=
  inferInstanceAs (Decidable (_ ∧ _))

def pairMatches (O : Observations) (old : String) (p : Plan) (f h : File) : Prop :=
  h.path = expectedPath P D O old p f ∧ h.dir = f.dir ∧ h.format = f.format ∧
    ContentMatches (expectedContent P D O old p f) (semantic (O.content h)) (headerFlexible p f)
instance (O : Observations) (old : String) (p : Plan) (f h : File) : Decidable (pairMatches P D O old p f h) :=
  inferInstanceAs (Decidable (_ ∧ _ ∧ _ ∧ _))

def p2Check (O : Observations) (G H : Graph) (old : String) (p : Plan) : Bool :=
  decide (H.length = (retained G p).length) &&
    ((retained G p).zip H).all (fun (f,h) => decide (pairMatches P D O old p f h))


/-- A demoted reference must survive parsing the prescribed native leftover.
The fragment interpretation is the same one used by Reading. -/
def leftoverUnkeepable (O : Observations) (old : String) (p : Plan) : Bool :=
  match p.source,p.survivor with
  | some s,some t =>
    let source := semantic (transformed P D O old p s)
    let target := semantic (transformed P D O old p t)
    let body := leftoverBody t.format (allocation source target).2
    let occurrences : List Site → List (OccKind × String) := fun (sites : List Site) => sites.filterMap (fun site => (occKind site.kind).map (fun kind => (kind,site.name)))
    needsLeftover (allocation source target).2 &&
      decide (occurrences (textSites body) ≠ occurrences (contentSites (O.read t.format (textString body))))
  | _,_ => false

def exceptionReports (O : Observations) (G : Graph) (old : String) (p : Plan) : List Report :=
  G.flatMap (fun f => (contentSites (O.content f)).filterMap (fun s =>
    let d := (siteResult P D O f.format old p (skipFile P D O old p f) s).2
    if d = .skippedConflict ∨ d = .unspellable ∨ d = .normalizedKey
    then some ⟨f.path,s.start,s.stop,d,s.kind⟩ else none))


theorem p2_sound {O : Observations} {G H : Graph} {old : String} {p : Plan}
    (h : p2Check P D O G H old p = true) :
    H.length = (retained G p).length ∧
    ∀ f k, (f,k) ∈ (retained G p).zip H → pairMatches P D O old p f k := by
  simp only [p2Check,Bool.and_eq_true,decide_eq_true_eq] at h
  exact ⟨h.1,fun f k hz => of_decide_eq_true (List.all_eq_true.mp h.2 (f,k) hz)⟩

theorem fixed_site (O : Observations) (fmt : Format) (old : String) (p : Plan) (skip : Bool) (s : Site)
    (h : s.kind = .task ∨ s.kind = .priority ∨ (Date13.target P D [] s.name).1 = .journal) :
    siteResult P D O fmt old p skip s = (s,.fixed) := by simp [siteResult,h]

theorem site_classification (O : Observations) (fmt : Format) (old : String) (p : Plan) (skip : Bool) (s : Site) :
    (siteResult P D O fmt old p skip s).2 = .fixed ∨
    (siteResult P D O fmt old p skip s).2 = .skippedConflict ∨
    (siteResult P D O fmt old p skip s).2 = .unspellable ∨
    (siteResult P D O fmt old p skip s).2 = .normalizedKey ∨
    (siteResult P D O fmt old p skip s).1.name = mappedName P D old p s.name := by
  by_cases hf : s.kind = .task ∨ s.kind = .priority ∨ (Date13.target P D [] s.name).1 = .journal
  · simp [siteResult,hf]
  by_cases hm : mappedName P D old p s.name = s.name
  · simp [siteResult,hf,hm]
  by_cases hs : skip = true
  · simp [siteResult,hf,hm,hs]
  by_cases hc : s.context = .splitLine
  · simp [siteResult,hf,hm,hs,hc]
  cases hk : (if s.kind = .propertyKey then O.keySpelling (mappedName P D old p s.name) else some (mappedName P D old p s.name)) with
  | none => simp [siteResult,hf,hm,hs,hc,hk]
  | some k =>
    cases hn : O.spell fmt s k with
    | none => simp [siteResult,hf,hm,hs,hc,hk,hn]
    | some raw =>
      by_cases he : k = mappedName P D old p s.name
      · subst k; simp [siteResult,hf,hm,hs,hc,hk,hn]
      · simp [siteResult,hf,hm,hs,hc,hk,hn,he]

theorem allocation_partition (keys : List String) (hs : List Header) :
    (allocate keys hs).1.length + (allocate keys hs).2.length = hs.length := by
  induction hs generalizing keys with
  | nil => rfl
  | cons h hs ih =>
    simp only [allocate]
    split
    · simp only [List.length_cons]; have := ih (keys ++ [headerKey h]); omega
    · simp only [List.length_cons]; have := ih keys; omega

theorem allocation_multiset (keys : List String) (hs : List Header) :
    List.Perm ((allocate keys hs).1 ++ (allocate keys hs).2) hs := by
  induction hs generalizing keys with
  | nil => exact List.Perm.refl []
  | cons h hs ih =>
    simp only [allocate]
    split
    · exact List.Perm.cons h (ih _)
    · exact (List.perm_middle).trans (List.Perm.cons h (ih _))

theorem allocation_order (keys : List String) (hs : List Header) :
    List.Sublist (allocate keys hs).1 hs ∧ List.Sublist (allocate keys hs).2 hs := by
  induction hs generalizing keys with
  | nil => exact ⟨.slnil,.slnil⟩
  | cons h hs ih =>
    simp only [allocate]
    split
    · exact ⟨(ih _).1.cons_cons h,(ih _).2.cons h⟩
    · exact ⟨(ih _).1.cons h,(ih _).2.cons_cons h⟩

theorem allocation_count_each (keys : List String) (hs : List Header) (h : Header) :
    (allocate keys hs).1.count h + (allocate keys hs).2.count h = hs.count h := by
  simpa only [List.count_append] using (allocation_multiset keys hs).count_eq h

theorem compact_text (t : Text) : textString (compact t) = textString t := by
  induction t with
  | nil => rfl
  | cons a rest ih =>
    cases a with
    | title v ps => simpa [compact,textString,String.join] using congrArg (v.native ++ ·) ih
    | site v => simpa [compact,textString,String.join] using congrArg (v.native ++ ·) ih
    | text v =>
      simp only [compact]
      cases he : compact rest with
      | nil => simp [he,textString,atomText] at ih ⊢; split <;> simp_all [textString,atomText]
      | cons x xs =>
        cases x with
        | text u => simp [he,textString,atomText] at ih ⊢; simp [← ih,String.append_assoc]
        | title x ps => simp [he,textString,atomText] at ih ⊢; split <;> simp_all [textString,atomText,String.append_assoc]
        | site x => simp [he,textString,atomText] at ih ⊢; split <;> simp_all [textString,atomText,String.append_assoc]

theorem fileless_cardinality (G : Graph) (p : Plan) (h : p.source = none) : retained G p = G := by
  simp only [retained,h]
  split
  · apply List.filter_eq_self.mpr; intro f _; simp
  · rfl

theorem headerOptions_save_independent (O : Observations) (save : File → File) (old : String) (p : Plan) (f : File) :
    headerFlexible p f = headerFlexible p f := rfl

theorem expected_save_independent (O : Observations) (save : File → File) (G : Graph) (old : String) (p : Plan) :
    expected P D {O with save := save} G old p = expected P D O G old p := rfl

end Cut
end CM
