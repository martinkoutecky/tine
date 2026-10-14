import GTransport6
namespace Storage3
open Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

def shadowGhost6 (g : Ghost P) : Storage21.Ghost P :=
  {g.toGhost with bad := [], wrote := fun p => union (g.wrote p) ((g.opRead p).map fun b => (b,0))}

theorem shadow_auth_mono6 (g k : Ghost P)
    (seen : ∀ p b, b ∈ g.seen p → b ∈ k.seen p)
    (wrote : ∀ p w, w ∈ g.wrote p → w ∈ k.wrote p)
    (read : ∀ p b, b ∈ g.opRead p → b ∈ k.opRead p) :
    AuthMono (shadowGhost6 g) (shadowGhost6 k) := by
  intro p b hb
  rcases hb with hs | ⟨v,hv⟩
  · exact Or.inl (seen p b hs)
  · rcases (mem_union_general _ _ _).mp hv with hw | hr
    · right; exact ⟨v,(mem_union_general _ _ _).mpr (Or.inl (wrote p (b,v) hw))⟩
    · obtain ⟨t,ht,eq⟩ := List.mem_map.mp hr
      have tb : t = b := congrArg Prod.fst eq
      subst t
      right; exact ⟨0,(mem_union_general _ _ _).mpr (Or.inr (List.mem_map.mpr ⟨b,read p b ht,rfl⟩))⟩

theorem shadow_Gcommit6 (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String)
    (h : GInv ⟨s.toSys,shadowGhost6 g⟩) : GInv (shadow6 (commit c x s g n)) := by
  apply shadow_Gtransport6 _ (Storage21.commit c (shadow6 x) s.toSys (shadowGhost6 g) n)
  · exact G_commit c (shadow6 x) _ _ _ h
  · rfl
  · rfl
  · intro p b hb; exact hb

theorem shadow_anchors_commit6 (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String)
    (h : Anchors ⟨s.toSys,shadowGhost6 g⟩) : Anchors (shadow6 (commit c x s g n)) := by
  apply shadow_anchors_transport6 _ (Storage21.commit c (shadow6 x) s.toSys (shadowGhost6 g) n)
  · exact anchors_commit c (shadow6 x) _ _ _ h
  · rfl
  · intro p b hb; exact hb

theorem shadow_resolution_commit6 (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String)
    (h : ResolutionAuth ⟨s.toSys,shadowGhost6 g⟩) : ResolutionAuth (shadow6 (commit c x s g n)) := by
  apply shadow_resolution_transport6 _ (Storage21.commit c (shadow6 x) s.toSys (shadowGhost6 g) n)
  · exact resolutionAuth_commit c (shadow6 x) _ _ _ h
  · rfl
  · intro p b hb; exact hb

end Storage3
