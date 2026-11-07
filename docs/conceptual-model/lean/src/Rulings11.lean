/- Round 11: D-20261105-CM11. Authoritative bridge entry points:
offers / answer (rename and merge, including the M-Q5 dialog), tree / children.
Round 12 extends this same API by D-20261105-CM12 (alias-only sources).
Round 9/10 operations remain low-level historical operations, not a dialog API.
No field is added to any existing structure. -/
import Witness10

set_option linter.unusedSectionVars false
namespace CM
namespace Rulings11
open Params Merge7 Rulings9 Ref10

variable {K : Type} [DecidableEq K] (P : Params K)

/-! ## Empty prefixes: the shared contribution -/

theorem namePrefixes_nonempty {s t : String} (h : t ∈ namePrefixes s) : t ≠ "" := by
  simp only [namePrefixes, nsClosure, List.tail_cons, List.mem_filter] at h
  simpa [String.isEmpty_iff] using h.2

theorem fileNsNames_nonempty {f : File} {t : String} (h : t ∈ P.fileNsNames f) : t ≠ "" := by
  unfold fileNsNames at h
  split at h
  · exact namePrefixes_nonempty h
  · cases h

/-- File parents and reference closure use exactly the same prefix list. -/
theorem fileNsNames_closure (f : File)
    (h : (P.inputNonblank f.text && P.nameIsNamespace (P.name f)) = true) :
    P.fileNsNames f = (P.occClosure (P.name f)).tail := by
  simp only [Bool.and_eq_true] at h
  simp [fileNsNames, h.1, h.2, occClosure, namePrefixes]

/-! ## Choices and the source kind -/

/-- No file of either kind claims the key; an enabled owner declares it.
This is a name operation: all declarers are rewritten, not one chosen owner. -/
def aliasSource (G : Graph) (old : String) : Bool :=
  !P.claimedKey G (P.key old) && P.declaredKey G (P.key old)

def noFileSource (G : Graph) (old : String) : Bool :=
  refSource P G old || aliasSource P G old

inductive SourceKind | realPage | emptyAliasFile | fileless | aliasOnly | unsupported
  deriving DecidableEq, Repr

def sourceKind (G : Graph) (old : String) : SourceKind :=
  match mergeSource P G old with
  | some S => if P.contentFree S && P.declaredKey G (P.key old)
      then .emptyAliasFile else .realPage
  | none => if refSource P G old then .fileless
      else if aliasSource P G old then .aliasOnly else .unsupported

theorem aliasSource_none_of_mergeSource {G : Graph} {old : String} {S : File}
    (h : mergeSource P G old = some S) : aliasSource P G old = false := by
  have hc : P.claimedKey G (P.key old) = true := by
    unfold mergeSource at h
    split at h
    · next f he =>
      have hf := (P.mem_claimants).mp (show f ∈ P.claimants G (.page, P.key old) by simp [he])
      exact P.claimedKey_iff.mpr ⟨f, hf.1, congrArg Prod.snd hf.2⟩
    · cases h
  simp [aliasSource, hc]

/-- A target governed by EMPTY-ALIAS rather than a solid file. -/
def redirectingAlias (G : Graph) (old new : String) : Bool :=
  decide (P.key new ≠ P.key old) && P.declaredKey G (P.key new) &&
    !P.solid G (.page, P.key new) && !P.solid G (.journal, P.key new)

/-- A redirecting alias name: no solid claimant; one direct declaring owner.
Conflicted destinations and same-key renames never enter the dialog.
EMPTY-ALIAS changes navigation, not file existence: do not erase its empty file. -/
def dialogOwner (G : Graph) (old new : String) : Option File :=
  if !redirectingAlias P G old new || P.conflictedB G (P.key new) then none
  else match (P.declaringOwners G).filter (fun f => P.declaresKey f (P.key new)) with
    | [D] => some D
    | _ => none

def target (G : Graph) (old new : String) : Option File :=
  match dialogOwner P G old new with
  | some D => some D
  | none => if redirectingAlias P G old new then none else mergeTarget P G old new

/-- The guard is shared verbatim with round 10; only target selection changes. -/
def mergeReady (G : Graph) (old new : String) : Bool :=
  match target P G old new with
  | none => false
  | some D => if noFileSource P G old then noFileMergeCore P G old new D
      else match mergeSource P G old with
        | some S => mergeAllowedTo P G old new S D
        | none => false

def mergeChosen (R : QRewriter) (J : (String → String) → Merger)
    (G : Graph) (old new : String) : Option Graph :=
  if mergeReady P G old new then
    match target P G old new with
    | none => none
    | some D =>
      let mq := queryMap P old new D
      let m := nsSubst P.key P.urlHasOrigin old new
      if noFileSource P G old then some (renameBy (R.at mq).base m G)
      else match mergeSource P G old with
        | some S => some (mergeBy (R.at mq).base (J mq) m S D G)
        | none => none
  else none

