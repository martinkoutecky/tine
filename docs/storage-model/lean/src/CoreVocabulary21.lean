import CoreAccepted21
namespace Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
def BadKinds : List String := ["B-unsaved-replaced","B'-window-replaced","B'-submit-not-taken",
  "C-overwrote-external","G-overwrote-unseen","request-to-unheld","D4-unclean-page-changed","rename-clobbered-target"]
def BadVocabulary (g : Ghost P) : Prop := ∀ t ∈ g.bad, t ∈ BadKinds

theorem vocabulary_add (g : Ghost P) (xs : List String) (h : BadVocabulary g)
    (new : ∀ t ∈ xs, t ∈ BadKinds) : BadVocabulary {g with bad := union g.bad xs} := by
  intro t ht
  rcases (mem_union_general _ _ _).mp ht with old | fresh
  · exact h t old
  · exact new t fresh

theorem vocabulary_tags (condition : Bool) (t : String) (h : t ∈ BadKinds) :
    ∀ a ∈ tags condition t, a ∈ BadKinds := by
  cases condition <;> simp [tags]; exact h

theorem vocabulary_commit (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String)
    (h : BadVocabulary g) : BadVocabulary (commit c x s g n).g := by
  intro t ht
  change t ∈ union (union g.bad _) _ at ht
  rcases (mem_union_general _ _ _).mp ht with first | last
  · rcases (mem_union_general _ _ _).mp first with old | fresh
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
    rcases (mem_union_general _ _ _).mp ht with first | last
    · rcases (mem_union_general _ _ _).mp first with old | fresh
      · exact h t old
      · exact vocabulary_tags _ _ (by simp [BadKinds]) t fresh
    · exact vocabulary_tags _ _ (by simp [BadKinds]) t last
  case check => simp only [check]; split <;> apply vocabulary_commit <;> exact h
  case draftSync p => apply vocabulary_commit; split <;> (try split) <;> exact h
  all_goals apply vocabulary_commit
  all_goals exact h


end Storage21
