import TypedInvisible5
namespace Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false
theorem protected_wOpen5 (c : Config P) (x : State P) (p : Fin P)
    (h : TypedProtected5 x) (hen : enabled c x (.wOpen p) = true) :
    TypedProtected5 (wOpen c x p) := by
  apply protected_commit5
  apply protected_pre_frame5 x _ _ h
  · rfl
  · rfl
  · exact fun _ _ h => h
  · intro z hb
    simp only [Blind5] at hb ⊢
    simp only [enabled,Bool.and_eq_true] at hen
    simp only [put,up1,List.mem_append,List.mem_singleton]
    constructor
    · by_cases eq : z = p
      · subst z; simp_all <;> grind
      · simp_all <;> grind
    · constructor
      · intro m hm hk
        rcases hm with old | fresh
        · exact hb.2.1 m old hk
        · subst m; simp_all <;> grind
      · exact hb.2.2
theorem protected_wEdit5 (c : Config P) (x : State P) (p : Fin P) (v : Int)
    (h : TypedProtected5 x) (hen : enabled c x (.wEdit p v) = true) :
    TypedProtected5 (wEdit c x p v) := by
  apply protected_commit5
  apply protected_pre_frame5 x _ _ h
  · rfl
  · rfl
  · exact fun _ _ h => h
  · intro z hb
    simp only [Blind5] at hb ⊢
    simp only [enabled,Bool.and_eq_true] at hen
    simp only [put,up1,List.mem_append,List.mem_singleton]
    constructor
    · by_cases eq : z = p
      · subst z; simp_all <;> grind
      · simp_all <;> grind
    · constructor
      · intro m hm hk
        exact hb.2.1 m hm hk
      · exact hb.2.2
theorem protected_wSend5 (c : Config P) (x : State P) (p : Fin P)
    (h : TypedProtected5 x) (hen : enabled c x (.wSend p) = true) :
    TypedProtected5 (wSend c x p) := by
  apply protected_commit5
  apply protected_pre_frame5 x _ _ h
  · rfl
  · rfl
  · exact fun _ _ h => h
  · intro z hb
    simp only [Blind5] at hb ⊢
    simp only [enabled,Bool.and_eq_true] at hen
    simp only [put,up1,List.mem_append,List.mem_singleton]
    constructor
    · by_cases eq : z = p
      · subst z; simp_all <;> grind
      · simp_all <;> grind
    · constructor
      · intro m hm hk
        rcases hm with old | fresh
        · exact hb.2.1 m old hk
        · subst m; simp_all <;> grind
      · exact hb.2.2
theorem protected_wResolve5 (c : Config P) (x : State P) (p : Fin P) (v : Int)
    (h : TypedProtected5 x) (hen : enabled c x (.wResolve p v) = true) :
    TypedProtected5 (wResolve c x p v) := by
  apply protected_commit5
  apply protected_pre_frame5 x _ _ h
  · rfl
  · rfl
  · intro q b hb
    by_cases eq : q = p
    · subst q; simpa [Seen,put,mem_union_general] using Or.inl hb
    · simpa [Seen,put,eq] using hb
  · intro z hb
    simp only [Blind5] at hb ⊢
    simp only [enabled,Bool.and_eq_true] at hen
    simp only [put,up1,List.mem_append,List.mem_singleton]
    constructor
    · by_cases eq : z = p
      · subst z; simp_all <;> grind
      · simp_all <;> grind
    · constructor
      · intro m hm hk
        rcases hm with old | fresh
        · exact hb.2.1 m old hk
        · subst m; simp_all <;> grind
      · exact hb.2.2
theorem protected_wDiscard5 (c : Config P) (x : State P) (p : Fin P)
    (h : TypedProtected5 x) (hen : enabled c x (.wDiscard p) = true) :
    TypedProtected5 (wDiscard c x p) := by
  apply protected_commit5
  apply protected_pre_frame5 x _ _ h
  · rfl
  · rfl
  · exact fun _ _ h => h
  · intro z hb
    simp only [Blind5] at hb ⊢
    simp only [enabled,Bool.and_eq_true] at hen
    simp only [put,up1,List.mem_append,List.mem_singleton]
    constructor
    · by_cases eq : z = p
      · subst z; simp_all <;> grind
      · simp_all <;> grind
    · constructor
      · intro m hm hk
        rcases hm with old | fresh
        · exact hb.2.1 m old hk
        · subst m; simp_all <;> grind
      · exact hb.2.2
