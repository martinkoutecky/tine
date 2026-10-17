import Guarantee31
import Safety35
namespace Storage21
theorem commit_named_bad10 (c : Config P) (x : State P) (ns : Sys P) (ng : Ghost P)
    (ps : List (Fin P)) (name : String)
    (unchanged : ∀ p,p ∉ ps → ns.pages p = x.s.pages p)
    (named : ∀ p ∈ ps,p ∈ ng.mine) (windows : ns.w = x.s.w) (bad : ng.bad = x.g.bad) :
    (commit c x ns ng name).g.bad = x.g.bad := by
  have b : ∀ np : Fin P → Promise, (ids P).all (fun p => keptB x p ns np ng.mine name) = true := by
    intro np
    apply List.all_eq_true.mpr
    intro p _
    by_cases mem : p ∈ ps
    · simp [keptB,List.contains_iff_mem,named p mem]
    · simp [keptB,unchanged p mem]
  have w : (ids P).all (fun p => keptW x p ns ng.owed name) = true := by simp [keptW,windows]
  simp [commit,b,w,bad,tags,union]
end Storage21
namespace Storage31
open Storage21 Storage3
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

-- An internal proof factorization, not an additional model action.
def silentMsg10 (c : Config P) (p : Fin P) : Up P :=
  {kind := "discard",p,q := zero c,t := 0,bv := 0,ro := noneBytes,cur := false}
def silentRestore10 (c : Config P) (x : State P) (p : Fin P) : State P :=
  Storage3.lift x (Storage21.upDiscard c (project x) x.s.toSys (silentMsg10 c p) true)
@[simp] theorem silent_project10 (c : Config P) (x : State P) (p : Fin P) :
    project (silentRestore10 c x p) =
      Storage21.upDiscard c (project x) x.s.toSys (silentMsg10 c p) true := rfl

theorem silent_bad10 (c : Config P) (x : State P) (p : Fin P)
    (held : (x.s.pages p).held = true) : (silentRestore10 c x p).g.bad = x.g.bad := by
  unfold silentRestore10 Storage3.lift Storage21.upDiscard
  simp only [silentMsg10,project,held,Bool.not_true,Bool.false_eq_true,ite_false]
  apply commit_named_bad10 (ps := [p])
  · intro q hq; simp at hq; simp [put,hq]
  · intro q hq; simpa using hq
  · rfl
  · rfl

