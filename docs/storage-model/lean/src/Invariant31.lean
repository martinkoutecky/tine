import Storage31
import GClosure6
import TypedClosure5
namespace Storage31
open Storage21 Storage3
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

-- Each field is carried along actual s3.1 histories; none is assumed at the headline.
structure Inv (c : Config P) (x : State P) : Prop where
  inactive : InactivePages (project x)
  protocol : ProtocolInv3 x
  snapshot : JobSnapshot x.s.toSys
  shapes : Shapes x.s.toSys
  bounds : VersionBounds x.s.toSys x.g.vc
  clear : JobClear x.s.toSys
  bytes : ByteDomain c x.s.toSys
  lineage : PromiseLineage (project x)
  windowSeen : WindowSeen (project x)
  carrierSeen : CarrierSeen (project x)
  epochs : EpochBounds x.s.toSys x.g.toGhost
  distinct : QueueDistinct x.s.toSys
  active : ActiveAgreement x.s.toSys x.g.toGhost
  fence : PublicationFence x.s.toSys x.g.toGhost
  promiseBytes : PromiseBytes c x.g.toGhost
  obs : ObsOnly c x.s.toSys
  protection : TypedProtected5 (project x)
  custody : Custody c (project x)
  authority : AuthorityInv6 x
  C : CInv c (project x)
  B : BInv (project x)
  W : WInv (project x)
  taken : TakenInv (project x)
  newTags : NewTagsClean (project x)
  vocabulary : BadVocabulary x.g.toGhost
  unheld : "request-to-unheld" ∉ x.g.bad
  G : GOnly (project x)
  trash : TrashInv x
end Storage31
