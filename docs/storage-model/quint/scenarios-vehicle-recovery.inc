  // Five pages: two disjoint moves and one shared referrer.
  action noop = all { s' = s, g' = g }
  action vehicleLoad = load(0).then(load(1)).then(load(2)).then(load(3)).then(load(4))
  action vehicleSave(p: int): bool = flush(p).then(check).then(rename).then(dirSync(true))
  action vehicleRemove(p: int): bool = flushDel(p).then(check).then(rename).then(dirSync(true))
  action vehicleOpen(p: int): bool = wOpen(p).then(deliverUp(true)).then(wRecv(p))
  action vehicleDiscard(p: int): bool = wDiscard(p).then(deliverUp(true)).then(wRecv(p))
  action vehicleApply = opApplyMoves(Map(0 -> ABSENT, 1 -> ABSENT, 2 -> 1, 3 -> 2, 4 -> 3),
    Map(0 -> 1, 1 -> 2, 2 -> ABSENT, 3 -> ABSENT, 4 -> ABSENT), Map(2 -> 0, 3 -> 1))
  action vehicleOverlap = opApplyMoves(Map(0 -> ABSENT, 2 -> 1, 3 -> 1),
    Map(0 -> 1, 2 -> ABSENT, 3 -> ABSENT), Map(2 -> 0, 3 -> 0))
  action vehicleMembers = vehicleLoad.then(vehicleApply).then(vehicleSave(2)).then(vehicleSave(3)).then(vehicleSave(4))
  action vehicleFirstSettled = vehicleMembers.then(vehicleRemove(0))
  action vehicleFirstCancelled = vehicleLoad.then(vehicleApply).then(vehicleOpen(2)).then(vehicleDiscard(2))
  val vehicleSafe = guarantee and orderHolds and renameResolves and draftOwner and retainedRecords and disjointPairs
  val vehicleIntact = s.pairs.length() == 2 and s.pairs.nth(0).vehicle == s.pairs.nth(1).vehicle
  val onlySibling = s.pairs.length() == 1 and s.pairs.head().src == 1 and s.pairs.head().records.get(4).bytes == 3
    and s.drafts.get(4).bytes == 3 and s.drafts.get(0) == NODRAFT and s.drafts.get(2) == NODRAFT
  val firstSettled = vehicleIntact and s.pairs.nth(0).phase == "settled" and s.pairs.nth(1).phase == "armed"
  val firstCancelled = vehicleIntact and s.pairs.nth(0).decision == "cancelled" and s.pairs.nth(1).decision == "none"
  action vehicleFinishSibling = vehicleSave(3).then(vehicleSave(4)).then(vehicleRemove(1)).then(pairRewrite(1, true))

  run ownVehicleRejectsOverlappingMoves = init.then(vehicleLoad).expect(vehicleSafe).then(vehicleOverlap.fail())
  run ownVehicleOrdersBeforeIndependentRemovals = init.then(vehicleLoad).then(vehicleApply).then(vehicleSave(2))
    .expect(gated(s, 4) and gated(s, 0)).then(vehicleSave(3)).expect(gated(s, 0))
    .then(vehicleSave(4)).then(vehicleRemove(1)).then(vehicleRemove(0)).expect(vehicleSafe and s.gates == List())
  run ownVehicleCancellationKeepsSibling = init.then(vehicleFirstCancelled).expect(firstCancelled and vehicleSafe)
    .then(pairRewrite(0, true)).expect(onlySibling and vehicleSafe)
    .then(crash).then(launch).then(vehicleFinishSibling).then(crash).then(launch)
    .expect(s.pairs == List() and s.disk.get(0) == 1 and s.disk.get(2) == ABSENT
      and s.disk.get(1) == ABSENT and s.disk.get(3) == 2 and vehicleSafe)
  run ownVehicleRewriteRemovesOnlySettledPair = init.then(vehicleFirstSettled).expect(firstSettled and vehicleSafe)
    .then(pairRewrite(0, true)).expect(onlySibling and vehicleSafe)
    .then(extWrite(2, 3)).then(crash).then(launch).expect(onlySibling and not(s.pages.get(2).held) and vehicleSafe)
    .then(vehicleFinishSibling).expect(s.pairs == List() and s.disk.get(2) == 3 and vehicleSafe)
  run ownVehicleCrashBeforeRewriteRewitnessesBoth = init.then(vehicleFirstSettled).then(crash).then(launch)
    .expect(vehicleIntact and vehicleSafe).then(vehicleSave(2)).then(vehicleSave(3)).then(vehicleSave(4))
    .then(vehicleRemove(0)).expect(firstSettled and vehicleSafe).then(pairRewrite(0, true)).expect(onlySibling and vehicleSafe)
  run ownVehicleFailedRewriteCanReinstallConflict = init.then(vehicleFirstSettled).then(pairRewrite(0, false))
    .expect(firstSettled and vehicleSafe).then(extWrite(2, 3)).then(crash).then(launch)
    .then(flush(2)).then(check).expect(vehicleIntact and s.pages.get(2).conflict and vehicleSafe)

  action vehicleHeldSource = vehicleMembers.then(extWrite(2, 3)).then(flushDel(0)).then(check)
  action vehicleNewerSource = vehicleHeldSource.then(vehicleOpen(0)).then(wEdit(0, 3)).then(wSend(0))
    .then(deliverUp(true)).then(wRecv(0)).then(draftSync(0)).then(vehicleSave(0))
  val sourceRetained = vehicleIntact and s.pairs.nth(0).phase == "retained" and s.pairs.nth(0).decision == "disarmed"
    and s.pairs.nth(0).records.get(0).base == 1 and s.disk.get(0) == 3 and s.disk.get(2) == 3
  run ownVehicleNewerSourceKeepsUnanchoredPreimage = init.then(vehicleNewerSource).expect(sourceRetained and vehicleSafe)
    .then(crash).then(launch).expect(sourceRetained and s.pages.get(0).buf == 3 and vehicleSafe)
    .then(pairRewrite(0, true).fail())

  action vehicleRecoverExact = vehicleMembers.then(flushDel(0)).then(check).then(rename).then(extWrite(2, 3))
    .then(dirSync(true)).then(vehicleOpen(2)).then(vehicleDiscard(2)).then(extWrite(2, 1)).then(crash).then(launch)
  val launchNeedsWitness = s.pairs.nth(0).phase == (if (RACES.contains("weak")) "cancelled" else "restore")
  run ownVehicleLaunchCannotCertifyDurableBytes = init.then(vehicleRecoverExact).expect(launchNeedsWitness and vehicleSafe)
    .then(crash).then(launch).expect(launchNeedsWitness and vehicleSafe)

  run mMOverlapMoves = init.then(if (MUTANT == "MOverlapMoves") vehicleLoad.then(vehicleOverlap) else noop).expect(disjointPairs)
  run mMSharedChoice = init.then(if (MUTANT == "MSharedChoice") vehicleFirstCancelled else noop)
    .expect(MUTANT != "MSharedChoice" or firstCancelled)
  run mMDropSibling = init.then(if (MUTANT == "MDropSibling") vehicleFirstSettled.then(pairRewrite(0, true)) else noop)
    .expect(MUTANT != "MDropSibling" or onlySibling)
  run mMDropShared = init.then(if (MUTANT == "MDropShared") vehicleFirstSettled.then(pairRewrite(0, true)) else noop)
    .expect(MUTANT != "MDropShared" or onlySibling)
  run mMRewriteOnFailure = init.then(if (MUTANT == "MRewriteOnFailure") vehicleFirstSettled.then(pairRewrite(0, false)) else noop)
    .expect(MUTANT != "MRewriteOnFailure" or firstSettled)
  run mMDropPreimage = init.then(if (MUTANT == "MDropPreimage") vehicleNewerSource else noop)
    .expect(MUTANT != "MDropPreimage" or sourceRetained)
  run mMUseLaunchBytes = init.then(if (MUTANT == "MUseLaunchBytes") vehicleRecoverExact else noop)
    .expect(MUTANT != "MUseLaunchBytes" or launchNeedsWitness)
