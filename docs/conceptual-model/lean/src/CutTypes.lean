import ReadingEffects
namespace CM
namespace Cut

inductive SiteKind
  | link | tag | embed | propertyKey | propertyValue | task | priority
  | query | macroArg | orgFile | title | aliasName
  deriving DecidableEq, Repr
inductive Context | ordinary | mdTag | orgHeadlineTag | splitLine | queryArg | macroArgument
  deriving DecidableEq, Repr
structure Site where
  start : Nat
  stop : Nat
  kind : SiteKind
  context : Context
  name : String
  native : String
  deriving DecidableEq, Repr
inductive Atom | text (value : String) | site (value : Site) | title (whole : Site) (parts : List (String ⊕ Site))
  deriving DecidableEq, Repr
abbrev Text := List Atom
inductive HeaderRole | title | property | aliases | other
  deriving DecidableEq, Repr
inductive NativeForm | markdown | directive | drawer | other
  deriving DecidableEq, Repr
structure Header where
  role : HeaderRole
  form : NativeForm
  text : Text
  /-- Parser-normalized property key; absent for non-property header text. -/
  key : Option String
  deriving DecidableEq, Repr
structure Block where
  depth : Nat
  body : Text
  underHeader : Bool := false
  deriving DecidableEq, Repr
structure Content where
  header : List Header
  blocks : List Block
  deriving DecidableEq, Repr

def atomText : Atom → String | .text s => s | .site s => s.native | .title s _ => s.native
def textString : Text → String
  | [] => "" | a :: rest => atomText a ++ textString rest
def atomSites : Atom → List Site
  | .text _ => []
  | .site s => [s]
  | .title s parts => s :: parts.filterMap (fun a => match a with | .inl _ => none | .inr t => some t)
def textSites (t : Text) : List Site := t.flatMap atomSites
def bodyText (t : Text) : Text := t
def headerKey (h : Header) : String :=
  h.key.getD ""
def aliases (c : Content) : List String := c.header.flatMap (fun h =>
  if h.role == .aliases then (textSites h.text).filterMap (fun s => if s.kind == .aliasName then some s.name else none) else [])
def title (c : Content) : Option String :=
  ((c.header.filter (fun h => h.role == .title)).flatMap (fun h => textSites h.text)).find?
    (fun s => s.kind == .title) |>.map Site.name
def contentSites (c : Content) : List Site :=
  c.header.flatMap (fun h => textSites h.text) ++ c.blocks.flatMap (fun b => textSites (bodyText b.body))

/-- One native parse supplies the names, content, depths and locations.
The reader law in GUARANTEES applies to this whole record, not just names. -/
structure Observations where
  read : Format → String → Content
  conflict : File → Bool
  spell : Format → Site → String → Option String
  keySpelling : String → Option String
  save : File → File
  supported : File → Bool
  stem : File → String
  parent : File → String
  extension : File → String
  encode : String → String
  foldedPath : String → String

def Observations.content (O : Observations) (f : File) := O.read f.format f.text

def occKind : SiteKind → Option OccKind
  | .link => some .link | .tag => some .tag | .embed => some .pageEmbed
  | .propertyKey => some .propertyKey | .propertyValue => some .linkableValue
  | .task => some .taskMarker | .priority => some .priority | _ => none

def contentRows (c : Content) : List Text :=
  c.header.map Header.text ++ c.blocks.map (fun b => bodyText b.body)

abbrev Bytes := List UInt8
def bytes (s : String) : Bytes := s.toUTF8.data.toList
structure Edit where
  start : Nat
  stop : Nat
  replacement : String
  deriving DecidableEq, Repr
def insertEdit (e : Edit) : List Edit → List Edit
  | [] => [e]
  | h :: rest => if e.start ≤ h.start then e :: h :: rest else h :: insertEdit e rest
def splice (input : Bytes) : Nat → List Edit → Bytes
  | cursor, [] => input.drop cursor
  | cursor, e :: es => (input.drop cursor).take (e.start - cursor) ++ bytes e.replacement ++ splice input e.stop es
