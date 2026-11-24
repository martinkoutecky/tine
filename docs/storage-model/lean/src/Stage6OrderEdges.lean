import Stage6GateSafety

namespace Stage6
open Storage21

private theorem endpoint_cases {p : Slot P} {gt : Gate P} :
    p ∈ endpoints gt.moves ↔ Receiver gt p ∨ Source gt p := by
  simp only [endpoints, List.mem_flatMap, List.mem_cons, List.not_mem_nil, or_false, Receiver, Source]
  constructor
  · rintro ⟨m, hm, hp | hp⟩
    · exact Or.inl ⟨m, hm, hp⟩
    · exact Or.inr ⟨m, hm, hp⟩
  · rintro (⟨m, hm, eq⟩ | ⟨m, hm, eq⟩)
    · exact ⟨m, hm, Or.inl eq⟩
    · exact ⟨m, hm, Or.inr eq⟩

def EdgeRole (gt : Gate P) (o : Storage32.Obl P) : Prop :=
  (¬Receiver gt o.1 ∧ ¬Source gt o.1 ∧ Receiver gt o.2.1 ∧ o.2.1 = o.2.2.2) ∨
  ∃ m ∈ gt.moves, m.src = o.1 ∧ m.dst = o.2.2.2 ∧ ¬Source gt o.2.1

def Backs (gt : Gate P) (o : Storage32.Obl P) : Prop :=
  o.1 ∈ gt.pending ∧ o.2.1 ∈ gt.pending ∧ gt.vers o.2.1 = some o.2.2.1 ∧ EdgeRole gt o

def OrderEdges (gates : List (Gate P)) (obl : List (Storage32.Obl P)) : Prop :=
  (∀ o ∈ obl, ∃ gt ∈ gates, Backs gt o) ∧
  (∀ gt ∈ gates, ∀ m ∈ gt.moves, m.src ∈ gt.pending → m.dst ∈ gt.pending →
    ∀ v, gt.vers m.dst = some v → (m.src, m.dst, v, m.dst) ∈ obl)

theorem backed_gated {gt : Gate P} (shape : gt.Shape) {o : Storage32.Obl P} (h : Backs gt o) :
    gt.gated o.1 = true := by
  have recv (p : Slot P) : (gt.moves.any fun m => p == m.dst) = true ↔ Receiver gt p := by
    simp [Receiver, List.any_eq_true]
  have src (p : Slot P) : (gt.moves.any fun m => p == m.src) = true ↔ Source gt p := by
    simp [Source, List.any_eq_true]
  rcases h.2.2.2 with ⟨nr, _, rq, _⟩ | ⟨m, mm, eq, _, ns⟩
  · have disabled : (gt.moves.any fun m => o.1 == m.dst) = false :=
      Bool.eq_false_iff.mpr (fun hit => nr ((recv _).mp hit))
    have waits : (gt.pending.any fun p => gt.moves.any fun m => p == m.dst) = true :=
      List.any_eq_true.mpr ⟨o.2.1, h.2.1, (recv _).mpr rq⟩
    simp [Gate.gated, h.1, disabled, waits]
  · have sp : Source gt o.1 := ⟨m, mm, eq.symm⟩
    have disabled : (gt.moves.any fun m => o.1 == m.dst) = false :=
      Bool.eq_false_iff.mpr (fun hit => source_not_receiver shape sp ((recv _).mp hit))
    have waits : (gt.pending.any fun p => !(gt.moves.any fun m => p == m.src)) = true :=
      List.any_eq_true.mpr ⟨o.2.1, h.2.1, by
        have off : (gt.moves.any fun m => o.2.1 == m.src) = false :=
          Bool.eq_false_iff.mpr (fun hit => ns ((src _).mp hit))
        simp [off]⟩
    simp [Gate.gated, h.1, disabled, (src _).mpr sp, waits]

theorem obligations_backed (x : State P) (moves : List (Move P)) (keys : List (Slot P))
    (distinct : (endpoints moves).Nodup) (ends : ∀ p ∈ endpoints moves, p ∈ keys)
    (o : Storage32.Obl P) (mem : o ∈ obligations x moves keys) :
    Backs (newGate x moves keys) o := by
  let gt := newGate x moves keys
  simp only [obligations, mem_union_general, List.mem_flatMap, List.mem_map] at mem
  rcases mem with (⟨p, pm, m, mm, rfl⟩ | ⟨m, mm, p, pm, rfl⟩) | ⟨m, mm, n, nm, rfl⟩
  · obtain ⟨pm, np⟩ := mem_exclude.mp pm
    have d := ends m.dst (endpoint_mem mm).1
    refine ⟨pm, d, new_gate_version d, Or.inl ⟨?_, ?_, ⟨m, mm, rfl⟩, rfl⟩⟩
    · exact fun r => np (endpoint_cases.mpr (Or.inl r))
    · exact fun s => np (endpoint_cases.mpr (Or.inr s))
  · obtain ⟨pm, np⟩ := mem_exclude.mp pm
    exact ⟨ends m.src (endpoint_mem mm).2, pm, new_gate_version pm,
      Or.inr ⟨m, mm, rfl, rfl, fun s => np (endpoint_cases.mpr (Or.inr s))⟩⟩
  · have d := ends n.dst (endpoint_mem nm).1
    refine ⟨ends m.src (endpoint_mem mm).2, d, new_gate_version d,
      Or.inr ⟨m, mm, rfl, rfl, ?_⟩⟩
    rintro ⟨s, sm, eq⟩
    have same := endpoint_unique distinct nm sm (Or.inl rfl) (Or.inr eq)
    subst s
    exact endpoint_distinct distinct nm eq.symm

