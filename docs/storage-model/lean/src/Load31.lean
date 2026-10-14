import Old31
namespace Storage31
open Storage21 Storage3
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

theorem load_live (c : Config P) (x : State P) (p : Fin P)
    (hen : Storage31.enabled c x (.load p) = true) : x.s.alive = true ∧ (x.s.pages p).held = false := by
  simpa [Storage31.enabled] using hen

theorem load_evolution (c : Config P) (x : State P) (p : Fin P)
    (unheld : (x.s.pages p).held = false) :
    PageEvolution (project x) (load c x p).s.toSys := by
  have eq := load_upOpen c x p unheld
  change PageEvolution (project x) (project (load c x p)).s
  rw [eq]
  exact evolution_upOpen c (project x) x.s.toSys (openMsg c p) true (evolution_refl (project x))

theorem authority_load (c : Config P) (x : State P) (p : Fin P)
    (unheld : (x.s.pages p).held = false) (h : AuthorityInv6 x)
    (bounds : VersionBounds x.s.toSys x.g.vc) : AuthorityInv6 (load c x p) := by
  let y := Storage21.upOpen c (shadow6 x) (shadow6 x).s (openMsg c p) true
  have sys : (shadow6 (load c x p)).s = y.s := by
    simp [y,load,shadow6,Storage3.commit,project,Storage21.upOpen,openMsg,unheld,ackMail,Storage21.commit]
  have guard : (shadow6 (load c x p)).g.guard = y.g.guard := by
    simp [y,load,shadow6,Storage3.commit,project,Storage21.upOpen,openMsg,unheld,ackMail,Storage21.commit]
  have mono : AuthMono y.g (shadow6 (load c x p)).g := by
    intro q b hb
    simpa [y,load,shadow6,Storage3.commit,project,Storage21.upOpen,openMsg,unheld,ackMail,Storage21.commit,Auth] using hb
  have gi := G_upOpen c (shadow6 x) (shadow6 x).s (openMsg c p) true h.structural
  have anchor : Anchors y := by
    simp only [y,Storage21.upOpen,shadow6,openMsg,unheld,Bool.false_or,Bool.not_true,
      Bool.false_eq_true,ite_false,beq_self_eq_true,ite_true,ackMail,Bool.not_false,Bool.true_eq_false]
    apply anchors_commit
    apply anchors_transport (shadow6 x) _ _ h.anchors bounds
    · intro q held; by_cases same : q = p
      · subst q; right; simp [put,shadow6]; omega
      · left; simp [put,same] at held ⊢; exact ⟨held,rfl,rfl⟩
    · rfl
    · rfl
    · intro m mem; exact mem
    · intro q b hb; exact hb
  have resolution : ResolutionAuth y := by
    apply resolutionAuth_frame c (shadow6 x) y h.resolution
    · simp [y,Storage21.upOpen,Storage21.commit,ackMail,openMsg]
    · exact authMono_upOpen c (shadow6 x) (shadow6 x).s (openMsg c p) true
  exact ⟨shadow_Gtransport6 _ _ gi sys guard mono,
    shadow_anchors_transport6 _ _ anchor sys mono,
    shadow_resolution_transport6 _ _ resolution sys mono⟩

