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
  val ok = guarantee and orderHolds and renameResolves

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

  // ================================================================ s3: page rename and delete
  // Paths: 0 holds 1, 1 holds 2 (it refers to page 0), 2 has no file (the new name).
  action saveD(p: int): bool = flushDel(p).then(publishIo(true))
  action renameAB = opRename(0, 2, Set(1), Map(1 -> 3))
  val renamed: bool = s.disk.get(0) == ABSENT and s.disk.get(2) == 1 and s.disk.get(1) == 3 and s.trash.get(0) == Set(1)
    and PAGES.forall(p => not(s.pages.get(p).held) or clean(s.pages.get(p)))
  // s3.2: in-run order dst, referrer, source deletion (restated from saveD(0) first).
  run ownRename = init.then(open(0)).then(load(1)).then(load(2)).then(renameAB).expect(s.drafts.get(2).bytes == 1 and ok)
    .then(save(2)).then(save(1)).then(saveD(0)).expect(renamed and ok)
  // From the CLI: no page is open in the window; the backend loads each page before the operation.
  run ownRenameCli = init.then(load(0)).then(load(1)).then(load(2)).then(renameAB).then(save(2)).then(save(1)).then(saveD(0)).expect(renamed and ok)
  // Interrupted after the old file went: the text and the rewrite are in drafts; launch restores them and
  // their saves complete the rename (D5). s3.2: the old file goes first only after a relaunch (no order across a
  // crash), so the restated run crashes once before the deletion's save.
  run ownRenameInterrupted = init.then(load(0)).then(load(1)).then(load(2)).then(renameAB).then(crash).then(launch).then(saveD(0)).then(draftSync(0)).then(crash).then(launch)
    .expect(s.pages.get(2).buf == 1 and s.pages.get(2).risk and s.pages.get(1).buf == 3 and s.disk.get(1) == 2 and ok)
    .then(save(2)).then(save(1)).expect(renamed and ok)
  // A rename whose old file is already gone only rewrites references (running it again after another
  // program or a lost rewrite left an old reference).
  run ownRenameRerun = init.then(load(0)).then(load(1)).then(load(2)).then(renameAB).then(save(2)).then(save(1)).then(saveD(0)).then(extWrite(1, 1)).then(observe(1))
    .then(renameAB).expect(s.pages.get(1).buf == 3 and s.pages.get(0).buf == ABSENT and ok).then(save(1)).expect(renamed and ok)
  // s3.2: as ownRenameInterrupted, the deletion's save precedes the destination's only after a relaunch.
  run ownRenamePower = init.then(load(0)).then(load(1)).then(load(2)).then(renameAB).then(crash).then(launch).then(saveD(0)).then(powerK(Set())).expect(s.drafts.get(2).bytes == 1 and ok)
    .then(launch).then(save(2)).expect(s.disk.get(2) == 1 and ok)
  run ownRenameRefusedDirtyRef = init.then(open(1)).then(load(0)).then(load(2)).then(typeIn(1, 3)).then(renameAB.fail())
  run ownRenameRefusedDirtySrc = init.then(open(0)).then(load(1)).then(load(2)).then(typeIn(0, 3)).then(renameAB.fail())
  run ownRenameRefusedTarget = init.then(extWrite(2, 3)).then(load(0)).then(load(1)).then(load(2)).then(renameAB.fail())
  run ownRenameWaitsForSave = init.then(open(1)).then(load(0)).then(load(2)).then(typeIn(1, 3)).then(flush(1)).then(renameAB.fail())
  run ownRenameAfterSave = init.then(open(1)).then(typeIn(1, 3)).then(flush(1))
    .then(publishIo(true)).then(load(0)).then(load(2)).then(renameAB).expect(s.pages.get(1).buf == 3 and ok)
  // Another program changes the old file after the rename read it: the delete is refused by the guard and
  // the page becomes a conflict, the other program's bytes on disk.
  run ownRenameExternalSrc = init.then(load(0)).then(load(1)).then(load(2)).then(renameAB).then(save(2)).then(save(1)).then(extWrite(0, 3)).then(flushDel(0)).then(check)
    .expect(s.pages.get(0).conflict and s.disk.get(0) == 3 and ok)
  // Typing on a referring page that the window had not sent yet becomes a conflict (stale), never lost.
  run ownRenameWindowTyping = init.then(open(1)).then(load(0)).then(load(2)).then(wEdit(1, 1)).then(renameAB).then(wRecv(1)).expect(s.w.get(1).text == 1 and ok)
    .then(wSend(1)).then(deliverUp(true)).expect(s.pages.get(1).conflict and s.pages.get(1).buf == 1 and ok)
  run ownDelete = init.then(open(0)).then(opDelete(0)).then(saveD(0))
    .expect(s.disk.get(0) == ABSENT and s.trash.get(0) == Set(1) and clean(s.pages.get(0)) and ok)
  run ownDeleteCli = init.then(load(1)).then(opDelete(1)).then(saveD(1)).expect(s.disk.get(1) == ABSENT and s.trash.get(1) == Set(2) and ok)
  run ownDeleteExternal = init.then(load(0)).then(opDelete(0)).then(extWrite(0, 3)).then(flushDel(0)).then(check)
    .expect(s.pages.get(0).conflict and s.disk.get(0) == 3 and ok)
  run ownDeleteRefusedDirty = init.then(open(0)).then(typeIn(0, 3)).then(opDelete(0).fail())
  run ownDeleteNothing = init.then(load(2)).then(opDelete(2).fail())
  // Mutants (each passes on the spec).
  run mMRN1 = init.then(load(0)).then(load(1)).then(load(2)).then(renameAB).then(crash).then(launch).then(saveD(0)).then(crash).expect(ok)
  run mMRN2 = init.then(open(1)).then(load(0)).then(load(2)).then(typeIn(1, 1)).then(if (opClean(s, 1) or MUTANT == "MRN2") renameAB else noop).expect(ok)
  // The target exists and the window has even shown it (so G would accept replacing it): no-clobber is its own
  // property.
  run mMRN3 = init.then(extWrite(2, 3)).then(open(2)).then(load(0)).then(load(1)).then(if (MUTANT == "MRN3") renameAB else noop).expect(ok)
  // MRN5 (src's deletion not drafted): after a crash before any save, launch must restore the deletion as a
  // pending edit of the old page; without it the old file stays beside the new one (a duplicate).
  run mMRN5 = init.then(load(0)).then(load(1)).then(load(2)).then(renameAB).then(crash).then(launch)
    .expect(s.pages.get(0).held and s.pages.get(0).buf == ABSENT and dirty(s.pages.get(0)) and ok)
  run mMTS = init.then(load(0)).then(opDelete(0)).then(saveD(0)).then(powerK(Set())).expect(ok)
  run mMRN4 = init.then(load(0)).then(load(1)).then(load(2)).then(renameAB).then(crash).expect(ok)
  run mMDT = init.then(load(0)).then(opDelete(0)).then(saveD(0)).expect(ok)
  // Review 2026-10-09 (subagent-tasks/notes/s3-review.md).
  // A crash before any save: launch restores every page of the operation, including the old page's deletion,
  // and their saves complete the rename (no duplicate).
  run ownRenameCrashBeforeSaves = init.then(load(0)).then(load(1)).then(load(2)).then(renameAB).then(crash).then(launch)
    .expect(s.pages.get(0).buf == ABSENT and s.pages.get(2).buf == 1 and s.pages.get(1).buf == 3 and ok)
    .then(saveD(0)).then(save(2)).then(save(1)).expect(renamed and ok)
  // A delete made durable keeps its trash copy across a power cut; one not yet durable is undone whole.
  run ownDeleteTrashPower = init.then(load(0)).then(opDelete(0)).then(saveD(0)).then(powerK(Set()))
    .expect((if (RACES.contains("weak")) s.disk.get(0) == 1 and s.trash.get(0) == Set()
             else s.disk.get(0) == ABSENT and s.trash.get(0) == Set(1)) and ok)
  run ownDeleteRevertedByPower = init.then(load(0)).then(opDelete(0)).then(flushDel(0)).then(check).then(rename).then(powerK(Set()))
    .expect(s.disk.get(0) == 1 and s.trash.get(0) == Set() and ok)
  // Accepted race (as every save, SPEC-s2 R1): another program creating the new name between the guard check
  // and the file step is replaced. Candidate s2.2: creates publish with no-replace where the platform has it.
  run ownRenameTargetR1 = init.then(load(0)).then(load(1)).then(load(2)).then(renameAB).then(flush(2)).then(check)
    .then(if (RACES.contains("R1")) extWrite(2, 3) else noop).then(rename).then(dirSync(true))
    .expect(s.disk.get(2) == 1 and ok)
  // s2.1: a read equal to the last read is applied after Tine's own save changed the page.
  run ownObserveAfterOwnSave = init.then(open(0)).then(typeIn(0, 2)).then(save(0))
    .then(extWrite(0, 1)).then(observe(0)).expect(s.pages.get(0).buf == 1 and ok)

  // Lean s3 round 1 counterexamples: unsent window input on a page an operation then changes; the stale
  // submit becomes a conflict over the operation's acknowledged (drafted) text. MDD: delete not drafted.
  run mMDD = init.then(wOpen(0)).then(deliverUp(true)).then(wRecv(0)).then(wEdit(0, 2))
    .then(opDelete(0)).then(wSend(0)).then(deliverUp(true)).expect(ok)
  run ownStaleAfterDelete = init.then(wOpen(0)).then(deliverUp(true)).then(wRecv(0)).then(wEdit(0, 2))
    .then(opDelete(0)).then(wSend(0)).then(deliverUp(true)).expect(ok)
  run ownStaleAfterRename = init.then(wOpen(0)).then(deliverUp(true)).then(wRecv(0)).then(wEdit(0, 2))
    .then(load(2)).then(opRename(0, 2, Set(), Map())).then(wSend(0)).then(deliverUp(true)).expect(ok)

  // s3.1: one backend read, no window. An honest durable write after load is not authority to delete it.
  run ownLoadDeleteExternal = init.expect(not(s.pages.get(0).held) and not(s.w.get(0).on) and ok)
    .then(load(0)).expect(s.pages.get(0).buf == 1 and s.pages.get(0).base == 1 and clean(s.pages.get(0)) and not(s.w.get(0).on) and ok)
    .then(extWriteD(0, 3, true)).then(opDelete(0))
    .expect(s.drafts.get(0).bytes == ABSENT and s.drafts.get(0).base == 1 and ok)
    .then(flushDel(0)).then(check)
    .expect(not(s.job.on) and s.pages.get(0).conflict and s.pages.get(0).buf == ABSENT and s.disk.get(0) == 3
      and s.trash.get(0) == Set() and s.trashStable.get(0) == Set() and g.removed.get(0) == Set() and ok)
  // Source changes after its load: move the loaded bytes; the source deletion meets a conflict, not a trash move.
  run ownLoadRenameExternalSrc = init.then(load(0)).then(load(2)).then(extWriteD(0, 3, true))
    .then(opRename(0, 2, Set(), Map())).expect(s.pages.get(2).buf == 1 and s.drafts.get(2).bytes == 1 and ok)
    .then(save(2)).expect(s.disk.get(2) == 1 and s.disk.get(0) == 3 and ok)
    .then(flushDel(0)).then(check)
    .expect(not(s.job.on) and s.pages.get(0).conflict and s.disk.get(0) == 3 and s.trash.get(0) == Set() and s.disk.get(2) == 1 and ok)
  // Referrer changes after load: its rewrite keeps the loaded base; the later guard protects the external text.
  run ownLoadRenameExternalRef = init.then(load(0)).then(load(1)).then(load(2)).then(extWriteD(1, 1, true))
    .then(renameAB).expect(s.pages.get(1).buf == 3 and s.pages.get(1).base == 2 and s.drafts.get(1).base == 2 and ok)
    .then(save(2)).then(flush(1)).then(check)
    .expect(not(s.job.on) and s.pages.get(1).conflict and s.pages.get(1).buf == 3 and s.disk.get(1) == 1 and ok)
  // Isolated held-page refusals: every other prerequisite is satisfied. MUL enables these and .fail() fails.
  run ownUnheldDelete = init.then(opDelete(0).fail())
  run ownUnheldRenameSrc = init.then(load(1)).then(load(2)).then(renameAB.fail())
  run ownUnheldRenameDst = init.then(load(0)).then(load(1)).then(renameAB.fail())
  run ownUnheldRenameRef = init.then(load(0)).then(load(2)).then(renameAB.fail())
  // In a refs-only rerun src must still be held, even though no src text changes.
  run ownUnheldRenameRefsSrc = init.then(extWrite(0, ABSENT)).then(load(1)).then(renameAB.fail())
  run ownUnheldRenameRefsRef = init.then(extWrite(0, ABSENT)).then(load(0)).then(renameAB.fail())
  // Positive controls: adding exactly the missing load enables each formerly refused action.
  run ownHeldDelete = init.then(load(0)).then(opDelete(0)).expect(s.drafts.get(0).bytes == ABSENT and ok)
  run ownHeldRenameSrc = init.then(load(1)).then(load(2)).then(load(0)).then(renameAB).expect(s.drafts.get(2).bytes == 1 and ok)
  run ownHeldRenameDst = init.then(load(0)).then(load(1)).then(load(2)).then(renameAB).expect(s.drafts.get(2).bytes == 1 and ok)
  run ownHeldRenameRef = init.then(load(0)).then(load(2)).then(load(1)).then(renameAB).expect(s.drafts.get(1).bytes == 3 and ok)
  // An unchanged dst need not be held in a refs-only rename. These controls isolate src/ref heldness.
  run ownHeldRenameRefsSrc = init.then(extWrite(0, ABSENT)).then(load(1)).then(load(0)).then(renameAB)
    .expect(s.pages.get(1).buf == 3 and not(s.pages.get(2).held) and ok)
  run ownHeldRenameRefsRef = init.then(extWrite(0, ABSENT)).then(load(0)).then(load(1)).then(renameAB)
    .expect(s.pages.get(1).buf == 3 and not(s.pages.get(2).held) and ok)
  // Durable external creation survives power keep Set(); operation's durable drafts survive independently.
  run ownLoadRenameTargetPower0 = init.then(load(0)).then(load(2)).then(opRename(0, 2, Set(), Map()))
    .then(extWriteD(2, 3, true)).then(powerK(Set())).expect(s.disk.get(2) == 3 and s.drafts.get(2).bytes == 1 and ok)
    .then(launch).expect(s.pages.get(2).buf == 1 and s.pages.get(2).base == ABSENT and s.pages.get(2).risk and ok)
    .then(flush(2)).then(check)
    .expect(not(s.job.on) and s.pages.get(2).conflict and s.pages.get(2).buf == 1 and s.disk.get(2) == 3
      and s.drafts.get(2).bytes == 1 and s.trash.get(2) == Set() and ok)
  // Durable external creation survives power keep Set(0); operation's durable drafts survive independently.
  run ownLoadRenameTargetPower1 = init.then(load(0)).then(load(2)).then(opRename(0, 2, Set(), Map()))
    .then(extWriteD(2, 3, true)).then(powerK(Set(0))).expect(s.disk.get(2) == 3 and s.drafts.get(2).bytes == 1 and ok)
    .then(launch).expect(s.pages.get(2).buf == 1 and s.pages.get(2).base == ABSENT and s.pages.get(2).risk and ok)
    .then(flush(2)).then(check)
    .expect(not(s.job.on) and s.pages.get(2).conflict and s.pages.get(2).buf == 1 and s.disk.get(2) == 3
      and s.drafts.get(2).bytes == 1 and s.trash.get(2) == Set() and ok)
  // Durable external creation survives power keep Set(1); operation's durable drafts survive independently.
  run ownLoadRenameTargetPower2 = init.then(load(0)).then(load(2)).then(opRename(0, 2, Set(), Map()))
    .then(extWriteD(2, 3, true)).then(powerK(Set(1))).expect(s.disk.get(2) == 3 and s.drafts.get(2).bytes == 1 and ok)
    .then(launch).expect(s.pages.get(2).buf == 1 and s.pages.get(2).base == ABSENT and s.pages.get(2).risk and ok)
    .then(flush(2)).then(check)
    .expect(not(s.job.on) and s.pages.get(2).conflict and s.pages.get(2).buf == 1 and s.disk.get(2) == 3
      and s.drafts.get(2).bytes == 1 and s.trash.get(2) == Set() and ok)
  // Durable external creation survives power keep Set(0, 1); operation's durable drafts survive independently.
  run ownLoadRenameTargetPower3 = init.then(load(0)).then(load(2)).then(opRename(0, 2, Set(), Map()))
    .then(extWriteD(2, 3, true)).then(powerK(Set(0, 1))).expect(s.disk.get(2) == 3 and s.drafts.get(2).bytes == 1 and ok)
    .then(launch).expect(s.pages.get(2).buf == 1 and s.pages.get(2).base == ABSENT and s.pages.get(2).risk and ok)
    .then(flush(2)).then(check)
    .expect(not(s.job.on) and s.pages.get(2).conflict and s.pages.get(2).buf == 1 and s.disk.get(2) == 3
      and s.drafts.get(2).bytes == 1 and s.trash.get(2) == Set() and ok)
  // Durable external creation survives power keep Set(2); operation's durable drafts survive independently.
  run ownLoadRenameTargetPower4 = init.then(load(0)).then(load(2)).then(opRename(0, 2, Set(), Map()))
    .then(extWriteD(2, 3, true)).then(powerK(Set(2))).expect(s.disk.get(2) == 3 and s.drafts.get(2).bytes == 1 and ok)
    .then(launch).expect(s.pages.get(2).buf == 1 and s.pages.get(2).base == ABSENT and s.pages.get(2).risk and ok)
    .then(flush(2)).then(check)
    .expect(not(s.job.on) and s.pages.get(2).conflict and s.pages.get(2).buf == 1 and s.disk.get(2) == 3
      and s.drafts.get(2).bytes == 1 and s.trash.get(2) == Set() and ok)
  // Durable external creation survives power keep Set(0, 2); operation's durable drafts survive independently.
  run ownLoadRenameTargetPower5 = init.then(load(0)).then(load(2)).then(opRename(0, 2, Set(), Map()))
    .then(extWriteD(2, 3, true)).then(powerK(Set(0, 2))).expect(s.disk.get(2) == 3 and s.drafts.get(2).bytes == 1 and ok)
    .then(launch).expect(s.pages.get(2).buf == 1 and s.pages.get(2).base == ABSENT and s.pages.get(2).risk and ok)
    .then(flush(2)).then(check)
    .expect(not(s.job.on) and s.pages.get(2).conflict and s.pages.get(2).buf == 1 and s.disk.get(2) == 3
      and s.drafts.get(2).bytes == 1 and s.trash.get(2) == Set() and ok)
  // Durable external creation survives power keep Set(1, 2); operation's durable drafts survive independently.
  run ownLoadRenameTargetPower6 = init.then(load(0)).then(load(2)).then(opRename(0, 2, Set(), Map()))
    .then(extWriteD(2, 3, true)).then(powerK(Set(1, 2))).expect(s.disk.get(2) == 3 and s.drafts.get(2).bytes == 1 and ok)
    .then(launch).expect(s.pages.get(2).buf == 1 and s.pages.get(2).base == ABSENT and s.pages.get(2).risk and ok)
    .then(flush(2)).then(check)
    .expect(not(s.job.on) and s.pages.get(2).conflict and s.pages.get(2).buf == 1 and s.disk.get(2) == 3
      and s.drafts.get(2).bytes == 1 and s.trash.get(2) == Set() and ok)
  // Durable external creation survives power keep Set(0, 1, 2); operation's durable drafts survive independently.
  run ownLoadRenameTargetPower7 = init.then(load(0)).then(load(2)).then(opRename(0, 2, Set(), Map()))
    .then(extWriteD(2, 3, true)).then(powerK(Set(0, 1, 2))).expect(s.disk.get(2) == 3 and s.drafts.get(2).bytes == 1 and ok)
    .then(launch).expect(s.pages.get(2).buf == 1 and s.pages.get(2).base == ABSENT and s.pages.get(2).risk and ok)
    .then(flush(2)).then(check)
    .expect(not(s.job.on) and s.pages.get(2).conflict and s.pages.get(2).buf == 1 and s.disk.get(2) == 3
      and s.drafts.get(2).bytes == 1 and s.trash.get(2) == Set() and ok)

  // ---- s3.2: a rename's in-run order (Martin 2026-10-10, option 2)
  action renLoaded = load(0).then(load(1)).then(load(2)).then(renameAB)
  run ownOrderSrcWaitsForDst = init.then(renLoaded).then(flushDel(0).fail())
  run ownOrderRefWaitsForDst = init.then(renLoaded).then(flush(1).fail())
  run ownOrderSrcWaitsForRef = init.then(renLoaded).then(save(2)).expect(s.disk.get(2) == 1 and ok).then(flushDel(0).fail())
  // An Uncertain or failed destination save is no witness: the referrer and the source keep waiting.
  run ownOrderUncertainDstHolds = init.then(renLoaded).then(flush(2)).then(check).then(rename).then(dirSync(false))
    .expect(s.disk.get(2) == 1 and ok).then(flush(1).fail())
  run ownOrderFailingDstKeepsSrc = init.then(renLoaded).then(flush(2)).then(saveFail)
    .expect(s.disk.get(0) == 1 and s.disk.get(2) == ABSENT and ok).then(flushDel(0).fail())
  run ownOrderCompletes = init.then(renLoaded).then(save(2)).then(save(1)).then(saveD(0))
    .expect(renamed and s.gates == List() and g.obl == Set() and ok).then(opDelete(2))
    .expect(g.renamed == Set() and ok)
  // D4 with no chaining: a witnessed page of a running order still waits for the order's end.
  run ownOrderNoChaining = init.then(renLoaded).then(save(2)).then(opDelete(2).fail())
  // Cancellation: a Discard of the unwitnessed destination reverts the source to its disk bytes and ends the order.
  run ownOrderDiscardDst = init.then(renLoaded).then(open(2)).then(discard(2))
    .expect(s.pages.get(0).buf == 1 and clean(s.pages.get(0)) and s.pages.get(2).buf == ABSENT and s.gates == List()
      and g.obl == Set() and ok)
    .then(save(1)).expect(s.disk.get(0) == 1 and s.disk.get(2) == ABSENT and s.disk.get(1) == 3 and ok)
  // ... newer input typed into the source stays, and saves.
  run ownOrderDiscardDstKeepsSrcInput = init.then(open(0)).then(load(1)).then(load(2)).then(renameAB).then(recv(0))
    .then(typeIn(0, 3)).then(open(2)).then(discard(2))
    .expect(s.pages.get(0).buf == 3 and s.gates == List() and ok).then(save(0)).expect(s.disk.get(0) == 3 and ok)
  // A discarded referrer leaves the order; the rest completes.
  run ownOrderDiscardRef = init.then(renLoaded).then(open(1)).then(discard(1)).then(save(2)).then(saveD(0))
    .expect(s.disk.get(0) == ABSENT and s.disk.get(2) == 1 and s.disk.get(1) == 2 and s.gates == List() and ok)
  // Across a crash only A and completion hold: the accepted effect (SPEC-s3 s3.2) of a failing destination write.
  run ownOrderAcceptedAcrossCrash = init.then(renLoaded).then(crash).then(launch)
    .expect(s.gates == List() and g.obl == Set() and g.renamed == Set() and ok)
    .then(flush(2)).then(saveFail).then(saveD(0))
    .expect(s.disk.get(0) == ABSENT and s.disk.get(2) == ABSENT and s.drafts.get(2).bytes == 1 and s.trash.get(0) == Set(1) and ok)
  // Mutants (each passes on the spec; each fails orderHolds with no power cut).
  run mMDF = init.then(renLoaded).then(if (MUTANT == "MDF") flushDel(0) else noop).expect(ok)
  run mMRF = init.then(renLoaded).then(if (MUTANT == "MRF") flush(1) else noop).expect(ok)
  run mMRD = init.then(renLoaded).then(save(2)).then(if (MUTANT == "MRD") flushDel(0) else noop).expect(ok)
  run mMPR = init.then(renLoaded).then(flush(2)).then(check).then(rename).then(dirSync(false))
    .then(if (MUTANT == "MPR") flush(1) else noop).expect(ok)
  run mMCV = init.then(renLoaded).then(flush(2)).then(saveFail).then(if (MUTANT == "MCV") flush(1) else noop).expect(ok)

  // ================================================================ s3.3: one host operation (HostOp)
  // Paths: 0 holds 1, 1 holds 2, 2 has no file. A feature plans on the bytes the pages hold now (cur).
  val cur: int -> int = PAGES.mapBy(p => opCur(s, p))
  // Merge page 1 into page 0: 0 receives the merged text 3, 1 is deleted (a move: 0 before 1's deletion).
  action merge = load(0).then(load(1)).then(opApply(Map(0 -> 3, 1 -> ABSENT), cur, 0, 1))
  val merged: bool = s.disk.get(0) == 3 and s.disk.get(1) == ABSENT and s.trash.get(1) == Set(2)
  run ownMerge = init.then(merge).expect(s.drafts.get(0).bytes == 3 and s.drafts.get(1).bytes == ABSENT and ok)
    .then(save(0)).then(saveD(1)).expect(merged and s.gates == List() and g.obl == Set() and ok)
  run ownMergeSrcWaits = init.then(merge).then(flushDel(1).fail())
  run ownMergeSrcWaitsUncertain = init.then(merge).then(flush(0)).then(publishIo(false)).then(flushDel(1).fail())
  // The merge crash: survivor written, source deletion pending; relaunch completes the deletion.
  run ownMergeCrash = init.then(merge).then(save(0)).then(crash).then(launch)
    .expect(s.pages.get(1).buf == ABSENT and s.pages.get(1).risk and ok).then(saveD(1)).expect(merged and ok)
  // Across a crash only A and completion: the deletion may go first after a relaunch; the survivor's draft holds it.
  run ownMergeCrashSrcFirst = init.then(merge).then(crash).then(launch).then(saveD(1))
    .expect(s.disk.get(1) == ABSENT and s.drafts.get(0).bytes == 3 and ok).then(save(0)).expect(merged and ok)
  // A Discard of the unwitnessed survivor cancels the merge: the source takes its disk bytes again.
  run ownMergeDiscardSurvivor = init.then(merge).then(open(0)).then(discard(0))
    .expect(s.pages.get(0).buf == 1 and s.pages.get(1).buf == 2 and clean(s.pages.get(1)) and s.gates == List()
      and g.obl == Set() and ok)
  // No chaining: a page of a running merge waits for the merge's end.
  run ownMergeNoChaining = init.then(merge).then(save(0)).then(opApply(Map(0 -> 1), cur, NONE, NONE).fail())
  run ownMergeRefusedDirty = init.then(open(1)).then(typeIn(1, 3)).then(load(0)).then(opApply(Map(0 -> 3, 1 -> ABSENT), cur, 0, 1).fail())
  // Create (a Guide page): expected absent.
  run ownCreate = init.then(load(2)).then(opApply(Map(2 -> 3), cur, NONE, NONE)).then(save(2)).expect(s.disk.get(2) == 3 and ok)
  run ownCreateRefusedExists = init.then(extWrite(2, 3)).then(load(2))
    .then(opApply(Map(2 -> 1), Map(0 -> 1, 1 -> 2, 2 -> ABSENT), NONE, NONE).fail())
  // A stale plan is refused (Changed): another program changed page 1 after the feature read it.
  action stalePlan = load(1).then(extWrite(1, 3)).then(observe(1))
  run ownApplyStale = init.then(stalePlan).then(opApply(Map(1 -> 1), Map(0 -> 1, 1 -> 2, 2 -> ABSENT), NONE, NONE).fail())
  // Trash, as opApply: the same as opDelete.
  run ownApplyDelete = init.then(load(0)).then(opApply(Map(0 -> ABSENT), cur, NONE, NONE)).then(saveD(0))
    .expect(s.disk.get(0) == ABSENT and s.trash.get(0) == Set(1) and ok)
  // A rename with no rewritten reference, as opApply: the destination first.
  run ownApplyRename = init.then(load(0)).then(load(2)).then(opApply(Map(2 -> 1, 0 -> ABSENT), cur, 2, 0))
    .then(flushDel(0).fail())
  run ownApplyRenameCompletes = init.then(load(0)).then(load(2)).then(opApply(Map(2 -> 1, 0 -> ABSENT), cur, 2, 0))
    .then(save(2)).then(saveD(0)).expect(s.disk.get(2) == 1 and s.disk.get(0) == ABSENT and s.gates == List() and ok)
  // Mutants (each passes on the spec).
  run mMEX = init.then(stalePlan).then(if (MUTANT == "MEX") opApply(Map(1 -> 1), Map(0 -> 1, 1 -> 2, 2 -> ABSENT), NONE, NONE) else noop)
    .expect(ok)
  run mMAC = init.then(open(1)).then(typeIn(1, 3)).then(load(0))
    .then(if (MUTANT == "MAC") opApply(Map(0 -> 3, 1 -> ABSENT), cur, 0, 1) else noop).expect(ok)
  run mMAN = init.then(merge).then(crash).expect(ok)
  run mMAM = init.then(merge).then(if (MUTANT == "MAM") flushDel(1) else noop).expect(ok)

  // ================================================================ s3.4: name-slot admission (C3)
  // This read-only transaction preflight models just its slot predicate, not parsing or a writer's policy.
  // The plan's expected bytes below are fixed at ABSENT before extClaim, independent of the held buffer.
  action planCreate(p: int): bool = all { s.disk.get(p) == ABSENT, not(s.claimed.contains(p)), noop }
  action createTwo = opApply(Map(2 -> 3), Map(2 -> ABSENT), NONE, NONE)
  // fail() is a terminal disabledness check, not a state transition. Replans therefore begin from the same
  // pre-refusal state: no version, draft, promise or operation can be recorded by a disabled opApply.
  action slotArrived = load(2).then(planCreate(2)).then(extClaim(2, true))
  run ownSlotArrivingTwin = init.then(slotArrived)
    .expect(s.disk.get(2) == ABSENT and opCur(s, 2) == ABSENT and clean(s.pages.get(2))
      and s.drafts.get(2) == NODRAFT and s.claimed.contains(2) and ok)
    .then(createTwo.fail())
  // Replan uses the transaction's same slot rule: it refuses/skips while the twin remains.
  run ownSlotReplanRefuses = init.then(slotArrived).then(planCreate(2).fail())
  run ownSlotReplanAfterTwinLeaves = init.then(slotArrived).then(extClaim(2, false)).then(planCreate(2))
    .then(createTwo).then(save(2)).expect(s.disk.get(2) == 3 and not(s.claimed.contains(2)) and ok)
  // A twin arriving on one create refuses the entire batch: the unrelated write records nothing too.
  run ownSlotBatchAtomicRefusal = init.then(load(0)).then(load(2)).then(planCreate(2)).then(extClaim(2, true))
    .expect(s.pages.get(0).buf == 1 and s.drafts.get(0) == NODRAFT and s.drafts.get(2) == NODRAFT and ok)
    .then(opApply(Map(0 -> 3, 2 -> 3), Map(0 -> 1, 2 -> ABSENT), NONE, NONE).fail())
  // A move into the claimed slot cannot record its receiver or delete its source.
  run ownSlotMoveRefusal = init.then(load(0)).then(load(2)).then(planCreate(2)).then(extClaim(2, true))
    .expect(s.disk.get(0) == 1 and s.gates == List() and g.obl == Set() and ok)
    .then(opApply(Map(0 -> ABSENT, 2 -> 1), Map(0 -> 1, 2 -> ABSENT), 2, 0).fail())
  // The guard is about absent creates, not all writes/deletions in a graph with a pre-existing collision.
  run ownSlotExistingWrite = init.then(load(0)).then(extClaim(0, true))
    .then(opApply(Map(0 -> 3), Map(0 -> 1), NONE, NONE)).then(save(0))
    .expect(s.disk.get(0) == 3 and s.claimed.contains(0) and ok)
  run ownSlotDelete = init.then(load(0)).then(extClaim(0, true))
    .then(opApply(Map(0 -> ABSENT), Map(0 -> 1), NONE, NONE)).then(saveD(0))
    .expect(s.disk.get(0) == ABSENT and s.claimed.contains(0) and ok)
  run ownSlotAbsentDelete = init.then(load(2)).then(extClaim(2, true))
    .then(opApply(Map(2 -> ABSENT), Map(2 -> ABSENT), NONE, NONE))
    .expect(s.drafts.get(2).bytes == ABSENT and ok)
  run ownSlotOtherPageClaim = init.then(load(2)).then(extClaim(0, true)).then(planCreate(2))
    .then(createTwo).then(save(2)).expect(s.disk.get(2) == 3 and s.claimed == Set(0) and ok)
  // A later external claimant is permitted: NS is admission safety, not permanent global uniqueness.
  run ownSlotClaimAfterAdmission = init.then(load(2)).then(planCreate(2)).then(createTwo)
    .then(extClaim(2, true)).then(save(2)).expect(s.disk.get(2) == 3 and s.claimed.contains(2) and ok)
  // Claimants are external disk state, not host memory. A crash/power cut must not free the slot.
  run ownSlotCrashKeepsClaim = init.then(extClaim(2, true)).then(crash).then(launch).then(load(2))
    .expect(s.claimed.contains(2) and ok).then(createTwo.fail())
  run ownSlotPowerKeepsClaim = init.then(extClaim(2, true)).then(powerK(Set())).then(launch).then(load(2))
    .expect(s.claimed.contains(2) and ok).then(createTwo.fail())
  // MNS must violate the guarantee at admission itself, before any attempt to publish.
  run mMNS = init.then(load(2)).then(planCreate(2)).then(extClaim(2, true))
    .then(if (MUTANT == "MNS") createTwo else noop).expect(ok)
  // Diagnostic witness: skipping C3 drafts and then publishes a second file for the same page name.
  // The check deliberately fails only after demonstrating that physical outcome.
  run ownSlotMutantTwoFiles = init.then(load(2)).then(planCreate(2)).then(extClaim(2, true))
    .then(if (MUTANT == "MNS") createTwo.then(save(2)) else noop)
    .expect(if (MUTANT == "MNS") s.disk.get(2) != ABSENT and s.claimed.contains(2)
      and g.bad.contains("name-slot-create-over-claim") else ok)
    .expect(ok)

  // ---- s3.5: cancelling Discard restores only a held source.
  // Literal round-9 defect trace; retirement resets the entire source to NOPAGE (including ver=0).
  action heldGuardRetired = renLoaded.then(open(0)).then(discard(0)).then(wClose(0)).then(deliverUp(true))
  val heldGuardRetiredOk = s.pages.get(0) == NOPAGE and not(s.pages.get(0).held)
    and s.pages.get(0).buf == ABSENT and s.pages.get(0).ver == NOPAGE.ver
  run ownHeldGuardRetired = init.then(heldGuardRetired)
    .expect(heldGuardRetiredOk and not(g.promise.get(0).on) and ok)
    .then(open(2)).then(discard(2))
    .expect(heldGuardRetiredOk and s.pages.get(2).buf == ABSENT
      and s.gates == List() and g.obl == Set() and ok)
  // Identical user outcome assertion: MRH must fail with QNT508, not a disabled action.
  run mMRH = init.then(heldGuardRetired)
    .expect(heldGuardRetiredOk and not(g.promise.get(0).on) and ok)
    .then(open(2)).then(discard(2))
    .expect(heldGuardRetiredOk and s.pages.get(2).buf == ABSENT
      and s.gates == List() and g.obl == Set() and ok)
  // Positive control: a held buffer still containing the operation's deletion takes its disk bytes.
  run ownHeldGuardRestores = init.then(renLoaded)
    .expect(s.pages.get(0).held and s.pages.get(0).buf == ABSENT and s.pages.get(0).ver > 0 and ok)
    .then(open(2)).then(discard(2))
    .expect(s.pages.get(0).held and s.pages.get(0).buf == s.disk.get(0) and s.pages.get(0).buf == 1
      and s.pages.get(0).ver == g.vc and clean(s.pages.get(0))
      and s.pages.get(2).buf == ABSENT and s.gates == List() and g.obl == Set() and ok)
