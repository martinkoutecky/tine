import FinalA
namespace Storage
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

@[simp] theorem unheld_commit (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String) :
    "request-to-unheld" ∉ (commit c x s g n).g.bad ↔ "request-to-unheld" ∉ g.bad := by
  simp [commit]

theorem unheld_execute (c : Config P) (x : State P) (a : Action P)
    (h : "request-to-unheld" ∉ x.g.bad) (life : Lifetime x.s) (bytes : ByteDomain c x.s) :
    "request-to-unheld" ∉ (execute c x a).g.bad := by
  cases a <;> simp only [execute]
  case deliverUp ok =>
    cases hq : x.s.up with
    | nil => exact h
    | cons m tail =>
      have mem : m ∈ x.s.up := by simp [hq]
      have kinds : m.kind = "open" ∨ m.kind = "close" ∨ m.kind = "discard" ∨ m.kind = "submit" ∨ m.kind = "op" := by
        simpa using (bytes.up m mem).1
      rcases kinds with kind | kind | kind | kind | kind
      · simp only [deliverUp,kind]; simp
        simp only [upOpen,unheld_commit]; split <;> (try split) <;> exact h
      · simp only [deliverUp,kind]; simp
        simp only [upClose,unheld_commit]; exact h
      · have held := life.held m mem m.p (by simp [Needs,kind])
        simp [deliverUp,kind,upDiscard,held]
        split <;> simpa using h
      · have held := life.held m mem m.p (by simp [Needs,kind])
        simpa [deliverUp,kind,upSubmit,held] using h
      · have src := life.held m mem m.p (by simp [Needs,kind])
        have dst := life.held m mem m.q (by simp [Needs,kind])
        simp [deliverUp,kind,upOp,src,dst]
        split <;> (try split) <;> simpa using h
  case check => simp only [check]; split <;> simpa using h
  case rename => simpa [rename,commit] using h
  case draftSync p => simp only [draftSync,unheld_commit]; split <;> (try split) <;> exact h
  all_goals simpa [wOpen,wEdit,wSend,wResolve,wDiscard,wOp,wClose,wRecv,observe,flush,dirSync,
    saveFail,draftSync,switchReq,switchFin,extWriteD,crash,windowCrash,power,launch,commit,gDown] using h

theorem unheld_reachable (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (hx : Reachable c x) : "request-to-unheld" ∉ x.g.bad := by
  induction hx with
  | initial => simp [init]
  | @advance x y hr a hs ih =>
    have life := lifetime_reachable c mutant x hr
    have bytes := bytes_reachable c x hr
    unfold step at hs; split at hs
    · cases hs; exact unheld_execute c x a ih life bytes
    · contradiction

def BadKinds : List String := ["B-unsaved-replaced","B'-window-replaced","B'-submit-not-taken",
  "C-overwrote-external","G-overwrote-unseen","request-to-unheld"]
def BadVocabulary (g : Ghost P) : Prop := ∀ t ∈ g.bad, t ∈ BadKinds

theorem vocabulary_add (g : Ghost P) (xs : List String) (h : BadVocabulary g)
    (new : ∀ t ∈ xs, t ∈ BadKinds) : BadVocabulary {g with bad := union g.bad xs} := by
  intro t ht
  rcases (mem_union _ _ _).mp ht with old | fresh
  · exact h t old
  · exact new t fresh

theorem vocabulary_tags (condition : Bool) (t : String) (h : t ∈ BadKinds) :
    ∀ a ∈ tags condition t, a ∈ BadKinds := by
  cases condition <;> simp [tags]; exact h

theorem vocabulary_commit (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String)
    (h : BadVocabulary g) : BadVocabulary (commit c x s g n).g := by
  intro t ht
  change t ∈ union (union g.bad _) _ at ht
  rcases (mem_union _ _ _).mp ht with first | last
  · rcases (mem_union _ _ _).mp first with old | fresh
    · exact h t old
    · exact vocabulary_tags _ _ (by simp [BadKinds]) t fresh
  · exact vocabulary_tags _ _ (by simp [BadKinds]) t last

theorem vocabulary_execute (c : Config P) (x : State P) (a : Action P) (h : BadVocabulary x.g) :
    BadVocabulary (execute c x a).g := by
  cases a <;> simp only [execute]
  case deliverUp ok =>
    cases hq : x.s.up with
    | nil => exact h
    | cons m tail =>
      simp only [deliverUp]; split <;> (try split) <;> (try split) <;> (try split)
      all_goals simp only [upOpen,upClose,upDiscard,upSubmit,upOp]
      all_goals (try split) <;> (try split)
      all_goals apply vocabulary_commit
      all_goals first | exact h | skip
      all_goals (try split) <;> (try split)
      all_goals first | exact h | skip
      all_goals apply vocabulary_add _ _ h
      all_goals first
        | (intro t ht; simp only [List.mem_singleton] at ht; subst t; simp [BadKinds])
        | exact vocabulary_tags _ _ (by simp [BadKinds])
  case rename =>
    apply vocabulary_commit
    intro t ht
    rcases (mem_union _ _ _).mp ht with first | last
    · rcases (mem_union _ _ _).mp first with old | fresh
      · exact h t old
      · exact vocabulary_tags _ _ (by simp [BadKinds]) t fresh
    · exact vocabulary_tags _ _ (by simp [BadKinds]) t last
  case check => simp only [check]; split <;> apply vocabulary_commit <;> exact h
  case draftSync p => apply vocabulary_commit; split <;> (try split) <;> exact h
  all_goals apply vocabulary_commit
  all_goals exact h

theorem vocabulary_reachable (c : Config P) (x : State P) (hx : Reachable c x) : BadVocabulary x.g := by
  induction hx with
  | initial => simp [BadVocabulary,init]
  | advance hr a hs ih =>
    unfold step at hs; split at hs
    · cases hs; exact vocabulary_execute c _ a ih
    · contradiction

theorem transitions_positiveTexts (c : Config P) (T : Nat) (texts : c.texts = positiveTexts T)
    (mutant : c.mutant = "none") (x : State P) (hx : Reachable c x) : transitions x = true := by
  have voc := vocabulary_reachable c x hx
  have b := B_reachable c T texts mutant x hx
  have w := Bprime1_reachable c mutant x hx
  have taken := Bprime2_reachable c mutant x hx
  have guarded := C_reachable c mutant x hx
  have seen := G_positiveTexts c T texts mutant x hx
  have held := unheld_reachable c mutant x hx
  have empty : x.g.bad = [] := by
    apply List.eq_nil_iff_forall_not_mem.mpr
    intro t mem
    have kind := voc t mem
    simp [BadKinds] at kind
    rcases kind with rfl | rfl | rfl | rfl | rfl | rfl <;> contradiction
  simp [transitions,empty]

theorem guarantee_positiveTexts (c : Config P) (T : Nat) (texts : c.texts = positiveTexts T)
    (mutant : c.mutant = "none") (x : State P) (hx : Reachable c x) : guarantee x = true := by
  simp only [guarantee,noLoss_positiveTexts c T texts mutant x hx,
    transitions_positiveTexts c T texts mutant x hx,accepted_reachable c mutant x hx]
  rfl

#print axioms unheld_reachable
#print axioms vocabulary_reachable
#print axioms transitions_positiveTexts
#print axioms guarantee_positiveTexts
end Storage