def renameReady (G : Graph) (old new : String) : Bool :=
  if noFileSource P G old then renameAllowedFree P G old new
  else Decided.renameAllowed P G old new

/-- Alias-only names use the same free guard and whole-graph rewrite as round 10.
No declaring page is deleted, appended to, or renamed merely for owning the alias. -/
def renameChosen (R : Decided.Rewriter) (G : Graph) (old new : String) : Option Graph :=
  if aliasSource P G old then
    if renameAllowedFree P G old new then some (P.rename R.base G old new) else none
  else rename10 P R G old new

/-- Answer two fills an existing empty file, or renames the source if no file claims
the alias name. A file-less source never offers it (R10-Q4). -/
def ownReady (G : Graph) (old new : String) : Bool :=
  (dialogOwner P G old new).isSome && !noFileSource P G old &&
    match mergeTarget P G old new with
    | some E => if (P.claim E).2 = P.key new then
        match mergeSource P G old with
        | some S => mergeAllowedTo P G old new S E
        | none => false
      else renameReady P G old new
    | none => false

def ownChosen (R : QRewriter) (J : (String → String) → Merger)
    (G : Graph) (old new : String) : Option Graph :=
  if ownReady P G old new then
    match mergeTarget P G old new with
    | some E => if (P.claim E).2 = P.key new then merge9 P R J G old new
      else rename10 P (R.at (nsSubst P.key P.urlHasOrigin old new)) G old new
    | none => none
  else none

inductive Choice | rename | mergeOwner | ownPage | cancel
  deriving DecidableEq, Repr

/-- R10-Q4 also prevents a plain-rewrite escape when several owners declare the
target and the existing unique-target merge guard refuses to choose one.
An ambiguous redirecting empty file is not an ordinary direct-merge fallback either. -/
def plainReady (G : Graph) (old new : String) : Bool :=
  renameReady P G old new && !((refSource P G old ||
      (aliasSource P G old && decide (P.key new ≠ P.key old))) && P.declaredKey G (P.key new)) &&
    !redirectingAlias P G old new

/-- Ordered choices for the bridge. Guarded choices are omitted if unsafe.
Cancellation is explicit and preserves the graph. -/
def offers (G : Graph) (old new : String) : List Choice :=
  if (dialogOwner P G old new).isSome then
    (if mergeReady P G old new then [.mergeOwner] else []) ++
    (if ownReady P G old new then [.ownPage] else []) ++ [.cancel]
  else
    (if (target P G old new).isSome then
      (if mergeReady P G old new then [.mergeOwner] else [])
    else if plainReady P G old new then [.rename] else []) ++ [.cancel]

/-- Execute only an offered answer. Legacy rename10's file-less plain rewrite onto
an alias can no longer be selected. The caller must use this API, not rename10. -/
def answer (R : QRewriter) (J : (String → String) → Merger)
    (G : Graph) (old new : String) (c : Choice) : Option Graph :=
  if (offers P G old new).contains c then
    match c with
    | .cancel => some G
    | .rename => renameChosen P (R.at (nsSubst P.key P.urlHasOrigin old new)) G old new
    | .mergeOwner => mergeChosen P R J G old new
    | .ownPage => ownChosen P R J G old new
  else none

theorem answer_unoffered (R : QRewriter) (J : (String → String) → Merger)
    {G : Graph} {old new : String} {c : Choice} (h : c ∉ offers P G old new) :
    answer P R J G old new c = none := by
  simp [answer, List.contains_iff_mem, h]

theorem cancel_offered (G : Graph) (old new : String) : Choice.cancel ∈ offers P G old new := by
  simp [offers]; split <;> simp

theorem answer_cancel (R : QRewriter) (J : (String → String) → Merger)
    (G : Graph) (old new : String) : answer P R J G old new .cancel = some G := by
  simp [answer, List.contains_iff_mem, cancel_offered P G old new]

/-- No answer-two choice for any file-less source, including an empty alias target. -/
theorem fileless_no_own {G : Graph} {old new : String} (h : refSource P G old = true) :
    Choice.ownPage ∉ offers P G old new := by
  simp [offers, ownReady, noFileSource, h]
  split <;> simp <;> split <;> simp <;> split <;> simp

theorem fileless_answer_two_refused (R : QRewriter) (J : (String → String) → Merger)
    {G : Graph} {old new : String} (h : refSource P G old = true) :
    answer P R J G old new .ownPage = none :=
  answer_unoffered P R J (fileless_no_own P h)

/-- Source kind is exported data, not a frontend inference. -/
theorem fileless_source_kind {G : Graph} {old : String} (h : refSource P G old = true) :
    sourceKind P G old = .fileless := by
  simp [sourceKind, Ref10.mergeSource_none P h, h]