def byteString (s : Bytes) : String := String.fromUTF8! (ByteArray.mk s.toArray)
def partText : String ⊕ Site → String | .inl s => s | .inr s => s.native
def partAtom : String ⊕ Site → Atom | .inl s => .text s | .inr s => .site s
def partsLocated : Nat → List (String ⊕ Site) → Bool
  | _, [] => true
  | cursor, p :: ps =>
    (match p with
      | .inl _ => true
      | .inr s => decide (s.start = cursor ∧ s.stop = cursor + (bytes s.native).length)) &&
    partsLocated (cursor + (bytes (partText p)).length) ps

def titleCoherent : Atom → Bool
  | .title s parts => decide (s.kind = .title ∧ s.name = s.native ∧
      String.join (parts.map partText) = s.native) && partsLocated s.start parts
  | _ => true

/-- Fragment parsing is the same reader. A faithful adapter must satisfy the
fragment law: parsing each outline payload preserves its occurrence sequence. -/
def readingInputs {K : Type} (O : Observations) (key : String → K)
    (url : String → Bool) : Consolidated13.Inputs K := ⟨{
  key := key, stem := O.stem, title := fun fmt t => title (O.read fmt t),
  aliasDecl := fun fmt t => aliases (O.read fmt t),
  outline := fun fmt t => (contentRows (O.read fmt t)).map textString,
  occ := fun fmt t => (contentSites (O.read fmt t)).filterMap (fun s => (occKind s.kind).map (·,s.name)),
  fileLe := Orders.fileLe, pageLe := fun _ _ => true, refKind := fun _ => .page,
  kindPref := .page, aliasKinds := fun _ => true, urlHasOrigin := url }⟩

inductive Answer | confirm | cancel
  deriving DecidableEq, Repr
structure Request where
  source : String
  typed : String
  answer : Option Answer := none
  attended : Bool := true
  deriving DecidableEq, Repr
structure Plan where
  destination : String
  source : Option File
  survivor : Option File
  deriving DecidableEq, Repr
inductive Refusal
  | absentSource | daySource | contested | ownChild | capture
  | mixedFormats | unstableOrgInput | titleIdentity | pathTaken | conflictRewrite
  | unattended | unavailableAnswer | cancelled | aliasHeld (opensPath : String)
  | unknownSurvivorSpelling | unsupportedInput | writerFailed | textP1 | observationsP2 | saveP3
  | invalidSites | malformedGraph | headerChildren | conflictPair | leftoverReference
  deriving DecidableEq, Repr
inductive Question | confirmMerge
  deriving DecidableEq, Repr
inductive Result (α : Type) | refuse (why : Refusal) | ask (question : Question) | success (value : α)
  deriving DecidableEq, Repr
abbrev Selection := Result Plan
inductive Disposition | unchanged | substituted | fixed | skippedConflict | unspellable | normalizedKey
  deriving DecidableEq, Repr
structure Report where
  path : String
  start : Nat
  stop : Nat
  disposition : Disposition
  kind : SiteKind
  deriving DecidableEq, Repr

def nsMatch {K : Type} [DecidableEq K] (key : String → K) (url : String → Bool) (old s : String) : Option Nat :=
  let parts := (Params.splitSlash s.toList).map String.ofList
  if isNsName url s then
    (List.range parts.length).find? (fun i => let partName := "/".intercalate (parts.take (i + 1));
      !partName.isEmpty && decide (key partName = key old))
  else if key s = key old then some 0 else none

def nsSubst {K : Type} [DecidableEq K] (key : String → K) (url : String → Bool) (old new s : String) : String :=
  let parts := (Params.splitSlash s.toList).map String.ofList
  match nsMatch key url old s with
  | none => s
  | some i => if !isNsName url s || i + 1 = parts.length then new
      else new ++ "/" ++ "/".intercalate (parts.drop (i + 1))
end Cut
end CM
