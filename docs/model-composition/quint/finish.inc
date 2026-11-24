  // One admitted request, added at the published storage model's scenario hook.
  pure val before = Map(0 -> 1, 1 -> 2, 2 -> ABSENT)
  pure val patch = Map(0 -> ABSENT, 1 -> 3, 2 -> 1)
  pure val footprint = patch.keys()
  type Request = { id: int, outcome: str, published: Set[int] }
  var request: Request

  action start = all { init, request' = { id: 0, outcome: "NotAdmitted", published: Set() } }
  action prepare = start.then(keep(load(0))).then(keep(load(2)))
    .then(keep(wOpen(1))).then(keep(deliverUp(true))).then(keep(wRecv(1)))
  action admit = all {
    opApply(patch, before, 2, 0),
    request' = { id: 1, outcome: "Pending", published: Set() },
  }
  action admitted = prepare.then(admit)
  val pending = request.id == 1 and Set("Pending", "Cancelling").contains(request.outcome)
  val exactImage = footprint.forall(p => s.stable.get(p) == patch.get(p) and s.disk.get(p) == patch.get(p))
  val noPrematureSuccess = request.outcome != "Applied" or (request.published == footprint and exactImage)
  val stillOwed = footprint.exclude(request.published)
  val retained = stillOwed.forall(p => s.drafts.get(p).bytes == patch.get(p))
  val slotBlocked = stillOwed.contains(2) and s.claimed.contains(2) and s.pages.get(2).base == ABSENT
  val staleBuffer = stillOwed.exists(p => s.pages.get(p).base == UNKNOWN)
  val changedBase = stillOwed.exists(p => {
    val pg = s.pages.get(p)
    (pg.conflict and pg.obs == s.disk.get(p))
      // Storage can answer Published after a foreign overwrite between rename and sync.
      // Its clean buffer then has no corresponding durable publication for this request.
      or (pg.held and clean(pg) and pg.buf == patch.get(p))
  })
    or request.published.exists(p => s.disk.get(p) != patch.get(p) or s.stable.get(p) != patch.get(p))

  action keep(a: bool): bool = all { a, request' = request }
  action report(outcome: str): bool = all { noop, request' = { ...request, outcome: outcome } }
  action idle = keep(noop)
  action stopHost = all { pending, keep(crash) }
  action cutPower(keepPaths: Set[int]): bool = all { pending, keep(powerK(keepPaths)) }
  action failWrite = all { pending, keep(saveFail) }
  action failSync = all { pending, keep(dirSync(false)) }
  action changeClaim(present: bool): bool = all { pending, keep(extClaim(2, present)) }
  action foreignWrite(p: int, bytes: int, durable: bool): bool = all { pending, keep(extWriteD(p, bytes, durable)) }

  // The old window still has the pre-admission referrer version.
  action editOldWindow = all { pending, s.w.get(1).bv != s.pages.get(1).ver, keep(wEdit(1, 1)) }
  action sendOldWindow = all { pending, keep(wSend(1)) }
  action deliverOldWindow = all { pending, s.up.length() > 0, s.up.head().kind == "submit",
    s.up.head().bv != s.pages.get(1).ver, keep(deliverUp(true)) }
  // Window traffic may get ahead of the request observer. Exercise the ordinary draft writer on this page.
  action syncStaleDraft = all { pending, s.pages.get(1).base == UNKNOWN, keep(draftSync(1)) }

  // Study the existing cancellation of the still-unwitnessed live destination order.
  action queueOpenDst = all { pending, partner(s, 2) == 0, keep(wOpen(2)) }
  action queueDiscardDst = all { pending, partner(s, 2) == 0, keep(wDiscard(2)) }
  action takeOpenDst = all { pending, s.up.length() > 0, s.up.head().kind == "open", keep(deliverUp(true)) }
  action recvDst = all { pending, keep(wRecv(2)) }
  action cancel = all {
    request.outcome == "Pending", partner(s, 2) == 0, not(request.published.contains(2)), s.disk.get(2) == ABSENT,
    s.up.length() > 0, s.up.head().kind == "discard",
    deliverUp(true), request' = { ...request, outcome: "Cancelling" },
  }
  def retired(p: int): bool = s.drafts.get(p) == NODRAFT
  def canRetire(p: int): bool = not(retired(p)) and draftEntry(s.pages.get(p)) == NODRAFT
  action settleCancellation =
    if (canRetire(2)) keep(draftSync(2))
    else if (canRetire(0)) keep(draftSync(0))
    else if (retired(2) and retired(0) and s.disk.get(2) == ABSENT and s.stable.get(2) == ABSENT
      and (s.drafts.get(1).bytes == patch.get(1) or request.published.contains(1))) report("Cancelled")
    else idle

  // A service choice is made with the host up and no job. The chosen storage action
  // still checks its own guard. Only the admitted contents are eligible for this driver.
  def saveCandidate(p: int): bool = {
    val pg = s.pages.get(p)
    stillOwed.contains(p) and pg.held and pg.buf == patch.get(p) and pg.base != UNKNOWN
      and not(pg.conflict) and dirty(pg) and not(gated(s, p)) and not(p == 2 and slotBlocked)
  }
  def readCandidate(p: int): bool = {
    val pg = s.pages.get(p)
    stillOwed.contains(p) and pg.held and table(pg, s.disk.get(p), g.vc + 1) != pg
  }
  action service =
    if (saveCandidate(2)) keep(flush(2))
    else if (saveCandidate(1)) keep(flush(1))
    else if (saveCandidate(0)) keep(flushDel(0))
    else if (readCandidate(2)) keep(observe(2))
    else if (readCandidate(1)) keep(observe(1))
    else if (readCandidate(0)) keep(observe(0))
    else idle

  action goodSync = {
    val p = s.job.p
    all {
      dirSync(true),
      request' = { ...request, published:
        if (s.job.bytes == patch.get(p) and s.disk.get(p) == patch.get(p))
        request.published.union(Set(p)) else request.published },
    }
  }
  action drive(policy: str): bool = all {
    pending,
    if (not(s.alive)) keep(launch)
    else if (policy == "early" and request.published != Set()) report("Applied")
    else if (s.job.on) {
      if (s.job.phase == 1) keep(check)
      else if (s.job.phase == 2) keep(rename)
      else goodSync
    }
    else if (request.outcome == "Cancelling") settleCancellation
    else if (request.published == footprint and exactImage) report("Applied")
    else if (policy != "retryOnly" and retained and slotBlocked) report("NameClaimed")
    else if (policy != "retryOnly" and retained and staleBuffer) report("StaleInput")
    else if (policy != "retryOnly" and retained and changedBase) report("BaseChanged")
    else service,
  }
  action poll = drive("normal")
  action pollRetryOnly = drive("retryOnly")
  action pollEarly = drive("early")
  action pollSkip = all { pending, idle }

  action fourPolls = poll.then(poll).then(poll).then(poll)
  action finish = fourPolls.then(fourPolls).then(fourPolls).then(poll)
  action toRename = admitted.then(poll).then(poll).then(poll)
  action interrupted = toRename.then(stopHost)
  action noResume = admitted.then(stopHost)
  action repeated = noResume.then(poll).then(stopHost).then(poll).then(stopHost).then(poll)
  action claimed = admitted.then(changeClaim(true))
  action stale = admitted.then(editOldWindow).then(sendOldWindow).then(deliverOldWindow)
  action overwritten = stale.then(syncStaleDraft)
  action cancellable = admitted.then(queueOpenDst).then(takeOpenDst).then(recvDst).then(queueDiscardDst)
  action discardDelivered = cancellable.then(cancel)
  action interruptedCancel = discardDelivered.then(stopHost)
  action preparedConflict = admitted.then(foreignWrite(2, 2, true))
  action syncRaced = toRename.then(foreignWrite(2, 2, true))
  action writerRound = poll.then(foreignWrite(2, 2, true)).then(poll)
    .then(foreignWrite(2, ABSENT, true)).then(poll)
  action afterWriterRound = admitted.then(writerRound)
  val backAtDestination = pending and not(s.job.on) and not(s.pages.get(2).conflict)
    and s.disk.get(2) == ABSENT and s.drafts.get(2).bytes == 1 and request.published == Set()

  run finishUncontended = admitted.then(finish)
    .expect(request.outcome == "Applied" and noPrematureSuccess and ok)
  run finishCrashBeforeSync = interrupted
    .expect(s.disk.get(2) == 1 and s.stable.get(2) == ABSENT and request.published == Set())
    .then(poll).then(poll).then(poll).then(finish)
    .expect(request.outcome == "Applied" and noPrematureSuccess and ok)
  run finishPowerBeforeSync = toRename.then(cutPower(Set()))
    .expect(s.disk.get(2) == ABSENT and s.drafts.get(2).bytes == 1 and retained)
    .then(poll).then(finish)
    .expect(request.outcome == "Applied" and noPrematureSuccess and ok)
  run finishRepeatedLaunch = repeated
    .expect(pending and retained and request.published == Set() and ok)
    .then(finish).expect(request.outcome == "Applied" and noPrematureSuccess and ok)
  run finishClaimed = claimed.expect(retained).then(poll)
    .expect(request.outcome == "NameClaimed" and retained and ok).then(keep(flush(2)).fail())
  run finishDiskConflict = preparedConflict.then(poll).then(poll).then(poll)
    .expect(request.outcome == "BaseChanged" and retained and ok)
  run finishSyncRaced = syncRaced.then(poll)
    .expect(pending and request.published == Set() and s.disk.get(2) == 2 and s.stable.get(2) == 2)
    .then(poll).expect(request.outcome == "BaseChanged" and retained and ok)
  run finishWriterKeepsPreempting = afterWriterRound
    .expect(backAtDestination and g.ext.get(2) == 2 and ok)
    .then(writerRound).expect(backAtDestination and g.ext.get(2) == 4 and ok)
  run finishStale = stale.expect(staleBuffer and retained and ok)
    .then(poll).expect(request.outcome == "StaleInput" and retained and ok)
  run finishStaleOverwritten = overwritten.then(fourPolls).then(poll).then(poll)
    .expect(pending and not(retained) and s.drafts.get(1).bytes == 1
      and s.stable.get(1) == 2 and s.pages.get(1).base == UNKNOWN and gated(s, 0) and ok)
  run finishStaleReportDoesNotProtectDraft = stale.then(poll)
    .expect(request.outcome == "StaleInput" and retained)
    .then(keep(draftSync(1))).then(keep(crash)).then(keep(launch))
    .expect(not(retained) and s.drafts.get(1).bytes == 1 and s.stable.get(1) == 2 and ok)
  run finishStalePlanRefused = prepare.then(keep(extWrite(1, 1))).then(keep(observe(1)))
    .expect(request.id == 0 and ok).then(admit.fail())
  run finishCancelled = discardDelivered
    .expect(request.outcome == "Cancelling" and s.gates == List() and s.disk.get(2) == ABSENT
      and s.pages.get(0).buf == 1 and not(retired(0)) and not(retired(2)) and ok)
    .then(poll).then(poll).then(poll)
    .expect(request.outcome == "Cancelled" and retired(0) and retired(2) and ok)
    .then(keep(flush(1))).then(keep(check)).then(keep(rename)).then(keep(dirSync(true)))
    .expect(request.outcome == "Cancelled" and s.disk.get(1) == 3 and s.disk.get(0) == 1
      and s.disk.get(2) == ABSENT and noPrematureSuccess and ok)
  run finishCancelInterrupted = interruptedCancel.then(poll)
    .expect(request.outcome == "Cancelling" and s.pages.get(2).buf == 1
      and s.pages.get(0).buf == ABSENT and not(canRetire(2)) and not(canRetire(0)) and ok)
    .then(poll).expect(request.outcome == "Cancelling" and ok)

  action sampleStep = any {
    poll, idle, stopHost, failWrite, failSync, changeClaim(true), changeClaim(false),
    editOldWindow, sendOldWindow, deliverOldWindow, syncStaleDraft,
    queueOpenDst, takeOpenDst, recvDst, queueDiscardDst, cancel,
    nondet keepPaths = footprint.powerset().oneOf() cutPower(keepPaths),
    nondet p = footprint.oneOf()
      nondet bytes = TEXTS.union(Set(ABSENT)).oneOf()
      nondet durable = BOOLS.oneOf() foreignWrite(p, bytes, durable),
  }
  val finishSafety = noPrematureSuccess and ok

  // Run only on the five-slot instance. Each opApply still has one designated pair.
  pure val secondPatch = Map(3 -> ABSENT, 1 -> 1, 4 -> 2)
  pure val secondBefore = Map(3 -> 2, 1 -> 3, 4 -> ABSENT)
  action secondMove = opApply(secondPatch, secondBefore, 4, 3)
  action firstPublishesReferrer = init.then(load(0)).then(load(1)).then(load(2))
    .then(opApply(patch, before, 2, 0)).then(save(2)).then(save(1))
    .then(extWrite(3, 2)).then(load(3)).then(load(4))
  run multiSharedReferrerRefused = firstPublishesReferrer
    .expect(clean(s.pages.get(1)) and touches(s, secondPatch.keys()) and ok).then(secondMove.fail())
  run multiSharedReferrerSequential = firstPublishesReferrer
    .then(saveD(0)).then(secondMove).then(save(4)).then(save(1)).then(saveD(3))
    .expect(s.stable.get(0) == ABSENT and s.stable.get(3) == ABSENT and s.stable.get(2) == 1
      and s.stable.get(4) == 2 and s.stable.get(1) == 1 and ok)