theorem silent_inv10 (c : Config P) (x : State P) (p : Fin P)
    (mutant : c.mutant = "none") (h : Inv c x)
    (held : (x.s.pages p).held = true) (unlocked : locked x.s.toSys p = false) :
    Inv c (silentRestore10 c x p) := by
  let m := silentMsg10 c p
  have eq := silent_project10 c x p
  have bad := silent_bad10 c x p held
  have core : project (silentRestore10 c x p) =
      Storage21.upDiscard c (project x) {x.s.toSys with up := x.s.up} m true := rfl
  have jobne : x.s.job.on = true → x.s.job.p ≠ p := by
    intro on same
    simp [locked,on,same] at unlocked
  refine {
    inactive := ?_
    protocol := ?_
    snapshot := ?_
    shapes := by change Shapes (project (silentRestore10 c x p)).s
                 rw [core]; exact shape_upDiscard c (project x) _ m true h.shapes
    bounds := by change VersionBounds (project (silentRestore10 c x p)).s (project (silentRestore10 c x p)).g.vc
                 rw [core]; exact bounds_upDiscard c (project x) _ m true h.bounds
    clear := ?_
    bytes := by change ByteDomain c (project (silentRestore10 c x p)).s
                rw [core]; exact bytes_upDiscard c (project x) _ m true h.bytes h.bytes
    lineage := by rw [core]; exact lineage_upDiscard c (project x) _ m true h.lineage h.bounds
    windowSeen := by rw [core]; simp only [Storage21.upDiscard,silentMsg10,m,held,Bool.not_true,Bool.false_eq_true,ite_false]
                     apply windowSeen_commit; exact h.bytes.w
    carrierSeen := ?_
    epochs := by change EpochBounds (project (silentRestore10 c x p)).s (project (silentRestore10 c x p)).g
                 rw [core]; exact epochs_upDiscard c (project x) _ m true h.epochs
    distinct := by simpa [QueueDistinct,silentRestore10,Storage3.lift,Storage21.upDiscard,silentMsg10,held,project,Storage21.commit,ackMail] using h.distinct
    active := by change ActiveAgreement (project (silentRestore10 c x p)).s (project (silentRestore10 c x p)).g
                 rw [core]; exact active_upDiscard c (project x) m x.s.up true h.active h.lineage.1
    fence := by change PublicationFence (project (silentRestore10 c x p)).s (project (silentRestore10 c x p)).g
                rw [core]; exact fence_upDiscard c (project x) m x.s.up true h.fence
    promiseBytes := by change PromiseBytes c (project (silentRestore10 c x p)).g
                       rw [core]; exact promiseBytes_upDiscard c (project x) _ m true h.promiseBytes
                         (bytes_upDiscard c (project x) _ m true h.bytes h.bytes).pages
    obs := by change ObsOnly c (project (silentRestore10 c x p)).s
              rw [core]; exact obsOnly_upDiscard c (project x) _ m true h.obs h.obs
    protection := by rw [core]; exact protected_upDiscard5 c (project x) _ m true h.protection
    custody := by rw [core]; exact custody_upDiscard c mutant (project x) m x.s.up true h.custody
    authority := ?_
    C := by rw [core]; exact C_upDiscard c (project x) _ m true rfl rfl h.C
    B := by rw [core]; exact B_upDiscard c (project x) m x.s.up true h.B
    W := by rw [core]; exact W_upDiscard c (project x) m x.s.up true h.W
    taken := by rw [core]; exact taken_upDiscard c (project x) _ m true h.taken
    newTags := by simpa only [NewTagsClean,project,bad] using h.newTags
    vocabulary := by intro t ht; exact h.vocabulary t (bad ▸ ht)
    unheld := by rw [bad]; exact h.unheld
    G := by simpa only [GOnly,project,bad] using h.G
    trash := h.trash
  }
  · intro down
    have oldDown : x.s.alive = false := by
      simpa [silentRestore10,Storage3.lift,Storage21.upDiscard,silentMsg10,held,project,Storage21.commit,ackMail] using down
    exact False.elim (Bool.noConfusion (held.symm.trans (h.inactive oldDown p)))
  · refine ⟨?_,?_,?_,?_⟩
    · simpa [Kinds,silentRestore10,Storage3.lift,Storage21.upDiscard,silentMsg10,held,project,Storage21.commit,ackMail] using h.protocol.kinds
    · intro down
      have oldDown : x.s.alive = false := by
        simpa [silentRestore10,Storage3.lift,Storage21.upDiscard,silentMsg10,held,project,Storage21.commit,ackMail] using down
      exact False.elim (Bool.noConfusion (held.symm.trans (h.inactive oldDown p)))
    · change Lifetime (project (silentRestore10 c x p)).s
      rw [core]
      simp only [Storage21.upDiscard,silentMsg10,m,held,Bool.not_true,Bool.false_eq_true,ite_false,ackMail,Bool.not_false,ite_true]
      apply lifetime_commit
      apply lifetime_frame x.s.toSys _ h.protocol.life
      · intro q; by_cases same : q = p
        · subst q; simp [put,held]
        · simp [put,same]
      · rfl
      · rfl
      · rfl
    · rw [core]
      simpa [Storage21.upDiscard,silentMsg10,m,held,carriage_commit,ackMail,Storage21.commit,Carriage,QueueCarries,project] using h.protocol.carriage
  · intro on
    have oldOn : x.s.job.on = true := by
      simpa [silentRestore10,Storage3.lift,Storage21.upDiscard,silentMsg10,held,project,Storage21.commit,ackMail] using on
    have ne := jobne oldOn
    simpa [silentRestore10,Storage3.lift,Storage21.upDiscard,silentMsg10,held,project,Storage21.commit,ackMail,put,ne] using h.snapshot oldOn
  · intro on
    have oldOn : x.s.job.on = true := by
      simpa [silentRestore10,Storage3.lift,Storage21.upDiscard,silentMsg10,held,project,Storage21.commit,ackMail] using on
    have ne := jobne oldOn
    simpa [silentRestore10,Storage3.lift,Storage21.upDiscard,silentMsg10,held,project,Storage21.commit,ackMail,put,ne] using h.clear oldOn
  · apply carrierSeen_frame (project x) (project (silentRestore10 c x p)) h.carrierSeen
    · simp [silentRestore10,Storage3.lift,Storage21.upDiscard,silentMsg10,held,project,Storage21.commit,ackMail]
    · rw [core]
      simp only [Storage21.upDiscard,silentMsg10,m,held,Bool.not_true,Bool.false_eq_true,ite_false]
      exact seen_commit c (project x) _ _ "uDiscard"
  · let z := Storage21.upDiscard c (shadow6 x) (shadow6 x).s m true
    have sys : (shadow6 (silentRestore10 c x p)).s = z.s := by
      simp [z,silentRestore10,Storage3.lift,Storage21.upDiscard,silentMsg10,m,held,shadow6,project,Storage21.commit]
    have guard : (shadow6 (silentRestore10 c x p)).g.guard = z.g.guard := by
      simp [z,silentRestore10,Storage3.lift,Storage21.upDiscard,silentMsg10,m,held,shadow6,project,Storage21.commit]
    have mono : AuthMono z.g (shadow6 (silentRestore10 c x p)).g := by
      intro q b hb
      simpa [z,silentRestore10,Storage3.lift,Storage21.upDiscard,silentMsg10,m,held,shadow6,project,Auth,Storage21.commit] using hb
    have structural := G_upDiscard c (shadow6 x) (shadow6 x).s m true h.authority.structural
    have anchor : Anchors z := by
      have evolution := evolution_upDiscard c (shadow6 x) (shadow6 x).s m true (evolution_refl (shadow6 x))
      simp only [z,Storage21.upDiscard,silentMsg10,m,shadow6,held,Bool.not_true,Bool.false_eq_true,ite_false,
        ackMail,Bool.not_false,ite_true] at evolution ⊢
      apply anchors_commit
      apply anchors_transport (shadow6 x) _ _ h.authority.anchors h.bounds
      · simpa only [evolution_commit,shadow6] using evolution
      · rfl
      · rfl
      · intro u hu; exact hu
      · exact authMono_refl (shadow6 x).g
    have resolution : ResolutionAuth z := by
      apply resolutionAuth_frame c (shadow6 x) z h.authority.resolution
      · simp [z,Storage21.upDiscard,silentMsg10,m,held,shadow6,project,Storage21.commit,ackMail]
      · exact authMono_upDiscard c (shadow6 x) (shadow6 x).s m true
    exact ⟨shadow_Gtransport6 _ _ structural sys guard mono,
      shadow_anchors_transport6 _ _ anchor sys mono,
      shadow_resolution_transport6 _ _ resolution sys mono⟩
end Storage31