theorem protected_wOp5 (c : Config P) (x : State P) (p q : Fin P) (t u : Int)
    (h : TypedProtected5 x) (hen : enabled c x (.wOp p q t u) = true) :
    TypedProtected5 (wOp c x p q t u) := by
  apply protected_commit5
  apply protected_pre_frame5 x _ _ h
  · rfl
  · rfl
  · exact fun _ _ h => h
  · intro z hb
    simp only [Blind5] at hb ⊢
    simp only [enabled,Bool.and_eq_true] at hen
    simp only [put,up1,List.mem_append,List.mem_singleton]
    constructor
    · by_cases eq : z = p
      · subst z; simp_all <;> grind
      · simp_all <;> grind
    · constructor
      · intro m hm hk
        rcases hm with old | fresh
        · exact hb.2.1 m old hk
        · subst m; simp_all <;> grind
      · exact hb.2.2
theorem protected_wClose5 (c : Config P) (x : State P) (p : Fin P)
    (h : TypedProtected5 x) (hen : enabled c x (.wClose p) = true) :
    TypedProtected5 (wClose c x p) := by
  apply protected_commit5
  apply protected_pre_frame5 x _ _ h
  · rfl
  · rfl
  · exact fun _ _ h => h
  · intro z hb
    simp only [Blind5] at hb ⊢
    simp only [enabled,Bool.and_eq_true] at hen
    simp only [put,up1,List.mem_append,List.mem_singleton]
    constructor
    · by_cases eq : z = p
      · subst z; simp_all <;> grind
      · simp_all <;> grind
    · constructor
      · intro m hm hk
        rcases hm with old | fresh
        · exact hb.2.1 m old hk
        · subst m; simp_all <;> grind
      · exact hb.2.2
theorem protected_commit_visible5 (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String)
    (h : ∀ p, (s.pages p).held = true → (s.pages p).typed = true →
      (s.pages p).buf ∈ g.seen p ∨
      ((g.promise p).ver ≥ (s.pages p).ver ∧ (g.promise p).bytes = (s.pages p).buf) ∨
      Blind5 s p ∨ ((s.w p).on = true ∧ (s.w p).text = (s.pages p).buf))
    (bytes : ∀ p, RealByte c (s.pages p).buf) : TypedProtected5 (commit c x s g n) := by
  intro p hh ht
  change (s.pages p).held = true at hh
  change (s.pages p).typed = true at ht
  rcases h p hh ht with hs | hp | hb | hv
  · exact Or.inl (seen_commit c x s g n p _ hs)
  · right; left
    have np : PostPromises x s g p = g.promise p := by simp [PostPromises,clean,ht]
    change (PostPromises x s g p).ver ≥ (s.pages p).ver ∧ (PostPromises x s g p).bytes = (s.pages p).buf
    simpa [np] using hp
  · right; right
    refine ⟨hb.1,hb.2.1,?_⟩
    intro mail
    change (if _ then _ else s.mb p).text = (s.pages p).buf
    split
    · rfl
    · rename_i no
      exact hb.2.2 (by simpa only [commit,no,Bool.false_eq_true,ite_false] using mail)
  · left
    apply (mem_union_general _ _ _).mpr
    right
    simp only [List.mem_filter,List.mem_singleton]
    exact ⟨hv.2.symm,by simpa [hv.1,RealByte] using bytes p⟩

theorem protected_wRecv5 (c : Config P) (mutant : c.mutant = "none") (x : State P) (p : Fin P)
    (h : TypedProtected5 x) (bytes : ByteDomain c x.s)
    (hen : enabled c x (.wRecv p) = true) : TypedProtected5 (wRecv c x p) := by
  simp only [wRecv]
  apply protected_commit_visible5
  · intro q hh ht
    change (x.s.pages q).held = true at hh
    change (x.s.pages q).typed = true at ht
    rcases h q hh ht with hs | hp | hb
    · exact Or.inl hs
    · exact Or.inr (Or.inl hp)
    · right; right
      by_cases eq : q = p
      · subst q
        have mail : (x.s.mb p).on = true := by simp [enabled] at hen; exact hen.2
        have text := hb.2.2 mail
        simp only [Blind5,put,ite_true]
        have off := hb.1
        have queue := hb.2.1
        simp only [off,Bool.false_and,Bool.false_eq_true,ite_false]
        split <;> (try split) <;> (try split)
        all_goals simp_all [mutant]
      · left
        simpa [Blind5,put,eq] using hb
  · exact bytes.pages

end Storage21
