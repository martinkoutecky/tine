  // ================================================================ directed scenarios (not part of the spec)
  action noop = all { s' = s, g' = g }
  action open(p: int): bool = wOpen(p).then(deliverUp(true)).then(wRecv(p))
  // type: the user types v, the window sends it, the backend applies it, the window reads the answer
  action typeIn(p: int, v: int): bool = wEdit(p, v).then(wSend(p)).then(deliverUp(true)).then(wRecv(p))
  action publishIo(ok: bool): bool = check.then(rename).then(dirSync(ok))
  action save(p: int): bool = flush(p).then(publishIo(true))
  action discard(p: int): bool = wDiscard(p).then(deliverUp(true)).then(wRecv(p))
  action recv(p: int): bool = if (s.mb.get(p).on) wRecv(p) else noop
  action drafts = (if (s.drafts.get(0) != draftEntry(s.pages.get(0))) draftSync(0) else noop)
    .then(if (s.drafts.get(1) != draftEntry(s.pages.get(1))) draftSync(1) else noop)
  action move = open(0).then(open(1)).then(wOp(0, 3, 3)).then(deliverUp(true)).then(wRecv(0)).then(wRecv(1))
  val ok = guarantee

  // ---- maximality (§2 table)
  run maxLoad = init.then(open(0)).then(extWrite(0, 3)).then(observe(0))
    .expect(s.pages.get(0).buf == 3 and clean(s.pages.get(0)) and ok)
  run maxLoadShown = maxLoad.then(wRecv(0)).expect(s.w.get(0).text == 3 and ok)
  run maxDeletedClean = init.then(open(0)).then(extWrite(0, ABSENT)).then(observe(0))
    .expect(s.pages.get(0).buf == ABSENT and ok)
  run maxSave = init.then(open(0)).then(typeIn(0, 2)).then(save(0))
    .expect(s.disk.get(0) == 2 and clean(s.pages.get(0)) and ok)
  // Equal bytes from another program advance the base but are not Tine's publication.
  run maxEqual = init.then(open(0)).then(typeIn(0, 2)).then(extWrite(0, 2)).then(observe(0))
    .expect(not(clean(s.pages.get(0))) and s.pages.get(0).base == 2 and not(s.pages.get(0).conflict) and ok)
  run maxEqualThenSave = maxEqual.then(save(0)).expect(clean(s.pages.get(0)) and s.disk.get(0) == 2 and ok)
  run qEqualThenExternal = maxEqual.then(extWrite(0, 3)).then(observe(0))
    .expect(s.pages.get(0).buf == 2 and s.pages.get(0).conflict and ok)
  run maxDeleted = init.then(open(0)).then(typeIn(0, 2)).then(extWrite(0, ABSENT)).then(observe(0))
    .expect(s.pages.get(0).conflict and s.pages.get(0).risk and ok)
  run qTypeDuringSave = init.then(open(0)).then(typeIn(0, 2)).then(flush(0)).then(wEdit(0, 3)).then(wSend(0))
    .then(check).then(rename).then(dirSync(true)).then(recv(0)).then(deliverUp(true)).then(wRecv(0))
    .expect(s.pages.get(0).buf == 3 and s.pages.get(0).base == 2 and s.w.get(0).text == 3 and ok)

  // ---- the window protocol
  // A change loads while the user types: the typed text reaches the backend as a conflict, never saved over
  // the change until the user has seen it.
  run qStaleSubmit = init.then(open(0)).then(wEdit(0, 2)).then(extWrite(0, 3)).then(observe(0)).then(wSend(0))
    .then(deliverUp(true)).then(wRecv(0))
    .expect(s.pages.get(0).buf == 2 and s.pages.get(0).conflict and s.w.get(0).text == 2 and s.w.get(0).conf and ok)
  run qStaleResolve = qStaleSubmit.then(wResolve(0, 2)).then(deliverUp(true)).then(wRecv(0)).then(save(0))
    .expect(s.disk.get(0) == 2 and ok)
  run qStaleExternalReverts = qStaleSubmit.then(extWrite(0, 1)).then(observe(0))
    .expect(s.pages.get(0).conflict and ok)
  // A push does not replace unsent input; the answer to a request does not either.
  run qPushKeepsTyping = init.then(open(0)).then(wEdit(0, 2)).then(extWrite(0, 3)).then(observe(0)).then(wRecv(0))
    .expect(s.w.get(0).text == 2 and s.w.get(0).pend and ok)
  run qAckKeepsTyping = init.then(open(0)).then(wEdit(0, 2)).then(wSend(0)).then(wEdit(0, 3)).then(deliverUp(true))
    .then(wRecv(0)).expect(s.w.get(0).text == 3 and s.w.get(0).pend and ok)
  run qAckKeepsTypingThenSend = qAckKeepsTyping.then(wSend(0)).then(deliverUp(true)).then(wRecv(0))
    .expect(s.pages.get(0).buf == 3 and not(s.pages.get(0).conflict) and ok)
  // A window reload keeps what it had sent; the new window sees it.
  run qWindowCrashKeepsSent = init.then(open(0)).then(wEdit(0, 2)).then(wSend(0)).then(windowCrash).then(deliverUp(true))
    .then(open(0)).expect(s.pages.get(0).buf == 2 and s.w.get(0).text == 2 and ok)
  run qWindowCrashLosesUnsent = init.then(open(0)).then(wEdit(0, 2)).then(windowCrash).expect(ok)
  run qWindowCrashThenType = init.then(open(0)).then(wEdit(0, 2)).then(wSend(0)).then(windowCrash).then(wOpen(0))
    .then(deliverUp(true)).then(deliverUp(true)).then(wRecv(0)).then(typeIn(0, 3))
    .expect(s.pages.get(0).buf == 3 and not(s.pages.get(0).conflict) and ok)
  run qCloseDirtyKeeps = init.then(open(0)).then(typeIn(0, 2)).then(wClose(0)).then(deliverUp(true))
    .expect(s.pages.get(0).held and ok)
  run qCloseClean = init.then(open(0)).then(typeIn(0, 2)).then(save(0)).then(wClose(0)).then(deliverUp(true))
    .expect(not(s.pages.get(0).held) and ok)

  // ---- the hard cases (each must hold in the spec)
  run qExternalAfterSave = init.then(open(0)).then(typeIn(0, 2)).then(save(0)).then(extWrite(0, 3)).then(crash).expect(ok)
  run qDeleteBeforeReply = init.then(open(0)).then(typeIn(0, 2)).then(flush(0)).then(check).then(rename)
    .then(extWrite(0, ABSENT)).then(dirSync(true)).then(crash).expect(not(s.alive) and ok)
  run qExternalBetweenRenameAndReply = init.then(open(0)).then(typeIn(0, 2)).then(flush(0)).then(check)
    .then(rename).then(extWrite(0, 1)).then(dirSync(true)).then(crash).expect(ok)
  run qUncertainDiscardPower = init.then(open(0)).then(typeIn(0, 2)).then(flush(0)).then(publishIo(false))
    .then(drafts).then(discard(0)).then(drafts).then(power(false, true)).expect(ok)
  run qCrashForgetsUncertain = init.then(open(0)).then(typeIn(0, 2)).then(flush(0)).then(publishIo(false))
    .then(drafts).then(crash).then(launch).then(open(0)).then(discard(0)).then(drafts).then(power(false, false)).expect(ok)
  run qFailedSaveDraftCrash = init.then(open(0)).then(typeIn(0, 2)).then(flush(0)).then(saveFail).then(drafts)
    .then(crash).then(launch).expect(s.pages.get(0).buf == 2 and ok)
  run qRetypeOldDraft = init.then(open(0)).then(typeIn(0, 2)).then(flush(0)).then(saveFail).then(drafts)
    .then(typeIn(0, 3)).then(typeIn(0, 2)).then(crash).then(launch).expect(ok and s.pages.get(0).buf == 2)
  run qDiscardSurvivesSwitch = init.then(open(0)).then(typeIn(0, 2)).then(switchReq).then(drafts).then(discard(0))
    .then(drafts).then(switchFin).then(launch)
    .expect(ok and not(s.pages.get(0).held) and s.drafts.get(0).bytes == NONE)
  run qNoSwitchWithStaleDraft = init.then(open(0)).then(typeIn(0, 2)).then(switchReq).then(drafts).then(discard(0))
    .expect(ok and not(canSwitch))
  run qCloseAfterExternal = init.then(open(0)).then(typeIn(0, 2)).then(flush(0)).then(saveFail).then(drafts)
    .then(extWrite(0, 3)).then(observe(0)).then(drafts).then(switchFin)
    .then(launch).expect(ok and s.pages.get(0).buf == 2)
  run qRestoredUnknownBase = qStaleSubmit.then(drafts).then(crash).then(launch).then(flush(0)).then(check)
    .expect(s.pages.get(0).conflict and ok)

  // ---- moves (the receiver's text is drafted when the move is applied; no ordering of saves)
  run qMoveDrafted = init.then(move).expect(s.drafts.get(1).bytes == 3 and s.pages.get(1).risk and ok)
  run qMoveSrcFirst = init.then(move).then(save(0)).then(crash).then(launch)
    .expect(ok and s.disk.get(0) == 3 and s.pages.get(1).buf == 3)
  run qMoveDstFirst = init.then(move).then(save(1)).then(save(0))
    .expect(s.disk.get(0) == 3 and s.disk.get(1) == 3 and ok)
  run qMoveCrashBeforeSaves = init.then(move).then(crash).then(launch).expect(ok and s.disk.get(0) == 1 and s.pages.get(1).buf == 3)
  run qMoveCrashBetween = init.then(move).then(save(1)).then(crash).then(launch)
    .expect(ok and s.disk.get(0) == 1 and s.disk.get(1) == 3)
  run qMovePowerAfterSrc = init.then(move).then(save(0)).then(power(false, false)).then(launch).expect(ok)
  run qMoveSwitch = init.then(move).then(switchReq).then(drafts).then(switchFin)
    .then(launch).expect(ok and s.pages.get(0).buf == 3 and s.pages.get(1).buf == 3)
  run qMoveRefused = init.then(open(0)).then(open(1)).then(wOp(0, 3, 3)).then(deliverUp(false)).then(wRecv(0)).then(wRecv(1))
    .expect(ok and s.pages.get(1).buf == 2 and s.w.get(1).text == 2 and s.w.get(0).text == 1 and s.drafts.get(1).bytes == NONE)
  run qMoveDstConflict = init.then(move).then(extWrite(1, 1)).then(flush(1)).then(check).then(drafts).then(crash)
    .then(launch).expect(ok and s.pages.get(1).buf == 3)
  run qMoveDiscardDst = init.then(move).then(discard(1)).then(drafts).then(crash).expect(ok)
  // Fable F1: a move on a stale version is refused whole; both windows get the backend's texts back.
  run qMoveStale = init.then(open(0)).then(open(1)).then(extWrite(1, 1)).then(observe(1)).then(wOp(0, 3, 3))
    .then(deliverUp(true)).expect(not(s.pages.get(1).conflict) and s.pages.get(0).buf == 1 and s.pages.get(1).buf == 1
      and s.drafts.get(1).bytes == NONE and ok)
    .then(wRecv(0)).then(wRecv(1)).expect(s.w.get(0).text == 1 and s.w.get(1).text == 1 and not(s.w.get(0).sent) and ok)

  // ---- read errors, reloads
  run qOpenFail = init.then(wOpen(0)).then(deliverUp(false)).then(wRecv(0))
    .expect(ok and not(s.w.get(0).on) and not(s.w.get(0).sent) and not(s.pages.get(0).held))
  run qOpenFailRetry = qOpenFail.then(open(0)).expect(ok and s.w.get(0).on and s.pages.get(0).held)
  run qDiscardReadFails = init.then(open(0)).then(typeIn(0, 2)).then(wDiscard(0)).then(deliverUp(false)).then(wRecv(0))
    .expect(ok and s.pages.get(0).buf == 2 and s.w.get(0).text == 2)
  run qStaleRestoredUnseen = qStaleSubmit.then(drafts).then(crash).then(launch).expect(ok and s.pages.get(0).base == UNKNOWN)
  run qWindowCrashQueuedMove = init.then(open(0)).then(open(1)).then(wOp(0, 3, 3)).then(windowCrash).then(deliverUp(true))
    .then(crash).then(launch).expect(ok and s.pages.get(1).buf == 3)

  // ---- one targeted trace per mutant (passes on the spec; the named mutant fails it)
  run mMC = init.then(open(0)).then(typeIn(0, 2)).then(flush(0)).then(extWrite(0, 3)).then(check)
    .then(if (s.job.on) rename else noop).expect(ok)
  run mMC1 = mMC
  run mMQ = init.then(open(0)).then(typeIn(0, 2)).then(flush(0)).then(publishIo(false)).then(observe(0))
    .then(power(false, false)).expect(ok)
  run mMLS = mMQ
  run mMRE = mMQ
  run mMLO = init.then(open(0)).then(typeIn(0, 2)).then(extWrite(0, 3)).then(observe(0)).expect(ok)
  run mMCL = init.then(open(0)).then(typeIn(0, 2)).then(wClose(0)).then(deliverUp(true)).expect(ok)
  run mMX = init.then(open(0)).then(typeIn(0, 2)).then(switchReq).then(if (canSwitch) switchFin else noop).expect(ok)
  // The restored draft survives a discard showing an earlier non-durable save of the same bytes (MDE).
  run qCrashForgetsUncertainCold = init.then(open(0)).then(typeIn(0, 2)).then(flush(0)).then(publishIo(false))
    .then(drafts).then(crash).then(launch).then(open(0)).then(discard(0)).then(drafts).then(power(false, false)).expect(ok)
  run mMST = init.then(open(0)).then(wEdit(0, 2)).then(extWrite(0, 3)).then(observe(0)).then(wSend(0)).then(deliverUp(true))
    .then(if (s.pages.get(0).conflict) noop else save(0)).expect(ok)
  run mMWA = init.then(open(0)).then(wEdit(0, 2)).then(extWrite(0, 3)).then(observe(0)).then(wRecv(0)).expect(ok)
  run mMOD = init.then(move).then(crash).expect(ok)
  run mMIS = init.then(open(0)).then(typeIn(0, 2)).expect(ok)
  run mMDQ = init.then(open(0)).then(wEdit(0, 2)).then(wSend(0)).then(windowCrash).expect(ok)
  run mMRB = qStaleSubmit.then(drafts).then(crash).then(launch).then(flush(0)).then(check)
    .then(if (s.job.on) rename else noop).expect(ok)
  run mMAK = init.then(open(0)).then(wEdit(0, 2)).then(wSend(0)).then(extWrite(0, 3)).then(observe(0)).then(wRecv(0)).expect(ok)
  run mMDO = init.then(open(0)).then(open(1)).then(typeIn(0, 2)).then(typeIn(1, 3)).expect(ok)
  run mMBD = mMRB
  // G across restarts does not depend on what the draft store records as the base.
  run qStaleCleanReleases = qStaleSubmit.then(discard(0)).then(typeIn(0, 2)).then(save(0)).expect(ok)
  run qStaleNoDraftReleases = qStaleSubmit.then(crash).then(launch).then(open(0)).then(typeIn(0, 2)).then(save(0))
    .expect(ok and s.disk.get(0) == 2)
  // Inductive lane (10/07): equal bytes from another program discharge the stale text (base case), and the
  // R1 race then overwrites like any save (accepted). Both were G false alarms with two outside writes.
  run qStaleEqualThenEdit = qStaleSubmit.then(extWrite(0, 2)).then(observe(0)).then(wRecv(0))
    .then(typeIn(0, 3)).then(save(0)).expect(ok and s.disk.get(0) == 3)
  // TLC R1 4,1,1 (10/08): a saved text is saved again unchanged; another program deletes the file between
  // the check and the rename (R1); the rename is Uncertain; a discard then shows Tine's own not-durable bytes.
  // A power cut leaves the other program's deletion, which the earlier saved promise accepts (Q12).
  // Reduction lane TLC 4,1,2 (10/08), found on the pre-G-release model: equal bytes from a second outside write.
  run qG412EqualAfterDelete = init.then(wOpen(0)).then(deliverUp(true)).then(wRecv(0)).then(extWrite(0, ABSENT))
    .then(observe(0)).then(wEdit(0, 2)).then(wSend(0)).then(wEdit(0, 1)).then(deliverUp(false)).then(wRecv(0))
    .then(wSend(0)).then(extWrite(0, 2)).then(observe(0)).then(deliverUp(false)).then(flush(0)).then(check)
    .then(rename).expect(ok)
  // Induction round 2 + cutoff lane (10/08), on the flag-based G: an old draft restored after a crash, while
  // the flag belonged to a newer undrafted stale text; the file was changed back to the draft's base.
  run qOldDraftAfterStaleCrash = init.then(wOpen(0)).then(deliverUp(true)).then(wRecv(0)).then(wEdit(0, 2)).then(wSend(0))
    .then(deliverUp(true)).then(wRecv(0)).then(flush(0)).then(saveFail).then(draftSync(0)).then(flush(0)).then(check)
    .then(rename).then(dirSync(true)).then(wEdit(0, 1)).then(extWrite(0, 3)).then(observe(0)).then(wSend(0))
    .then(deliverUp(true)).then(wRecv(0)).then(crash).then(extWrite(0, 1)).then(launch).then(flush(0)).then(check)
    .then(rename).expect(ok)
  // Byte-based G still flags a save over a change the user never saw: the window never shows 3.
  run qGNeverShown = init.then(open(0)).then(wEdit(0, 2)).then(extWrite(0, 3)).then(observe(0)).then(wSend(0))
    .then(deliverUp(true)).expect(ok and s.pages.get(0).conflict)
  // TLC base 4,1,1 on byte-based G (10/08): the user types while a reload from disk is on its way; another
  // program deletes the file. The typed text stays on the old version, so it becomes a conflict, never a save
  // over the deletion the user has not seen.
  run qTypeDuringDiscard = init.then(open(0)).then(wDiscard(0)).then(wEdit(0, 3)).then(extWrite(0, ABSENT))
    .then(deliverUp(true)).then(wRecv(0)).then(wSend(0)).then(deliverUp(true))
    .expect(ok and s.pages.get(0).conflict and s.pages.get(0).base == UNKNOWN)
  run mMDA = init.then(open(0)).then(wDiscard(0)).then(wEdit(0, 3)).then(extWrite(0, ABSENT))
    .then(deliverUp(true)).then(wRecv(0)).then(wSend(0)).then(deliverUp(true))
    .then(if (s.pages.get(0).conflict) noop else flush(0)).then(if (s.job.on) check else noop)
    .then(if (s.job.on) rename else noop).expect(ok)
  // ind3 (10/08), G: text typed during a move the backend refused was typed on the window's text, not on the
  // backend's: it becomes a conflict, not a save over external bytes never shown.
  run qTypeDuringRefusedMove = init.then(open(0)).then(open(1))
    .then(extWrite(1, 3)).then(observe(1)).then(wOp(0, 3, 1)).then(wEdit(1, 2))
    .then(deliverUp(false)).then(wRecv(1)).then(wSend(1)).then(deliverUp(true))
    .then(if (s.pages.get(1).conflict) noop else flush(1)).then(if (s.job.on) check else noop)
    .then(if (s.job.on) rename else noop).expect(ok and s.disk.get(1) == 3)
  // TLC base/weak 4,1,1 (10/08): the same on the move's source page, whose file another program deleted.
  run qTypeDuringRefusedMoveSrc = init.then(open(0)).then(open(1)).then(wOp(0, 2, 1)).then(wEdit(0, 1))
    .then(extWrite(0, ABSENT)).then(observe(0)).then(deliverUp(false)).then(wRecv(0)).then(wSend(0))
    .then(deliverUp(false)).then(if (s.pages.get(0).conflict) noop else flush(0)).then(if (s.job.on) check else noop)
    .then(if (s.job.on) rename else noop).expect(ok and s.disk.get(0) == ABSENT)
  // TLC base 4,1,1 (10/08): text typed during a discard of an Uncertain save arrives stale; it replaces the
  // reloaded buffer, whose bytes the user had been shown and typed over, after another program deleted the file.
  run qStaleOverShownBuffer = init.then(open(0)).then(wEdit(0, 2)).then(wSend(0)).then(deliverUp(false))
    .then(flush(0)).then(wRecv(0)).then(wDiscard(0)).then(wEdit(0, 1)).then(check).then(rename)
    .then(dirSync(false)).then(deliverUp(true)).then(wRecv(0)).then(wSend(0)).then(extWrite(0, ABSENT))
    .then(observe(0)).then(deliverUp(false)).expect(ok and s.pages.get(0).buf == 1 and s.pages.get(0).conflict)
  // ind4 (10/08), A: a rename whose directory sync never ran survives a crash; the draft of text typed after
  // relaunch was deleted when a discard showed the same bytes, and a power cut lost it. The discard now keeps
  // the draft until Tine has saved those bytes itself (MDE: it does not).
  run qBareRenameDraftDiscarded = init.then(open(0)).then(typeIn(0, 2)).then(flush(0)).then(check).then(rename)
    .then(crash).then(launch).then(open(0)).then(typeIn(0, 3)).then(typeIn(0, 2)).then(switchReq).then(draftSync(0))
    .then(discard(0)).then(draftSync(0)).then(power(false, true)).expect(ok and (s.disk.get(0) == 2 or s.drafts.get(0).bytes == 2))
  // cutoff4 (10/08), A: a discard during an Uncertain save claims no durability for the old promise.
  run qDiscardDuringUncertainKeepsPromise = init.then(open(0)).then(typeIn(0, 2)).then(save(0)).then(typeIn(0, 1))
    .then(flush(0)).then(check).then(rename).then(dirSync(false)).then(power(true, false)).then(launch)
    .then(open(0)).then(typeIn(0, 2)).then(flush(0)).then(check).then(rename).then(dirSync(false))
    .then(discard(0)).then(power(false, false)).expect(ok)
  // ind4 (10/08), B: the same class as qStaleOverShownBuffer (an Uncertain save reloaded, typed over, stale).
  run qStaleOverShownBuffer2 = init.then(open(0)).then(typeIn(0, 2)).then(flush(0)).then(check).then(rename)
    .then(dirSync(false)).then(wDiscard(0)).then(wEdit(0, 3)).then(deliverUp(true)).then(wRecv(0))
    .then(extWrite(0, 3)).then(observe(0)).then(wSend(0)).then(deliverUp(true)).expect(ok)
  // ind5 (10/08), B: the reload loads external bytes 3 the user never sees (typing pending), the other program
  // then replaces them with 2, and the stale typing replaces the buffer's 3. Those bytes held no user input
  // and their author superseded them: not a loss (B's stale clause accepts an untyped buffer).
  run qStaleOverSupersededRead = init.then(open(0)).then(typeIn(0, 2)).then(flush(0)).then(check).then(rename)
    .then(dirSync(false)).then(extWrite(0, 3)).then(wDiscard(0)).then(wEdit(0, 1)).then(deliverUp(true))
    .then(wRecv(0)).then(extWrite(0, 2)).then(observe(0)).then(wSend(0)).then(deliverUp(true)).expect(ok)
  // ind3 (10/08), A: a discard does not revive a saved promise an external write had ended; the not-durable
  // retyped save a crash interrupted was never promised.
  run qDiscardAfterExemptCrash = init.then(open(0)).then(typeIn(0, 2)).then(save(0))
    .then(extWrite(0, 3)).then(observe(0)).then(wRecv(0)).then(typeIn(0, 2))
    .then(flush(0)).then(check).then(rename).then(crash).then(launch).then(open(0))
    .then(discard(0)).then(power(false, true)).expect(ok)
  run qR1DeleteUncertainDiscard = init.then(open(0)).then(typeIn(0, 2)).then(save(0)).then(typeIn(0, 1)).then(typeIn(0, 2))
    .then(wDiscard(0)).then(flush(0)).then(check).then(if (RACES.contains("R1")) extWrite(0, ABSENT) else noop)
    .then(rename).then(dirSync(false)).then(deliverUp(true)).then(power(false, false)).expect(ok)
  run qStaleEqualThenR1 = qStaleSubmit.then(extWrite(0, 2)).then(observe(0))
    .then(flush(0)).then(check).then(if (RACES.contains("R1")) extWrite(0, 3) else noop).then(rename).expect(ok)
  // cutoff4 (model 9a8f67e1) noLoss witness: a saved copy whose rename was never dir-synced, then power. Fixed by launch dir-sync.
  run qSavedWatermark = init.expect(guarantee).then(wOpen(0)).expect(guarantee).then(deliverUp(true)).expect(guarantee).then(wRecv(0)).expect(guarantee).then(wEdit(0,2)).expect(guarantee).then(wSend(0)).expect(guarantee).then(deliverUp(true)).expect(guarantee).then(wRecv(0)).expect(guarantee).then(flush(0)).expect(guarantee).then(check).expect(guarantee).then(rename).expect(guarantee).then(dirSync(true)).expect(guarantee).then(wEdit(0,1)).expect(guarantee).then(wSend(0)).expect(guarantee).then(deliverUp(true)).expect(guarantee).then(wRecv(0)).expect(guarantee).then(flush(0)).expect(guarantee).then(check).expect(guarantee).then(rename).expect(guarantee).then(dirSync(false)).expect(guarantee).then(power(true, false)).expect(guarantee).then(launch).expect(guarantee).then(wOpen(0)).expect(guarantee).then(deliverUp(true)).expect(guarantee).then(wRecv(0)).expect(guarantee).then(wEdit(0,2)).expect(guarantee).then(wSend(0)).expect(guarantee).then(deliverUp(true)).expect(guarantee).then(wRecv(0)).expect(guarantee).then(flush(0)).expect(guarantee).then(check).expect(guarantee).then(rename).expect(guarantee).then(dirSync(false)).expect(guarantee).then(wDiscard(0)).expect(guarantee).then(deliverUp(true)).expect(guarantee).then(wRecv(0)).expect(guarantee)
    .then(power(false, false))
    .expect(guarantee)

  // Astra 2026-10-08 finding 1: another program writes the drafted bytes without fsync; a discard shows them.
  // The page stays at risk and Tine saves them itself before the draft goes (MDE: the draft goes at once).
  run mMDE = init.then(open(0)).then(typeIn(0, 2)).then(switchReq).then(drafts).then(extWriteD(0, 2, false))
    .then(discard(0)).expect(s.pages.get(0).risk and ok).then(drafts).then(power(false, true)).expect(ok)
  // After the power cut reverted the other program's write, the restored text meets the disk as a conflict.
  run qDiscardOverVolatileRestored = mMDE.then(launch).expect(s.pages.get(0).buf == 2 and s.pages.get(0).risk and ok)
    .then(observe(0)).expect(s.pages.get(0).conflict and s.pages.get(0).buf == 2 and ok)
  // Without the power cut, Tine's own save of the equal bytes releases the draft.
  run qDiscardOverVolatileResaves = init.then(open(0)).then(typeIn(0, 2)).then(switchReq).then(drafts)
    .then(extWriteD(0, 2, false)).then(discard(0)).then(flush(0)).then(publishIo(true)).then(drafts)
    .then(power(false, true)).expect((RACES.contains("weak") or s.disk.get(0) == 2) and s.drafts.get(0).bytes == NONE and ok)
  // Launch's directory sync does not make another program's unflushed write durable.
  // No churn: discarding unsaved typing over Tine's own Published text needs no resave.
  run qDiscardOverPublishedNoResave = init.then(open(0)).then(typeIn(0, 2)).then(save(0)).then(typeIn(0, 3))
    .then(discard(0)).expect(clean(s.pages.get(0)) and s.pages.get(0).buf == 2 and ok)
