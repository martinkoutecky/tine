/-
Tine conceptual model v0, definitions (SPEC-v0 §2–§4). Executable: every definition computes, so the
model can be evaluated on small graphs (`Demo.lean`) and, later, compared with Tine by a fixture bridge.

The model is parametric in a record `Params K` of given functions (the key function, config, and the
parser-facing extractors) and two orders. Nothing about them is assumed except where a theorem says so;
the only laws any round-1 theorem uses are that the two orders are linear (`Params.Lawful`).
-/
import MinBy
import Reach

namespace CM

/-! ## Files and graphs (SPEC §2.2) -/

inductive Dir | journals | pages
  deriving DecidableEq, Repr

inductive Format | md | org
  deriving DecidableEq, Repr

/-- A file of the graph. -/
structure File where
  path : String
  dir : Dir
  format : Format
  text : String
  deriving DecidableEq, Repr

/-- A graph is a finite set of files, represented by a list. The theorems show that nothing the model
computes depends on the list beyond its members (T1). -/
abbrev Graph := List File

/-- SPEC §2.2: paths are unique within a graph. Only the block-lookup results (`occAt`) need it. -/
def WF (G : Graph) : Prop := (G.map File.path).Nodup

inductive Kind | journal | page
  deriving DecidableEq, Repr

/-- SPEC §2.3: a page is a pair (kind, key). -/
abbrev Page (K : Type) := Kind × K

inductive OccKind | link | tag | pageEmbed | propertyKey | linkableValue | taskMarker | priority
  deriving DecidableEq, Repr