theorem inv_load (c : Config P) (x : State P) (p : Fin P)
    (h : Inv c x) (hen : Storage31.enabled c x (.load p) = true) : Inv c (load c x p) := by
  obtain ⟨live,unheld⟩ := load_live c x p hen
  have eq := load_upOpen c x p unheld
  have grow : ∀ q, (x.s.pages q).held = true →
      (put x.s.pages p ({held := true,buf := x.s.disk p,base := x.s.disk p,obs := x.s.disk p,ver := x.g.vc+1} : Page) q).held = true :=
    grow_put x.s.pages x.s.pages p _ (fun _ hp => hp) rfl
  refine {
    inactive := by simp [InactivePages,load,Storage3.commit,project,Storage21.commit,live]
    protocol := ?_
    snapshot := ?_
    shapes := by
      change Shapes (project (load c x p)).s
      rw [eq]; exact shape_upOpen c (project x) _ _ true h.shapes
    bounds := by
      change VersionBounds (project (load c x p)).s (project (load c x p)).g.vc
      rw [eq]; exact bounds_upOpen c (project x) _ _ true h.bounds
    clear := ?_
    bytes := by
      change ByteDomain c (project (load c x p)).s
      rw [eq]; exact bytes_upOpen c (project x) _ _ true h.bytes h.bytes
    lineage := by rw [eq]; exact lineage_upOpen c (project x) _ _ true h.lineage h.bounds
    windowSeen := by
      exact windowSeen_commit c (project x) _ _ _ h.bytes.w
    carrierSeen := by
      apply carrierSeen_frame (project x) (project (load c x p)) h.carrierSeen rfl
      exact seen_commit c (project x) _ _ "load"
    epochs := by
      change EpochBounds (project (load c x p)).s (project (load c x p)).g
      rw [eq]; exact epochs_upOpen c (project x) _ _ true h.epochs
    distinct := by exact h.distinct
    active := by
      change ActiveAgreement (project (load c x p)).s (project (load c x p)).g
      rw [eq]; exact active_upOpen c (project x) _ x.s.up true h.active h.lineage.1
    fence := by
      change PublicationFence (project (load c x p)).s (project (load c x p)).g
      rw [eq]; exact fence_upOpen c (project x) _ x.s.up true h.fence
    promiseBytes := by
      change PromiseBytes c (project (load c x p)).g
      rw [eq]; exact promiseBytes_upOpen c (project x) _ _ true h.promiseBytes
        (bytes_upOpen c (project x) _ _ true h.bytes h.bytes).pages
    obs := by
      change ObsOnly c (project (load c x p)).s
      rw [eq]; exact obsOnly_upOpen c (project x) _ _ true h.obs h.obs
    protection := by rw [eq]; exact protected_upOpen5 c (project x) _ _ true h.protection
    custody := by rw [eq]; exact custody_upOpen c (project x) _ x.s.up true h.custody
    authority := authority_load c x p unheld h.authority h.bounds
    C := by rw [eq]; exact C_upOpen c (project x) _ _ true rfl rfl h.C
    B := by rw [eq]; exact B_upOpen c (project x) _ x.s.up true h.B
    W := by rw [eq]; exact W_upOpen c (project x) _ x.s.up true h.W
    taken := by rw [eq]; exact taken_upOpen c (project x) _ _ true h.taken
    newTags := by simpa [load,Storage3.commit,project,Storage21.commit,NewTagsClean] using h.newTags
    vocabulary := by
      unfold load Storage3.commit project
      exact vocabulary_commit c (project x) _ _ "load" h.vocabulary
    unheld := by simpa [load,Storage3.commit,project,Storage21.commit] using h.unheld
    G := by simpa [load,Storage3.commit,project,Storage21.commit,GOnly] using h.G
    trash := by exact h.trash
  }
  · refine ⟨h.protocol.kinds,?_,?_,h.protocol.carriage⟩
    · simp [InactiveClient,load,Storage3.commit,project,Storage21.commit,live]
    · exact lifetime_commit_grow c (project x) _ {x.g.toGhost with vc := x.g.vc+1} "load" h.protocol.life live rfl rfl rfl grow
  · intro on
    have ne : x.s.job.p ≠ p := by
      intro same
      have held := (h.snapshot on).1
      rw [same,unheld] at held
      contradiction
    simpa [load,Storage3.commit,project,Storage21.commit,put,ne] using h.snapshot on
  · intro on
    have ne : x.s.job.p ≠ p := by
      intro same
      have held := (h.snapshot on).1
      rw [same,unheld] at held
      contradiction
    simpa [load,Storage3.commit,project,Storage21.commit,put,ne] using h.clear on
end Storage31
