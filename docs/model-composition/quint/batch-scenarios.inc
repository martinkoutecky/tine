  // Slot layout from finish.inc: s1=0, referrer=1, d1=2, s2=3, d2=4.
  pure val batchBefore = Map(0 -> 1, 1 -> 2, 2 -> ABSENT, 3 -> 2, 4 -> ABSENT)
  pure val batchAfter = Map(0 -> ABSENT, 1 -> 3, 2 -> 1, 3 -> ABSENT, 4 -> 2)
  pure val batchMoves = List((2, 0), (4, 3))
  action batchPrepared = init.then(extWrite(3, 2)).then(load(0)).then(load(1)).then(load(3))
    .then(open(2)).then(open(4))
  action admitBatch = opBatch(batchAfter, batchBefore, batchMoves)
  action batchAdmitted = batchPrepared.then(admitBatch)

  // Production checks a late twin at the guard. An exact-path creation is no-clobber,
  // and replacing a write after its publication read makes the sync witness uncertain.
  action batchCheck = if (s.job.on and s.job.phase == 1 and s.job.base == ABSENT and s.job.bytes != ABSENT
    and s.disk.get(s.job.p) == ABSENT and s.claimed.contains(s.job.p)) saveFail else check
  action batchRename = if (s.job.on and s.job.phase == 2 and s.job.base == ABSENT and s.job.bytes != ABSENT
    and s.disk.get(s.job.p) != ABSENT) saveFail else rename
  action batchSync = dirSync(s.job.bytes == ABSENT or s.job.ep == g.ext.get(s.job.p))
  action beginWrite(p: int): bool = flushWrite(p, false)
  action saveWrite(p: int): bool = beginWrite(p).then(batchCheck).then(batchRename).then(batchSync)
  action saveDelete(p: int): bool = flushDel(p).then(batchCheck).then(batchRename).then(batchSync)
  action firstReceiver = batchAdmitted.then(saveWrite(2))

  val removedCustody = batchMoves.foldl(true, (holds, move) => {
    val p = move._2
    holds and (s.disk.get(p) != ABSENT or g.removed.get(p).subseteq(s.trash.get(p)))
  })
  val discardIsolation = g.discardOk
  val batchSafety = ok and gateOwnership and requirementsBlock and discardIsolation and removedCustody
  val exactBatch = s.disk == batchAfter and s.stable == batchAfter
  val originalFootprint = s.gates.length() == 1 and owned(s.gates.head()) == PAGES
  val batchFootprintHolds = s.gates.foldl(true, (holds, gt) => holds and owned(gt) == PAGES)
  val firstReceiverOnly = s.disk.get(2) == 1 and s.stable.get(2) == 1 and s.disk.get(4) == ABSENT
  val discardedSecondOnly = s.pages.get(0).buf == ABSENT and s.pages.get(3).buf == 2
    and partner(s, 4) == NONE and originalFootprint
    and s.gates.head().moves == Set((2, 0)) and s.gates.head().open == Set(0, 1)
  action overlapDiscarded = opBatch(Map(3 -> 3), Map(3 -> 2), List())
  action overlapWitnessed = opBatch(Map(2 -> 3), Map(2 -> 1), List())
  run stage6RepeatedBindingRefused = batchPrepared.then(opBatch(batchAfter, batchBefore, List((2, 0), (2, 0))).fail())
  run stage6SharedEndpointRefused = batchPrepared.then(opBatch(batchAfter, batchBefore, List((2, 0), (4, 0))).fail())
  run stage6BothReceiversCanStart = batchAdmitted.expect(batchSafety and originalFootprint)
    .then(saveWrite(4)).then(saveWrite(2)).then(saveWrite(1)).then(saveDelete(3)).then(saveDelete(0))
    .expect(exactBatch and s.gates.length() == 0 and batchSafety)
  run stage6FirstReceiverBlocksReferrer = firstReceiver.expect(firstReceiverOnly and batchSafety).then(beginWrite(1).fail())
  run stage6FirstReceiverBlocksSource = firstReceiver.expect(firstReceiverOnly and batchSafety).then(flushDel(0).fail())
  action secondDiscarded = firstReceiver.then(wDiscard(4)).then(deliverUp(false))
    .expect(partner(s, 4) == 3 and s.pages.get(3).buf == ABSENT and batchSafety)
    .then(wRecv(4)).then(discard(4)).expect(discardedSecondOnly and batchSafety)
  run stage6DiscardedSlotStillOwned = secondDiscarded.then(overlapDiscarded.fail())
  run stage6PublishedSlotStillOwned = secondDiscarded.then(overlapWitnessed.fail())
  run stage6SourceWaitsForReferrerAfterDiscard = secondDiscarded.then(flushDel(0).fail())
  run stage6DiscardIsLocal = secondDiscarded
    .then(saveWrite(1)).then(saveDelete(0)).expect(s.gates.length() == 0 and s.disk.get(3) == 2 and batchSafety)
    .then(overlapDiscarded)
  run stage6DiscardBeforeCleanupCanReplay = secondDiscarded.then(crash).then(launch)
    .expect(s.gates.length() == 0 and s.pages.get(3).buf == ABSENT and s.pages.get(4).buf == 2 and batchSafety)
  run stage6LastDiscardRetiresGate = batchAdmitted.then(discard(4)).then(discard(2))
    .expect(s.gates.length() == 0 and g.obl == Set() and s.pages.get(0).buf == 1 and s.pages.get(3).buf == 2 and batchSafety)
    .then(saveWrite(1))
  run stage6CrashBetweenReceivers = firstReceiver.then(crash)
    .expect(firstReceiverOnly and s.gates.length() == 0 and batchSafety)
    .then(launch).then(saveDelete(3))
    .expect(s.disk.get(4) == ABSENT and s.trash.get(3).contains(2) and batchSafety)
    .then(saveWrite(4)).then(observe(2)).then(saveWrite(2)).then(saveWrite(1)).then(saveDelete(0))
    .expect(exactBatch and batchSafety)
  action secondBeforeSync = firstReceiver.then(beginWrite(4)).then(batchCheck).then(batchRename)
    .expect(s.disk.get(4) == 2 and s.stable.get(4) == ABSENT and gated(s, 1) and batchSafety)
  run stage6PowerRevertsReceiver = secondBeforeSync.then(powerK(Set())).then(launch)
    .expect(s.disk.get(4) == ABSENT and s.pages.get(4).buf == 2 and s.gates.length() == 0 and batchSafety)
  run stage6PowerKeepsReceiver = secondBeforeSync.then(powerK(Set(4))).then(launch)
    .expect(s.disk.get(4) == 2 and s.stable.get(4) == 2 and s.gates.length() == 0 and batchSafety)
  run stage6ConflictKeepsOwed = firstReceiver.then(extWrite(4, 3)).then(beginWrite(4)).then(batchCheck)
    .expect(s.pages.get(4).conflict and s.disk.get(2) == 1 and s.disk.get(4) == 3
      and List(0, 3).foldl(true, (held, p) => held and s.disk.get(p) == batchBefore.get(p)
        and s.drafts.get(p).base == batchBefore.get(p))
      and Set(0, 1, 3, 4).forall(p => s.drafts.get(p).bytes == batchAfter.get(p)) and batchSafety)
    .expect(gated(s, 1) and gated(s, 0)).then(crash).then(launch)
    .expect(Set(0, 1, 3, 4).forall(p => s.pages.get(p).buf == batchAfter.get(p)) and batchSafety)
  run stage6ClaimAtAdmission = batchPrepared.then(extClaim(4, true)).then(admitBatch.fail())
  run stage6ClaimAtGuard = firstReceiver.then(beginWrite(4)).then(extClaim(4, true)).then(batchCheck)
    .expect(not(s.job.on) and s.disk.get(4) == ABSENT and gated(s, 1) and batchSafety)
  run stage6ClaimAfterGuard = firstReceiver.then(beginWrite(4)).then(batchCheck).then(extClaim(4, true))
    .then(batchRename).then(batchSync).expect(s.disk.get(4) == 2 and not(gated(s, 1)) and batchSafety)
  run stage6CreateNoClobber = firstReceiver.then(beginWrite(4)).then(batchCheck).then(extWrite(4, 3)).then(batchRename)
    .expect(not(s.job.on) and s.disk.get(4) == 3 and gated(s, 1) and batchSafety)
  run stage6ForeignWriteAtSync = firstReceiver.then(beginWrite(4)).then(batchCheck).then(batchRename)
    .then(extWrite(4, 3)).then(batchSync)
    .expect(s.pages.get(4).risk and s.drafts.get(4).bytes == 2 and gated(s, 1) and batchSafety)

  // The source has no contribution to add. Its survivor still owes an own save of the same bytes.
  action admitMerge = opBatch(Map(0 -> ABSENT, 1 -> 2), Map(0 -> 1, 1 -> 2), List((1, 0)))
  run stage6UnchangedSurvivorBlocksSource = batchPrepared.then(admitMerge).then(flushDel(0).fail())
  run stage6UnchangedSurvivor = batchPrepared.then(admitMerge)
    .expect(s.pages.get(1).risk and s.drafts.get(1).bytes == 2 and s.disk.get(1) == 2 and batchSafety)
    .then(beginWrite(1)).then(batchCheck).then(batchRename)
    .expect(s.disk.get(1) == 2 and s.job.phase == 3 and gated(s, 0))
    .then(batchSync).then(saveDelete(0)).expect(s.stable.get(0) == ABSENT and s.stable.get(1) == 2 and batchSafety)
  run stage6UnchangedSurvivorStillGuarded = batchPrepared.then(admitMerge).then(extWrite(1, 3))
    .then(beginWrite(1)).then(batchCheck).expect(s.disk.get(1) == 3 and s.pages.get(1).conflict and batchSafety)
    .then(flushDel(0).fail())

  // A post-relaunch Discard has no partner. After retiring d1's draft, the R1 write changes what s1 actually deletes.
  action plannedLossPrefix = batchAdmitted.then(crash).then(launch).then(open(2)).then(discard(2))
    .expect(s.pages.get(0).buf == ABSENT and batchSafety).then(draftSync(2))
    .then(flushDel(0)).then(batchCheck).then(extWrite(0, 3)).then(batchRename)
  val plannedSourceCopy = PAGES.exists(p => copies(p).contains(1) or s.drafts.get(p).base == 1 or s.trash.get(p).contains(1))
  run stage6PlannedSourceCounterexample = plannedLossPrefix.then(batchSync).then(draftSync(0))
    .then(powerK(Set())).then(launch).expect(s.disk.get(0) == ABSENT and s.disk.get(2) == ABSENT
      and s.drafts.get(2) == NODRAFT and not(plannedSourceCopy)
      and g.removed.get(0) == Set(3) and s.trash.get(0) == Set(3) and batchSafety)
  run stage6TwoPowerCuts = plannedLossPrefix.then(powerK(Set(0))).then(powerK(Set()))
    .expect(s.disk.get(0) == ABSENT and g.removed.get(0) == Set(3) and s.trash.get(0) == Set(3) and batchSafety)
  run stage6PowerRevertsRemoval = plannedLossPrefix.then(powerK(Set()))
    .expect(s.disk.get(0) == 3 and g.removed.get(0) == Set() and s.trash.get(0) == Set() and batchSafety)

  action liveRemoval = batchAdmitted.then(saveWrite(2)).then(saveWrite(4)).then(saveWrite(1))
    .then(flushDel(0)).then(batchCheck).then(batchRename)
  val unsettled = PAGES.filter(p => s.disk.get(p) != s.stable.get(p)
    or s.trash.get(p) != s.trashStable.get(p) or g.removed.get(p) != g.delDurable.get(p))
  action selectedPower(keep: Set[int]): bool = all { keep.subseteq(unsettled), powerK(keep) }
  action queueDiscard(p: int): bool = all { s.up.length() == 0, s.pages.get(p).held,
    if (s.w.get(p).on) wDiscard(p) else wOpen(p) }
  action receiveReply(p: int): bool = all { s.w.get(p).sent, wRecv(p) }
