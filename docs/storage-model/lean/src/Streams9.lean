import Conservativity9
namespace Storage34
open Storage21
set_option maxHeartbeats 4000000

theorem noClaim_stream_projects33_9 (c : Config P) (disk : Fin P → Int) (xs : Nat → State P)
    (as : Nat → Storage33.Action P) (initial : xs 0 = initWith c disk)
    (hr : ∀ n,step c (xs n) (of33 (as n)) = some (xs (n+1))) :
    (∀ n,xs n = embed (project (xs n))) ∧
      (∀ n,Storage33.step c (project (xs n)) (as n) = some (project (xs (n+1)))) := by
  have shape : ∀ n,xs n = embed (project (xs n)) := by
    intro n
    induction n with
    | zero => rw [initial]; rfl
    | succ n ih =>
      have hn := hr n
      rw [ih,prior33_step_exact9] at hn
      cases hs : Storage33.step c (project (xs n)) (as n) with
      | none => simp [hs] at hn
      | some y =>
        simp only [hs,Option.map_some,Option.some.injEq] at hn
        rw [← hn]
        rfl
  refine ⟨shape,?_⟩
  intro n
  have hn := hr n
  rw [shape n,prior33_step_exact9] at hn
  cases hs : Storage33.step c (project (xs n)) (as n) with
  | none => simp [hs] at hn
  | some y =>
    simp only [hs,Option.map_some,Option.some.injEq] at hn
    have eq := congrArg project hn
    simpa using eq

theorem oldOnly_infinite_projects21_9 (c : Config P) (disk : Fin P → Int) (xs : Nat → State P)
    (as : Nat → Storage21.Action P) (initial : xs 0 = initWith c disk) (mutant : c.mutant = "none")
    (hr : ∀ n,step c (xs n) (.prior (.old (as n))) = some (xs (n+1))) :
    Storage3.Stream21 c (fun n => Storage3.project (Storage32.project (project (xs n)))) as ∧
      ∀ n,xs n = embed (Storage32.embed (Storage3.embed (Storage3.project (Storage32.project (project (xs n)))))) ∧
        guarantee (xs n) = Storage21.guarantee (Storage3.project (Storage32.project (project (xs n)))) := by
  have hs := noClaim_stream_projects33_9 c disk xs (fun n => .prior (.old (as n))) initial hr
  have old := Storage33.infinite_old_runs_exact8 c disk (fun n => project (xs n)) as (by rw [initial]; rfl) mutant hs.2
  refine ⟨old.1,?_⟩
  intro n
  refine ⟨?_,(old.2 n).2⟩
  rw [hs.1 n]
  exact congrArg embed (old.2 n).1
#print axioms noClaim_stream_projects33_9
#print axioms oldOnly_infinite_projects21_9
end Storage34