theorem obligations_complete (x : State P) (moves : List (Move P)) (keys : List (Slot P))
    {m : Move P} (mm : m ∈ moves) :
    (m.src, m.dst, version x m.dst, m.dst) ∈ obligations x moves keys := by
  unfold obligations
  rw [mem_union_general]
  exact Or.inr (List.mem_flatMap.mpr ⟨m, mm, List.mem_map.mpr ⟨m, mm, rfl⟩⟩)

private theorem publish_kept {obl : List (Storage32.Obl P)} {q : Slot P} {v : Int}
    {o : Storage32.Obl P} :
    o ∈ Storage32.oblPublished obl q v ↔ o ∈ obl ∧ ¬(o.2.1 = q ∧ v ≥ o.2.2.1) := by
  simp only [Storage32.oblPublished, List.mem_filter, Bool.not_eq_true']
  grind

private theorem role_has_moves {gt : Gate P} {o : Storage32.Obl P} (role : EdgeRole gt o) :
    gt.moves ≠ [] := by
  rcases role with ⟨_, _, ⟨m, mm, _⟩, _⟩ | ⟨m, mm, _⟩
  all_goals intro eq; simp [eq] at mm

theorem edges_published {gates : List (Gate P)} {obl : List (Storage32.Obl P)}
    (edges : OrderEdges gates obl) (q : Slot P) (v : Int)
    (free : ∀ o ∈ obl, o.1 ≠ q) :
    OrderEdges (published gates q v) (Storage32.oblPublished obl q v) := by
  constructor
  · intro o om
    obtain ⟨old, kept⟩ := publish_kept.mp om
    obtain ⟨gt, gm, a, b, ver, role⟩ := edges.1 o old
    let ht := {gt with pending := exclude gt.pending [q]}
    by_cases hv : (gt.vers q).any (fun wanted => v >= wanted) = true
    · have am : o.1 ∈ ht.pending := mem_exclude.mpr ⟨a, by simpa using free o old⟩
      have bm : o.2.1 ∈ ht.pending := by
        apply mem_exclude.mpr
        refine ⟨b, ?_⟩
        intro same
        have eq : o.2.1 = q := by simpa using same
        exact kept ⟨eq, by simpa [← eq, ver] using hv⟩
      have live := retained_some (gt := ht) (role_has_moves role) (by intro eq; simp [eq] at am)
      exact ⟨ht, List.mem_filterMap.mpr ⟨gt, gm, by simpa [hv, ht] using live⟩, am, bm, ver, role⟩
    · have live := retained_some (gt := gt) (role_has_moves role) (by intro eq; simp [eq] at a)
      exact ⟨gt, List.mem_filterMap.mpr ⟨gt, gm, by simpa [hv] using live⟩, a, b, ver, role⟩
  · intro ht hm m mm src dst w ver
    obtain ⟨gt, gm, map⟩ := List.mem_filterMap.mp hm
    split at map
    · obtain rfl := retained_eq map
      obtain ⟨src, _⟩ := mem_exclude.mp src
      obtain ⟨dst, nq⟩ := mem_exclude.mp dst
      exact publish_kept.mpr ⟨edges.2 gt gm m mm src dst w ver, fun pair => nq (by simpa using pair.1)⟩
    · rename_i stays
      obtain rfl := retained_eq map
      refine publish_kept.mpr ⟨edges.2 gt gm m mm src dst w ver, ?_⟩
      rintro ⟨eq, le⟩
      exact stays (by simpa [← eq, ver] using le)

private def cancels (obl : List (Storage32.Obl P)) (p : Slot P) : Bool :=
  obl.any fun o => o.2.1 == p && o.2.2.2 == p

private theorem discard_kept {obl : List (Storage32.Obl P)} {p : Slot P} {o : Storage32.Obl P} :
    o ∈ Storage32.oblDiscarded obl p ↔
      o ∈ obl ∧ o.1 ≠ p ∧ o.2.1 ≠ p ∧ ¬(cancels obl p = true ∧ o.2.2.2 = p) := by
  simp only [Storage32.oblDiscarded, cancels, List.mem_filter, Bool.not_eq_true']
  grind

theorem edges_discarded {gates : List (Gate P)} {obl : List (Storage32.Obl P)}
    (own : GateOwnership gates) (edges : OrderEdges gates obl) (p : Slot P) :
    OrderEdges (discarded gates p) (Storage32.oblDiscarded obl p) := by
  constructor
  · intro o om
    obtain ⟨old, na, nb, kept⟩ := discard_kept.mp om
    obtain ⟨gt, gm, am, bm, ver, role⟩ := edges.1 o old
    by_cases pending : p ∈ gt.pending
    · let gone := gt.moves.filter fun m => m.dst == p
      let ht := {gt with
        moves := gt.moves.filter (fun m => m.dst != p)
        pending := exclude gt.pending (p :: gone.map Move.src)}
      have member (m : Move P) (mm : m ∈ gt.moves) (ne : m.dst ≠ p) : m ∈ ht.moves := by
        simp [ht, mm, ne]
      have removedSource (q : Slot P) (h : q ∈ gone.map Move.src) :
          ∃ m ∈ gt.moves, m.dst = p ∧ m.src = q := by
        simpa [gone, and_assoc] using h
      have noSrc (q : Slot P) (ns : ¬Source gt q) : q ∉ gone.map Move.src := by
        intro h
        obtain ⟨m, mm, _, eq⟩ := removedSource q h
        exact ns ⟨m, mm, eq.symm⟩
      have notSourceAfter (q : Slot P) (ns : ¬Source gt q) : ¬Source ht q := by
        rintro ⟨m, mm, same⟩
        exact ns ⟨m, (List.mem_filter.mp mm).1, same⟩
      have frame :
          o.1 ∉ gone.map Move.src ∧ o.2.1 ∉ gone.map Move.src ∧ EdgeRole ht o := by
        rcases role with ⟨nr, ns, ⟨m, mm, eq⟩, tag⟩ | ⟨m, mm, eq, tag, ns⟩
        · have nsq : ¬Source gt o.2.1 := fun src =>
            source_not_receiver (own.1 gt gm) src ⟨m, mm, eq⟩
          refine ⟨noSrc _ ns, noSrc _ nsq, Or.inl ⟨?_, notSourceAfter _ ns, ?_, tag⟩⟩
          · rintro ⟨n, nm, eq⟩
            exact nr ⟨n, (List.mem_filter.mp nm).1, eq⟩
          · exact ⟨m, member m mm (fun h => nb (eq.trans h)), eq⟩
        · have survives : m.dst ≠ p := by
            intro dest
            have dm : m.dst ∈ gt.pending := dest ▸ pending
            have opt := (own.1 gt gm).2.1 _ (endpoint_mem mm).1
            obtain ⟨w, vw⟩ := Option.isSome_iff_exists.mp opt
            have edge := edges.2 gt gm m mm (eq ▸ am) dm w vw
            have cancel : cancels obl p = true :=
              List.any_eq_true.mpr ⟨(m.src, m.dst, w, m.dst), edge, by simp [dest]⟩
            exact kept ⟨cancel, tag.symm.trans dest⟩
          refine ⟨?_, noSrc _ ns, Or.inr ⟨m, member m mm survives, eq, tag, notSourceAfter _ ns⟩⟩
          intro rm
          obtain ⟨n, nm, nd, np⟩ := removedSource o.1 rm
          have same := endpoint_unique (own.1 gt gm).1 mm nm (Or.inr eq.symm) (Or.inr np.symm)
          exact survives (same ▸ nd)
      have ap : o.1 ∈ ht.pending := mem_exclude.mpr ⟨am, by simpa using And.intro na frame.1⟩
      have bp : o.2.1 ∈ ht.pending := mem_exclude.mpr ⟨bm, by simpa using And.intro nb frame.2.1⟩
      have live := retained_some (gt := ht) (role_has_moves frame.2.2) (by intro eq; simp [eq] at ap)
      exact ⟨ht, List.mem_filterMap.mpr ⟨gt, gm, by simpa [pending, ht, gone] using live⟩,
        ap, bp, ver, frame.2.2⟩
    · have live := retained_some (gt := gt) (role_has_moves role) (by intro eq; simp [eq] at am)
      exact ⟨gt, List.mem_filterMap.mpr ⟨gt, gm, by simpa [pending] using live⟩, am, bm, ver, role⟩
  · intro ht hm m mm src dst w ver
    obtain ⟨gt, gm, map⟩ := List.mem_filterMap.mp hm
    apply discard_kept.mpr
    split at map
    · obtain rfl := retained_eq map
      obtain ⟨src, ns⟩ := mem_exclude.mp src
      obtain ⟨dst, nd⟩ := mem_exclude.mp dst
      have ns' : m.src ≠ p := fun eq => ns (by simp [eq])
      have nd' : m.dst ≠ p := fun eq => nd (by simp [eq])
      exact ⟨edges.2 gt gm m (List.mem_filter.mp mm).1 src dst w ver, ns', nd', fun h => nd' h.2⟩
    · rename_i notPending
      obtain rfl := retained_eq map
      have absentPending : p ∉ gt.pending := by simpa using notPending
      have ns : m.src ≠ p := fun eq => absentPending (eq ▸ src)
      have nd : m.dst ≠ p := fun eq => absentPending (eq ▸ dst)
      exact ⟨edges.2 gt gm m mm src dst w ver, ns, nd, fun h => nd h.2⟩

end Stage6