/-- SPEC §2.3: a block is identified by (file path, position in that file's outline). -/
abbrev BlockId := String × Nat

/-- The given functions of the model.

* `key` — SPEC §2.1. Laws K1/K2 are in `KeyLaws`; no round-1 theorem needs them.
* `stem`, `refKind` — config (SPEC §2.2); `refKind` replaces `journalDay` and is the model's rule for
  open point O3: the kind of a reference-only page is a function of its *key* (forced by T1, which says
  resolution depends on a spelling only through `key`).
* `title`, `aliasDecl`, `outline`, `occ` — the parser-facing extractors, each a function of a file's
  format and text (or a block's text) only. This typing *is* law O1 (locality of `occ`); O2
  (finiteness) holds because they return lists.
  `title fmt text` = the page title property (Md `title::` in the preamble, Org `#+title:`) if present;
  `aliasDecl fmt text` = the spellings of the `alias::`/`aliases::` property of the page header (SPEC §4.1);
  `outline fmt text` = the texts of the file's blocks in outline order.
* `fileLe` — the owner order on files (SPEC §2.3, O1); `pageLe` — the order among direct declaring
  pages that navigation picks from (SPEC §4.3, O2). Both are parameters; theorems assume them linear.
* `kindPref` — which kind wins when a key is claimed both by a journal and by a page (round 1's O7;
  decided since as a conflict, D-20261019-ZY2: `kindPref` is only the internal totalisation of that invalid state).
-/
structure Params (K : Type) where
  key : String → K
  stem : File → String
  title : Format → String → Option String
  aliasDecl : Format → String → List String
  outline : Format → String → List String
  occ : Format → String → List (OccKind × String)
  fileLe : File → File → Bool
  pageLe : Page K → Page K → Bool
  refKind : K → Kind
  kindPref : Kind
  /-- Open O4: which source kinds may declare aliases. The SPEC's current
  page-only reading is the default, not a silently fixed product decision. -/
  aliasKinds : Kind → Bool := fun k => k == .page
  /-- Parser boundary for OG gp-util/url?: true exactly when WHATWG URL parsing
  succeeds with an origin other than null. The default is the URL-free fixture
  instance, not a production URL parser. -/
  urlHasOrigin : String → Bool := fun _ => false
  /-- Parser boundary: OG string/blank? negated. The default is the
  nonempty-token fixture instance, not a Unicode whitespace implementation. -/
  inputNonblank : String → Bool := fun s => !s.isEmpty

/-- OG `text/namespace-page?` (`deps/graph-parser/src/logseq/graph_parser/text.cljs:79`, 0.10.9
`c67b8b5fa`): the name contains `/`, does not start with `../` or `./`, and is not an
origin-bearing URL (`url`, the supplied `gp-util/url?` boundary, `util.cljs:82`). Round 10:
the ONE namespace test, asked by file-name parents, occurrence closure and the rename
subtree map alike. -/
def isNsName (url : String → Bool) (s : String) : Bool :=
  s.toList.contains '/' && !s.startsWith "../" && !s.startsWith "./" && !url s

/-- SPEC §2.1 laws on `key` (K2 is the model's *definition* of sameness of names: two spellings name
the same page iff their keys are equal; the model never compares spellings otherwise). -/
structure KeyLaws {K : Type} (key : String → K) (spell : K → String) : Prop where
  k1 : ∀ k, key (spell k) = k

namespace Params

variable {K : Type} [DecidableEq K] (P : Params K)

/-- The laws the round-1 theorems use: both orders are linear. -/
structure Lawful : Prop where
  file : Linear P.fileLe
  page : Linear P.pageLe

/-! ## Pages (SPEC §2.3) -/

/-- `kind f = journal` iff `f` is in `journals/` (DESIGN Q4). -/
def kindOf (f : File) : Kind :=
  match f.dir with
  | .journals => .journal
  | .pages => .page

/-- The file's effective name. A page file: its title property, else its stem. A journal file: its
stem (the date), even when it has a title property, which is then a property only (round 9,
D-20261103-CMA CM-NAMES (2); before round 9 the title won for journals too). -/
def name (f : File) : String :=
  match f.dir with
  | .journals => P.stem f
  | .pages => (P.title f.format f.text).getD (P.stem f)

/-- The page a file claims. -/
def claim (f : File) : Page K := (kindOf f, P.key (P.name f))

def claimants (G : Graph) (p : Page K) : List File := G.filter (fun f => decide (P.claim f = p))

/-- The owner: the least claimant under `fileLe`. -/
def owner (G : Graph) (p : Page K) : Option File := minBy P.fileLe (P.claimants G p)

def isOwner (G : Graph) (f : File) : Bool := decide (P.owner G (P.claim f) = some f)

/-- A page is real iff some file claims it. -/
def real (G : Graph) (p : Page K) : Bool := G.any (fun f => decide (P.claim f = p))

/-- The real pages (one entry per claiming file; duplicates are harmless). -/
def realPages (G : Graph) : List (Page K) := G.map P.claim

/-- Blocks of the graph: the blocks of owner files (blocks of non-owner claimants belong to no page). -/
def blocks (G : Graph) : List BlockId :=
  G.flatMap (fun f => if P.isOwner G f then (List.range (P.outline f.format f.text).length).map (f.path, ·)
                     else [])

/-- The file at a path (the first one; unique under `WF`). -/
def fileAt (G : Graph) (p : String) : Option File := G.find? (fun f => decide (f.path = p))

/-! ## References (SPEC §3) -/

/-- The occurrences of a block (empty for an id that is not a block of the graph's files). -/
def occAt (G : Graph) (b : BlockId) : List (OccKind × String) :=
  match fileAt G b.1 with
  | none => []
  | some f =>
    match (P.outline f.format f.text)[b.2]? with
    | none => []
    | some t => P.occ f.format t

/-- Split a character list at every `/` (structural, so the kernel can evaluate it). -/
def splitSlash : List Char → List (List Char)
  | [] => [[]]
  | c :: cs =>
    let r := splitSlash cs
    if c = '/' then [] :: r
    else match r with
      | h :: t => (c :: h) :: t
      | [] => [[c]]

/-- SPEC §3.2: `a/b/c` also references `a/b` and `a`. The spelling itself, then its nonempty proper
namespace prefixes (split at `/`, OG `split-namespace-pages`). -/
def nsClosure (s : String) : List String :=
  let parts := (splitSlash s.toList).map String.ofList
  s :: ((List.range (parts.length - 1)).map (fun i => "/".intercalate (parts.take (i + 1)))).filter
    (fun t => !t.isEmpty)

/-- OG text/namespace-page?, on the effective name after the given name
extractors. Relative paths and origin-bearing URLs are not namespaces. -/
def nameIsNamespace (s : String) : Bool := isNsName P.urlHasOrigin s

/-- Round 10 (OG parity, divergence S15): the namespace closure of an occurrence
spelling. OG `with-page-refs` (`block.cljs:365`) takes namespace prefixes of a
referenced name only when `namespace-page?` holds, the same test file names
use (`extract.cljc:195`). So `[[./x]]`, `[[../x/y]]` and `[[https://a.b/c]]`
reference only themselves; `[[a/b]]` still references `a`. -/
def occClosure (s : String) : List String := if P.nameIsNamespace s then nsClosure s else [s]

/-- Nonempty proper namespace prefixes. Round 11, D-20261105-CM11 R10-Q1:
shared with occurrence closure; no empty page from a leading slash. This deliberately
replaces OG extract.cljc:194–199, which keeps the empty prefix. -/
def namePrefixes (s : String) : List String := (nsClosure s).tail

/-- Parents of every nonblank file (journals and non-owner claimants included).
The same nonempty prefixes as references, with the existing extraction boundary. -/
def fileNsNames (f : File) : List String :=
  if P.inputNonblank f.text && P.nameIsNamespace (P.name f)
  then namePrefixes (P.name f) else []

def refNames (G : Graph) (b : BlockId) : List String := (P.occAt G b).flatMap (fun o => P.occClosure o.2)

/-! ## Aliases (SPEC §4.1, §4.2) -/

/-- Owners whose kind is enabled by the O4 policy parameter. -/
def declaringOwners (G : Graph) : List File :=
  G.filter (fun f => P.isOwner G f && P.aliasKinds (kindOf f))

/-- The alias edges, oriented (declaring page's key, declared key); the graph is undirected (`Reach`). -/
def aliasEdges (G : Graph) : List (K × K) :=
  (P.declaringOwners G).flatMap
    (fun f => (P.aliasDecl f.format f.text).map (fun s => ((P.claim f).2, P.key s)))

/-- Does `f` declare a spelling of key `k`? -/
def declaresKey (f : File) (k : K) : Bool := (P.aliasDecl f.format f.text).any (fun s => decide (P.key s = k))

/-- The real pages declaring key `k` directly. -/
def declarers (G : Graph) (k : K) : List (Page K) :=
  ((P.declaringOwners G).filter (fun f => P.declaresKey f k)).map P.claim

/-- `class k` (SPEC §4.2): the connected component of `k` in the alias graph. -/
def aliasClass (G : Graph) (k : K) : K → Prop := cls (P.aliasEdges G) k

/-! ## Resolution and concepts (SPEC §4.3) -/

def otherKind : Kind → Kind
  | .journal => .page
  | .page => .journal

/-- Round 9 (EMPTY-ALIAS, D-20261103-CMA): a file with no real content, OG's
`page-empty-or-dummy?` (`frontend/db/model.cljs`): no block, or exactly one block whose text is
empty. In the intended extractor a page header (properties-only first block) is a block, so a file
with only `alias::`/`title::`/`icon::` lines has content (OG: its pre-block's content is the
property text); the title and alias conjuncts say the same for extractors that keep the header out
of `outline` (the toy parsers). -/
def contentFree (f : File) : Bool :=
  (P.title f.format f.text).isNone && (P.aliasDecl f.format f.text).isEmpty &&
    match P.outline f.format f.text with
    | [] => true
    | [t] => t.isEmpty
    | _ => false

/-- Round 9: a page is solid when some file claiming it has real content. For a key with one
claimant (every valid state) this is "the owner has content"; on a conflicted key (invalid, O1) it
totalises like `real`. -/
def solid (G : Graph) (p : Page K) : Bool := G.any (fun f => decide (P.claim f = p) && !P.contentFree f)

/-- Navigation from a key: a real page of that key (kind `kindPref` first) that is solid, or
real while the key has no direct declarer; else the least direct declarer under `pageLe`; else the
reference-only page of kind `refKind k`. Round 9 (EMPTY-ALIAS, D-20261103-CMA): a content-free file
loses to a direct declarer of its key, as in OG (`get-redirect-page-name`); a file with content
still wins (AL1), and with no declarer every real file wins exactly as before round 9
(`Rulings9.resolve_eq_r8_undeclared`). -/
def resolveKey (G : Graph) (k : K) : Page K :=
  if P.solid G (P.kindPref, k) ||
      (P.real G (P.kindPref, k) && (minBy P.pageLe (P.declarers G k)).isNone) then (P.kindPref, k)
  else if P.solid G (otherKind P.kindPref, k) ||
      (P.real G (otherKind P.kindPref, k) && (minBy P.pageLe (P.declarers G k)).isNone) then
    (otherKind P.kindPref, k)
  else
    match minBy P.pageLe (P.declarers G k) with
    | some q => q
    | none => (P.refKind k, k)

/-- `resolve s` (SPEC §4.3). "Declares `s` directly" is read as "declares a spelling whose key is
`key s`" (K2: the model compares names only by key). -/
def resolve (G : Graph) (s : String) : Page K := P.resolveKey G (P.key s)

/-- `concept s = class (key (resolve s))`, as the set of its keys. -/
def concept (G : Graph) (s : String) : K → Prop := P.aliasClass G (P.resolve G s).2

/-- Executable membership in `concept`. -/
def conceptB (G : Graph) (s : String) (k : K) : Bool := sameCls (P.aliasEdges G) (P.resolve G s).2 k

/-- The keys of `concept s`, listed (for display): candidates are the key itself and every edge end. -/
def conceptKeys (G : Graph) (s : String) : List K :=
  let r := (P.resolve G s).2
  let E := P.aliasEdges G
  (r :: E.flatMap (fun e => [e.1, e.2])).eraseDups.filter (fun k => sameCls E r k)

/-! ## References between nodes (SPEC §4.4) -/

/-- `refs b`: the set of concepts of `b`'s occurrence spellings, closed under namespaces. -/
def refs (G : Graph) (b : BlockId) : (K → Prop) → Prop := fun C => ∃ s ∈ P.refNames G b, C = P.concept G s

/-- Block `b` references page `t` iff `class t ∈ refs b`. -/
def References (G : Graph) (b : BlockId) (t : Page K) : Prop := P.refs G b (P.aliasClass G t.2)

/-- Executable `References`. -/
def referencesB (G : Graph) (b : BlockId) (t : Page K) : Bool :=
  (P.refNames G b).any (fun s => sameCls (P.aliasEdges G) (P.resolve G s).2 t.2)

end Params
end CM