/-- Round 12: an unsupported source is neither a source file, a reference-only
source nor an alias-only source. The added alias exclusion is essential (CM12). -/
theorem unsupported_no_mutation {G : Graph} {old new : String}
    (hs : mergeSource P G old = none) (hr : refSource P G old = false)
    (ha : Decided.renameAllowed P G old new = false) (hal : aliasSource P G old = false) :
    offers P G old new = [.cancel] := by
  simp [offers, mergeReady, ownReady, plainReady, renameReady, noFileSource, hs, hr, ha, hal]
  split <;> cases ht : target P G old new <;> simp_all
  all_goals cases mergeTarget P G old new <;> simp_all

/-- An accepted merge has exactly the old after-graph shape at the chosen survivor.
This exposes both the typed-name text map and the survivor-name query map. -/
theorem mergeChosen_structure (R : QRewriter) (J : (String → String) → Merger)
    {G G' : Graph} {old new : String} (h : mergeChosen P R J G old new = some G') :
    ∃ D, target P G old new = some D ∧ mergeReady P G old new = true ∧
      ((noFileSource P G old = true ∧
        G' = renameBy (R.at (queryMap P old new D)).base
          (nsSubst P.key P.urlHasOrigin old new) G) ∨
       ∃ S, mergeSource P G old = some S ∧
        G' = mergeBy (R.at (queryMap P old new D)).base (J (queryMap P old new D))
          (nsSubst P.key P.urlHasOrigin old new) S D G) := by
  unfold mergeChosen at h
  split at h
  · next ha =>
    cases ht : target P G old new with
    | none => rw [ht] at h; cases h
    | some D =>
      rw [ht] at h
      simp only at h
      split at h
      · next hr => exact ⟨D, rfl, ha, Or.inl ⟨hr, (Option.some.inj h).symm⟩⟩
      · cases hs : mergeSource P G old with
        | none => rw [hs] at h; cases h
        | some S => rw [hs] at h; exact ⟨D, rfl, ha, Or.inr ⟨S, rfl, (Option.some.inj h).symm⟩⟩
  · cases h

/-! ## Namespace tree: one universe, literal keys, no per-alias nodes -/

/-- Spellings are supplied by the bridge's canonical key renderer (K1). The model
does not invent another normalization algorithm or choose an alias spelling. -/
def treeNodes (G : Graph) : List (Page K) := (Decided.pages P G).eraseDups

def treeParent (spell : K → String) (G : Graph) (node : Page K) : Option (Page K) :=
  if (treeNodes P G).contains node then
    match nsParent P (spell node.2) with
    | none => none
    | some s => (treeNodes P G).find? (fun p => decide (p.2 = P.key s))
  else none

def children (spell : K → String) (G : Graph) (node : Page K) : List (Page K) :=
  (treeNodes P G).filter (fun p => decide (treeParent P spell G p = some node))

def tree (spell : K → String) (G : Graph) : List (Page K × List (Page K)) :=
  (treeNodes P G).map (fun p => (p, children P spell G p))

theorem treeNodes_exact (G : Graph) (p : Page K) :
    p ∈ treeNodes P G ↔ p ∈ Decided.pages P G := by simp [treeNodes]

theorem children_exact (spell : K → String) (G : Graph) (p c : Page K) :
    c ∈ children P spell G p ↔ c ∈ Decided.pages P G ∧ treeParent P spell G c = some p := by
  simp [children, treeNodes]

theorem treeParent_existing (spell : K → String) {G : Graph} {p c : Page K}
    (h : treeParent P spell G c = some p) : p ∈ Decided.pages P G := by
  unfold treeParent at h
  split at h
  · split at h
    · cases h
    · simpa [treeNodes] using List.mem_of_find?_eq_some h
  · cases h

theorem tree_exact (spell : K → String) (G : Graph) (p : Page K) (cs : List (Page K)) :
    (p, cs) ∈ tree P spell G ↔ p ∈ Decided.pages P G ∧ cs = children P spell G p := by
  simp [tree, treeNodes, List.mem_map, Prod.mk.injEq, and_left_comm, eq_comm]

/-- Q9 holds in the tree as in every other page view. This includes alias-only
namespace prefixes; if a file claims the alias spelling it is a real node. -/
theorem alias_only_no_node (spell : K → String) {G : Graph} {k : K}
    (hc : P.claimedKey G k = false) (hd : P.declaredKey G k = true) (κ : Kind) :
    (κ, k) ∉ treeNodes P G ∧
    ∀ p, (κ, k) ∉ children P spell G p := by
  have hn : (κ, k) ∉ Decided.pages P G := by
    simp only [Decided.pages, P.mem_pagesR, Prod.snd, Bool.true_and]
    have hr : P.real G (κ, k) = false := by
      have hh : P.real G (.journal, k) = false ∧ P.real G (.page, k) = false := by
        simpa [claimedKey] using hc
      cases κ
      · exact hh.1
      · exact hh.2
    simp [hr, hc, hd]
  exact ⟨by simpa [treeNodes] using hn, fun p h => hn ((children_exact P spell G p (κ, k)).mp h).1⟩


/-- The global Namespaces view omits isolated pages. The full forest above also
exports them, so page Hierarchy and namespace queries can ask the same children. -/
def namespaceNodes (spell : K → String) (G : Graph) : List (Page K) :=
  (treeNodes P G).filter (fun p =>
    (treeParent P spell G p).isSome || !(children P spell G p).isEmpty)

def namespaceTree (spell : K → String) (G : Graph) : List (Page K × List (Page K)) :=
  (namespaceNodes P spell G).map (fun p => (p, children P spell G p))

theorem namespaceNodes_exact (spell : K → String) (G : Graph) (p : Page K) :
    p ∈ namespaceNodes P spell G ↔ p ∈ Decided.pages P G ∧
      ((treeParent P spell G p).isSome = true ∨ (children P spell G p).isEmpty = false) := by
  simp [namespaceNodes, treeNodes]

theorem mergeChosen_real (R : QRewriter) (J : (String → String) → Merger)
    {G : Graph} {old new : String} {S D : File}
    (ht : target P G old new = some D) (hs : mergeSource P G old = some S)
    (hr : refSource P G old = false) (ha : mergeReady P G old new = true) :
    mergeChosen P R J G old new =
      some (mergeBy (R.at (queryMap P old new D)).base (J (queryMap P old new D))
        (nsSubst P.key P.urlHasOrigin old new) S D G) := by
  simp [mergeChosen, ht, hs, hr, ha, noFileSource, aliasSource_none_of_mergeSource P hs]

theorem mergeChosen_ref (R : QRewriter) (J : (String → String) → Merger)
    {G : Graph} {old new : String} {D : File}
    (ht : target P G old new = some D) (hr : refSource P G old = true)
    (ha : mergeReady P G old new = true) :
    mergeChosen P R J G old new =
      some (renameBy (R.at (queryMap P old new D)).base
        (nsSubst P.key P.urlHasOrigin old new) G) := by
  simp [mergeChosen, ht, hr, ha, noFileSource]

/-- Exact merge choices are meaningful, never an offer that cannot execute. -/
theorem mergeChosen_isSome (R : QRewriter) (J : (String → String) → Merger)
    (G : Graph) (old new : String) :
    (mergeChosen P R J G old new).isSome = mergeReady P G old new := by
  unfold mergeChosen
  cases ha : mergeReady P G old new
  · simp
  · simp only [Bool.true_eq, ite_true]
    cases ht : target P G old new with
    | none => simp [mergeReady, ht] at ha
    | some D =>
      simp only
      split
      · rfl
      · next hr =>
        cases hs : mergeSource P G old with
        | none => simp [mergeReady, ht, hs, hr] at ha
        | some S => rfl

/-- The exact list for a file-less source at an alias dialog, if the merge guard
passes: only answer one and cancellation, even if an empty target file exists. -/
theorem fileless_offer_set {G : Graph} {old new : String}
    (hd : (dialogOwner P G old new).isSome = true)
    (hr : refSource P G old = true) (ha : mergeReady P G old new = true) :
    offers P G old new = [.mergeOwner, .cancel] := by
  simp [offers, hd, ownReady, noFileSource, hr, ha]

theorem file_offer_set {G : Graph} {old new : String}
    (hd : (dialogOwner P G old new).isSome = true)
    (hm : mergeReady P G old new = true) (ho : ownReady P G old new = true) :
    offers P G old new = [.mergeOwner, .ownPage, .cancel] := by
  simp [offers, hd, hm, ho]

/-- A file-less source cannot bypass the merge by selecting plain rename onto
any alias target, including ambiguous multiple-declarer targets. -/
theorem fileless_alias_no_plain {G : Graph} {old new : String}
    (hr : refSource P G old = true) (hd : P.declaredKey G (P.key new) = true) :
    Choice.rename ∉ offers P G old new := by
  simp [offers, plainReady, noFileSource, hr, hd]
  split <;> simp <;> split <;> simp <;> split <;> simp

theorem fileless_alias_plain_refused (R : QRewriter) (J : (String → String) → Merger)
    {G : Graph} {old new : String}
    (hr : refSource P G old = true) (hd : P.declaredKey G (P.key new) = true) :
    answer P R J G old new .rename = none :=
  answer_unoffered P R J (fileless_alias_no_plain P hr hd)

end Rulings11
end CM
