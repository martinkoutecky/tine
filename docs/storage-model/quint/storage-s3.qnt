// og storage, model s3 (2026-10-09): s2 + page rename (with reference rewrite) and page delete.
// s3 is a conservative extension of s2.1 = s2 + the observe fix below (Martin 2026-10-09: "behaves exactly the same outside those added
// operations"). Changes to s2's text, each the identity on runs without the new operations:
// - PAGES may hold more than two paths; paths 0 and 1 start as in s2, any further path starts absent.
//   power, wOp and launch's version numbering are written for any number of paths (same transitions on two).
// - rename (the save's file step) moves the bytes it removes to the trash, and records them in g.removed, when
//   the save writes "no file"; dirSync makes the trash durable with the path; power reverts it with the path.
//   G also accepts bytes a page operation computed its text from (g.opRead). A save writes "no file" and
//   g.opRead is non-empty only after a new operation.
// New: state trash, trashStable; ghost opRead, removed, delDurable; actions opRename, opDelete, flushDel;
// tags D4-unclean-page-changed, rename-clobbered-target; guarantee conjunct trashed. Prose: SPEC-s3.md.
// Decisions D4-D6 (Martin 2026-10-09, defaults accepted): a page an operation would change must be clean
// (the client saves first; until then the operation is refused); no intent record (every page the operation
// changes is drafted, in one file, so launch completes an interrupted operation); delete moves the file to
// the graph trash.
//
// --- s2 header follows.
// og storage, simple model s2 (2026-10-07). Candidate to replace storage.qnt (round 2, storage-r2.qnt).
// s1 + the Sol/high review (subagent-tasks/notes/s1-review.md) + the epoch quotient from the reduction lane.
//
// Design (Martin 2026-10-07, OS1/OS2):
// - The backend owns every page's state and does all file I/O. One page is never touched by two backend
//   steps at once: a save holds the page from start to answer, and the window's requests wait for it.
// - The window is an editor client. It sends its text with the backend version it was typed on, and shows
//   what the backend pushes. A text typed on a version the backend has moved past (a change loaded while
//   the user typed) becomes a conflict with an unknown base: never saved until the user resolves it.
// - A multi-file operation (here: a move of text from one page to another) is ordinary edits of its pages,
//   with no operation record. Applying it first writes the receiving page's new text to the draft store; if
//   that write fails the move is refused. So the moved text is durable before anything else happens, and an
//   interrupted move leaves a duplicate, never a loss, on a filesystem that keeps an fsynced rename. On a
//   weak filesystem (R3) a power cut may revert the receiver's acknowledged save after its draft is gone,
//   and the moved text is lost with it, like any acknowledged save there. An interrupted operation is not
//   completed or undone (accepted by Martin 2026-10-07, as in original Logseq).
// - Implementation obligations the model assumes (checked by conformance tests, not here):
//   a page's reads and the application of their result are serialized, and an older read never applies
//   after a newer one; draft writes and removals for a page are serialized, carry the version they write, and
//   one older than what the store holds is never applied; a move holds both pages from its draft write to
//   their update; the backend takes a window request (it is in the request queue) before the window counts
//   it as sent; every answer carries its request's identity and window generation, and an answer for an
//   earlier window or another request is dropped; an orderly switch drains the window, the backend and every
//   in-flight draft write before it closes.
//
// Texts are small integers; equal integers are equal bytes.
module storage {
  // ================================================================ parameters
  pure val PAGES = Set(0, 1, 2)
  pure val TEXTS = Set(1, 2, 3)
  pure val ABSENT = -1          // no file at the path
  pure val NONE = -2            // no draft / no value
  pure val UNKNOWN = -3         // base of a text typed on a version the backend has moved past
  // Environment capabilities (accepted races and faults). The driver rewrites this line per profile.
  pure val RACES: Set[str] = Set("crash", "power")
  // Deliberately broken variants, to show the properties catch losses. "none" is the spec.
  pure val MUTANT: str = "none"
  // SWEEP-MUTS: {'MRN1': 'base', 'MRN2': 'base', 'MRN3': 'base', 'MRN4': 'base', 'MRN5': 'base', 'MTS': 'base', 'MDT': 'base', 'MC': 'base', 'MC1': 'R1', 'MQ': 'base', 'MLS': 'base', 'MLO': 'base', 'MCL': 'base', 'MX': 'base', 'MST': 'base', 'MWA': 'base', 'MOD': 'base', 'MIS': 'base', 'MDQ': 'base', 'MRB': 'base', 'MRE': 'base', 'MAK': 'base', 'MDO': 'base', 'MBD': 'base', 'MDA': 'base', 'MDE': 'base', 'MDD': 'base'}
  // SWEEP-CONTROL: {}
  // TLC bounds (tlc2.py rewrites these three lines).
  pure val VMAX: int = 1000
  pure val EMAX: int = 1000
  pure val QMAX: int = 1000

  // ================================================================ state
  type Page = {
    held: bool,                  // the backend holds a buffer for the path
    buf: int, base: int,         // buffer bytes; the disk bytes the buffer is based on (UNKNOWN: see above)
    ver: int,                    // buffer version: changes on every buffer change
    risk: bool,                  // at risk: has (or is getting) a draft
    conflict: bool,
    typed: bool,                 // the buffer holds input Tine has not itself published
    obs: int                     // the disk bytes last read
  }
  type Draft = { bytes: int, base: int, ver: int }
  type Job = { on: bool, p: int, bytes: int, base: int, ver: int, phase: int, ep: int }
  // The window's copy of a page: its text, the backend version that text was typed on, unsent input,
  // a request awaiting its answer, and the conflict shown (with the disk bytes shown).
  type W = { on: bool, text: int, bv: int, pend: bool, sent: bool, obs: int, conf: bool }
  // The backend's newest state for the window, per page (a newer push replaces an unread one).
  // ack: the version the window's request produced (-1: not an answer). held false: the page could not open.
  // took: at version ack the page's buffer is the text the window's request carried (an applied submit or move).
  type Mail = { on: bool, held: bool, ver: int, text: int, obs: int, conf: bool, ack: int, took: bool }
  // A window request, in order. op: p = src, q = dst, t/t2 their new texts, bv/bv2 their versions.
  // submit: ro != NONE resolves a conflict against the disk bytes ro that the user was shown.
  // cur: sent by the current window (a reload marks every queued request as from an earlier window, whose
  // answers are dropped; only this equality is ever observed, so no epoch counter is needed).
  type Up = { kind: str, p: int, q: int, t: int, t2: int, bv: int, bv2: int, ro: int, cur: bool }
  type Sys = {
    alive: bool,
    disk: int -> int,            // what a read sees now
    stable: int -> int,          // what a power cut cannot revert
    drafts: int -> Draft,        // durable (app data, strong filesystem)
    trash: int -> Set[int],      // s3: per path, the bytes a Tine delete moved to the graph trash
    trashStable: int -> Set[int], // s3: the part of trash a power cut cannot revert (as stable for paths)
    pages: int -> Page, job: Job,
    w: int -> W, mb: int -> Mail, up: List[Up]
  }
  var s: Sys

  // ---------------------------------------------------------------- ghost (the guarantee's bookkeeping)
  type Promise = { on: bool, bytes: int, ver: int, saved: bool, ep: int }
  type Ghost = {
    vc: int,                              // version counter (never reset)
    promise: int -> Promise,              // per page: the newest acknowledged text
    ext: int -> int,                      // per path: number of external writes so far
    wrote: int -> Set[(int, int)],        // per page: (bytes, version) Tine put at the path or in a draft
    owed: Set[(int, int)],                // (page, text) the window sent and the backend has not yet taken
    guard: bool,                          // the running save's guard check found the base on disk
    seen: int -> Set[int],                // per page, every file content the window has shown the user (G)
    mine: Set[int],                       // pages the current user request may replace (transient)
    opRead: int -> Set[int],              // s3: per page, file bytes a page operation computed its new text from
    removed: int -> Set[int],             // s3: per path, bytes a Tine delete removed from it
    delDurable: int -> Set[int],          // s3: the part of removed whose delete a directory sync made durable
    bad: Set[str]                         // violated transition properties, recorded by the step
  }
  var g: Ghost

  // ================================================================ constants and helpers
  pure val NOPAGE: Page = { held: false, buf: ABSENT, base: ABSENT, ver: 0, risk: false, conflict: false,
    typed: false, obs: ABSENT }
  pure val NODRAFT: Draft = { bytes: NONE, base: NONE, ver: 0 }
  pure val NOJOB: Job = { on: false, p: 0, bytes: 0, base: 0, ver: 0, phase: 0, ep: 0 }
  pure val NOW: W = { on: false, text: ABSENT, bv: -1, pend: false, sent: false, obs: ABSENT, conf: false }
  pure val NOMAIL: Mail = { on: false, held: false, ver: 0, text: ABSENT, obs: ABSENT, conf: false, ack: -1, took: false }
  pure val NOPROMISE: Promise = { on: false, bytes: NONE, ver: 0, saved: false, ep: 0 }

  pure def clean(pg: Page): bool = not(pg.typed) and pg.buf == pg.base and not(pg.risk)
  pure def dirty(pg: Page): bool = not(clean(pg))
  // The draft store mirrors the backend: a page has a draft exactly when it is at risk.
  pure def draftEntry(pg: Page): Draft = if (pg.held and pg.risk) { bytes: pg.buf, base: pg.base, ver: pg.ver } else NODRAFT

  // §2: apply a read of the path (bytes v) to a page. nv is the version a load gets.
  pure def table(pg: Page, v: int, nv: int): Page =
    val q = { ...pg, obs: v }
    if (MUTANT == "MQ" and v == q.buf) { ...q, base: v, conflict: false, risk: false, typed: false }
    else if (v == q.base) { ...q, conflict: false }
    else if (clean(q) or (MUTANT == "MLO" and not(q.risk))) { ...q, buf: v, base: v, ver: nv, conflict: false, typed: false }
    // Equal bytes advance the base but publish nothing: the input is still Tine's to save (an external change
    // wins only over text Tine itself published by a guarded rename).
    else if (v == q.buf) { ...q, base: v, conflict: false }
    else { ...q, conflict: true, risk: true }

  // A save's outcome. The page was held throughout, so the outcome is about the buffer as it is. The backend
  // does not read the path here; its next read (observe) reconciles.
  pure def onReply(pg: Page, out: str, bytes: int): Page =
    if (out == "Published") { ...pg, base: bytes, typed: false, risk: false }
    else if (out == "Uncertain" and MUTANT == "MLS") { ...pg, base: bytes, typed: false, risk: false }
    else { ...pg, risk: true }

  // A window submit applied to a page: on the version it was typed on, an edit; otherwise a conflict whose
  // base is unknown (MST: applied as an edit anyway). MIS: the submit is answered but not applied.
  pure def submitTo(pg: Page, t: int, bv: int, v: int): Page =
    if (MUTANT == "MIS") pg
    else if (bv == pg.ver or MUTANT == "MST") { ...pg, buf: t, typed: true, ver: v }
    else { ...pg, buf: t, base: UNKNOWN, typed: true, ver: v, conflict: true, risk: true }

  // A promise moves only forward in version; an ended promise keeps its version as a watermark.
  pure def ack(pr: Promise, bytes: int, ver: int, saved: bool, ep: int): Promise =
    if (ver > pr.ver or (ver == pr.ver and pr.on and saved and not(pr.saved)))
      { on: true, bytes: bytes, ver: ver, saved: saved, ep: ep }
    else pr
  pure def endPromise(pr: Promise, watermark: int): Promise = { ...pr, on: false, ver: watermark }

  def down(sys: Sys): Sys =
    { ...sys, alive: false, pages: PAGES.mapBy(_ => NOPAGE), job: NOJOB,
      w: PAGES.mapBy(_ => NOW), mb: PAGES.mapBy(_ => NOMAIL), up: List() }
  def gDown(gh: Ghost): Ghost = { ...gh, owed: Set(), guard: false }
  def locked(p: int): bool = s.job.on and s.job.p == p
  // The answer to a window request goes to the window that sent it; an earlier window's answer is dropped.
  pure def ackMail(sys: Sys, p: int, ok: bool, v: int, took: bool): Sys =
    if (not(ok)) sys
    else { val pg = sys.pages.get(p)
      { ...sys, mb: sys.mb.set(p, { on: true, held: pg.held, ver: pg.ver, text: pg.buf, obs: pg.obs, conf: pg.conflict, ack: v, took: took }) } }
  pure def up1(kind: str, p: int, t: int, bv: int, ro: int): Up =
    { kind: kind, p: p, q: 0, t: t, t2: 0, bv: bv, bv2: 0, ro: ro, cur: true }
  // (page, text) pairs a request carries for the backend to take.
  pure def carries(u: Up): Set[(int, int)] =
    if (u.kind == "submit") Set((u.p, u.t)) else if (u.kind == "op") Set((u.p, u.t), (u.q, u.t2)) else Set()

  // Every step goes through here. It pushes each changed page to the window that shows it, and keeps the
  // ghost. Ghost rule: when a step other than a user action turns a dirty page clean, Tine shows the user its
  // text as saved, so the buffer text becomes the page's promise. On the spec only a Published save does that
  // (with its own saved promise); any other way is not a guarded rename by Tine (saved = false).
  action commit(nsys: Sys, ng: Ghost, name: str): bool = {
    val np = PAGES.mapBy(p => {
      val a = s.pages.get(p)
      val b = nsys.pages.get(p)
      if (not(ng.mine.contains(p)) and a.held and dirty(a) and b.held and clean(b) and b.buf == a.buf)
        ack(ng.promise.get(p), b.buf, b.ver, false, 0)
      else ng.promise.get(p) })
    val mb = PAGES.mapBy(p => {
      val a = s.pages.get(p)
      val b = nsys.pages.get(p)
      val m = nsys.mb.get(p)
      val wp = nsys.w.get(p)
      if (nsys.alive and (wp.on or wp.sent) and (a.ver != b.ver or a.buf != b.buf or a.obs != b.obs
          or a.conflict != b.conflict or a.held != b.held))
        { on: true, held: b.held, ver: b.ver, text: b.buf, obs: b.obs, conf: b.conflict, ack: if (m.on) m.ack else -1, took: m.on and m.took }
      else m })
    all {
      s' = { ...nsys, mb: mb },
      // G: a window's text (received or typed) is seen. A conflict's file bytes are seen only once the user
      // resolves against them (wResolve): shown in the conflict view, they are not yet decided on.
      g' = { ...ng, promise: np, mine: Set(), seen: PAGES.mapBy(p => { val w = nsys.w.get(p)
               ng.seen.get(p).union(Set(w.text).filter(x => w.on and (x == ABSENT or TEXTS.contains(x)))) }),
             bad: ng.bad.union(if (PAGES.forall(p => keptB(p, nsys, np, ng.mine, name))) Set() else Set("B-unsaved-replaced"))
                        .union(if (PAGES.forall(p => keptW(p, nsys, ng.owed, name))) Set() else Set("B'-window-replaced")) },
    }
  }

  // ================================================================ init
  pure val initDisk: int -> int = PAGES.mapBy(p => if (p == 0) 1 else if (p == 1) 2 else ABSENT)
  action init = all {
    s' = { alive: true, disk: initDisk, stable: initDisk,
      drafts: PAGES.mapBy(_ => NODRAFT), trash: PAGES.mapBy(_ => Set()), trashStable: PAGES.mapBy(_ => Set()), pages: PAGES.mapBy(_ => NOPAGE), job: NOJOB,
      w: PAGES.mapBy(_ => NOW), mb: PAGES.mapBy(_ => NOMAIL), up: List() },
    g' = { vc: 0, promise: PAGES.mapBy(_ => NOPROMISE), ext: PAGES.mapBy(_ => 0),
      wrote: PAGES.mapBy(_ => Set()), owed: Set(), guard: false,
      seen: PAGES.mapBy(_ => Set()), mine: Set(), opRead: PAGES.mapBy(_ => Set()), removed: PAGES.mapBy(_ => Set()), delDurable: PAGES.mapBy(_ => Set()), bad: Set() },
  }

  // ================================================================ the window (user actions and requests)
  action wOpen(p: int): bool = { val a = s.w.get(p)
    all { s.alive, not(a.on), not(a.sent),
      commit({ ...s, w: s.w.set(p, { ...NOW, sent: true }), up: s.up.append(up1("open", p, 0, 0, NONE)) }, g, "wOpen") } }

  action wEdit(p: int, v: int): bool = { val a = s.w.get(p)
    all { s.alive, a.on, v != a.text, commit({ ...s, w: s.w.set(p, { ...a, text: v, pend: true }) }, g, "wEdit") } }

  // Unsent input goes to the backend, one request per page at a time (debounced in the code).
  action wSend(p: int): bool = { val a = s.w.get(p)
    all { s.alive, a.on, a.pend, not(a.sent),
      commit({ ...s, w: s.w.set(p, { ...a, pend: false, sent: true }), up: s.up.append(up1("submit", p, a.text, a.bv, NONE)) },
        { ...g, owed: g.owed.union(Set((p, a.text))) }, "wSend") } }

  // Conflict resolution: the user approves bytes v against the disk bytes they were shown (keep mine: v = text).
  action wResolve(p: int, v: int): bool = { val a = s.w.get(p)
    all { s.alive, a.on, a.conf, not(a.sent),
      commit({ ...s, w: s.w.set(p, { ...a, text: v, pend: false, sent: true }), up: s.up.append(up1("submit", p, v, a.bv, a.obs)) },
        { ...g, owed: g.owed.union(Set((p, v))), seen: g.seen.set(p, g.seen.get(p).union(Set(a.obs).filter(x => x == ABSENT or TEXTS.contains(x)))) }, "wResolve") } }

  // Discard / reload from disk: the user drops what the page holds.
  action wDiscard(p: int): bool = { val a = s.w.get(p)
    all { s.alive, a.on, not(a.sent),
      commit({ ...s, w: s.w.set(p, { ...a, pend: false, sent: true }), up: s.up.append(up1("discard", p, 0, a.bv, NONE)) }, g, "wDiscard") } }

  // A move of text from src to dst, as one request: both new texts with the versions they were made from.
  action wOp(src: int, dS: int, dD: int): bool = wOpTo(src, 1 - src, dS, dD)
  action wOpTo(src: int, dst: int, dS: int, dD: int): bool = {
    val a = s.w.get(src)
    val b = s.w.get(dst)
    all { s.alive, src != dst, a.on, b.on, not(a.pend), not(a.sent), not(b.pend), not(b.sent), dS != a.text, dD != b.text,
      commit({ ...s, w: s.w.set(src, { ...a, text: dS, sent: true }).set(dst, { ...b, text: dD, sent: true }),
        up: s.up.append({ kind: "op", p: src, q: dst, t: dS, t2: dD, bv: a.bv, bv2: b.bv, ro: NONE, cur: true }) },
        { ...g, owed: g.owed.union(Set((src, dS), (dst, dD))) }, "wOp") } }

  action wClose(p: int): bool = { val a = s.w.get(p)
    all { s.alive, a.on, not(a.pend), not(a.sent),
      commit({ ...s, w: s.w.set(p, NOW), up: s.up.append(up1("close", p, 0, 0, NONE)) }, g, "wClose") } }

  // The window reads its mail. An answer to its request: the request is done; the shown text is replaced
  // only if nothing was typed since (an open that failed leaves the page closed, with an error). A push:
  // shown only when nothing is typed or on its way (MWA: also over unsent input).
  action wRecv(p: int): bool = { val m = s.mb.get(p); val a = s.w.get(p)
    all { s.alive, m.on,
      val isAck = (m.ack >= 0 or MUTANT == "MAK") and a.sent
      val na =
        if (isAck and not(m.held)) NOW
        // Text typed while a request was on its way was typed on the window's text. It moves to the request's
        // version only if the backend took that text (an applied submit or move); after a discard or a refused
        // move it keeps its version, and so becomes a conflict if sent. MDA: it moves to the version regardless.
        else if (isAck and a.pend) { ...a, sent: false, bv: if (m.took or MUTANT == "MDA") m.ack else a.bv, obs: m.obs, conf: m.conf }
        else if (isAck) { on: true, text: m.text, bv: m.ver, pend: false, sent: false, obs: m.obs, conf: m.conf }
        else if (a.on and not(a.sent) and (not(a.pend) or MUTANT == "MWA")) { ...a, text: m.text, bv: m.ver, obs: m.obs, conf: m.conf }
        else if (a.on) { ...a, obs: m.obs, conf: m.conf }
        else a
      commit({ ...s, w: s.w.set(p, na), mb: s.mb.set(p, NOMAIL) }, g, if (isAck) "wAck" else "wRecv") } }

  // ================================================================ the backend: window requests, in order
  // An open reads the file; a read can fail (permissions, I/O error): the open is refused visibly.
  action upOpen(sys: Sys, m: Up, ok: bool, rok: bool): bool = {
    val pg = sys.pages.get(m.p)
    val d = s.disk.get(m.p)
    val v1 = g.vc + 1
    val np = if (pg.held or not(rok)) pg else { ...NOPAGE, held: true, buf: d, base: d, obs: d, ver: v1 }
    commit(ackMail({ ...sys, pages: sys.pages.set(m.p, np) }, m.p, ok, np.ver, false),
      if (np.ver == v1) { ...g, vc: v1 } else g, if (pg.held or rok) "uOpen" else "uOpenFail") }

  // §6.3: a clean page whose view closes is retired (MCL: also a dirty page not at risk).
  action upClose(sys: Sys, m: Up): bool = {
    val pg = sys.pages.get(m.p)
    val go = pg.held and (clean(pg) or (MUTANT == "MCL" and not(pg.risk)))
    commit(if (go) { ...sys, pages: sys.pages.set(m.p, NOPAGE) } else sys, g, "uClose") }

  // Discard: the buffer takes the disk bytes (a failed read refuses it visibly).
  action upDiscard(sys: Sys, m: Up, ok: bool, rok: bool): bool = {
    val pg = sys.pages.get(m.p)
    val d = s.disk.get(m.p)
    val v1 = g.vc + 1
    val pr = g.promise.get(m.p)
    // A discard claims no durability: Tine cannot tell whether the bytes it reads are durable (its own
    // Uncertain or interrupted save, or another program's write that a power cut may revert). So a page at
    // risk whose buffer or draft holds the very bytes it reads stays at risk, and Tine saves them itself before
    // the draft goes (MDE: it does not). A discard showing a live promise's bytes leaves that promise as it is;
    // any other promise ends: the user chose the disk's bytes. (A promise an external write already ended,
    // which A exempts, stays ended.)
    val hold = pg.risk and (d == pg.buf or d == s.drafts.get(m.p).bytes) and MUTANT != "MDE"
    val nw = { ...pg, buf: d, base: d, ver: v1, obs: d, conflict: false, risk: hold, typed: false }
    val live = pr.on and not(pr.saved and g.ext.get(m.p) > pr.ep)
    val npr = if (live and pr.bytes == d) pr else endPromise(pr, v1)
    if (not(pg.held)) commit(sys, { ...g, bad: g.bad.union(Set("request-to-unheld")) }, "uDiscard")
    else if (not(rok)) commit(ackMail(sys, m.p, ok, pg.ver, false), g, "uDiscardFail")
    else commit(ackMail({ ...sys, pages: sys.pages.set(m.p, nw) }, m.p, ok, v1, false),
      { ...g, vc: v1, promise: g.promise.set(m.p, npr), mine: Set(m.p) }, "uDiscard") }

  // B'(2): a submitted text becomes the page's buffer.
  action upSubmit(sys: Sys, m: Up, ok: bool): bool = {
    val pg = sys.pages.get(m.p)
    val v1 = g.vc + 1
    val resolve = m.ro != NONE
    val nw = if (resolve and MUTANT != "MIS") { ...pg, buf: m.t, base: m.ro, typed: true, ver: v1, conflict: pg.obs != m.ro, risk: pg.risk or pg.obs != m.ro }
             else submitTo(pg, m.t, m.bv, v1)
    val stale = not(resolve) and m.bv != pg.ver
    val taken = if (nw.buf == m.t) Set() else Set("B'-submit-not-taken")
    // MDO: the request also drops the other page's buffer.
    val pages = if (MUTANT == "MDO") PAGES.mapBy(q => if (q == m.p) nw else NOPAGE) else sys.pages.set(m.p, nw)
    if (not(pg.held)) commit(sys, { ...g, bad: g.bad.union(Set("request-to-unheld")) }, "uSubmit")
    else commit(ackMail({ ...sys, pages: pages }, m.p, ok, v1, nw.buf == m.t),
      { ...g, vc: v1, mine: if (stale) Set() else Set(m.p), owed: g.owed.exclude(Set((m.p, m.t))), bad: g.bad.union(taken),
             },
      if (resolve) "uResolve" else if (stale) "uStale" else "uSubmit") }

  // A move: the receiving page's new text is written to the draft store first (dok: that write succeeded),
  // then both pages take their new texts, each like a submit. The receiver is at risk until it is saved, so
  // its draft stays. If the draft write fails, the move is refused and nothing changes. MOD: the move goes
  // ahead without the receiver's draft.
  // A move made on a version the backend has moved past (either page) is refused whole, like a failed draft
  // write: half-applying it would leave the moved text only as the receiver's conflict (Fable F1). MSM: the
  // stale page becomes a conflict and the other is applied.
  // The ghost acknowledges the move when it is applied: the receiver's new text is promised from then on.
  action upOp(sys: Sys, m: Up, ok: bool, dok: bool): bool = {
    val src = m.p
    val dst = m.q
    val a = sys.pages.get(src)
    val b = sys.pages.get(dst)
    val vD = g.vc + 1
    val vS = g.vc + 2
    val nb = { ...submitTo(b, m.t2, m.bv2, vD), risk: true }
    val na = submitTo(a, m.t, m.bv, vS)
    val stale = Set(src, dst).filter(p => if (p == src) m.bv != a.ver else m.bv2 != b.ver)
    val drafted = if (MUTANT == "MOD") sys.drafts else sys.drafts.set(dst, draftEntry(nb))
    val owed = g.owed.exclude(Set((src, m.t), (dst, m.t2)))
    if (not(a.held) or not(b.held)) commit(sys, { ...g, bad: g.bad.union(Set("request-to-unheld")) }, "uOp")
    else if ((not(dok) and MUTANT != "MOD") or (stale != Set() and MUTANT != "MSM"))
      commit(ackMail(ackMail(sys, dst, ok, b.ver, false), src, ok, a.ver, false), { ...g, owed: owed }, "uOpRefused")
    else commit(ackMail(ackMail({ ...sys, drafts: drafted, pages: sys.pages.set(dst, nb).set(src, na) }, dst, ok, vD, nb.buf == m.t2), src, ok, vS, na.buf == m.t),
      { ...g, vc: vS, owed: owed, mine: Set(src, dst).exclude(stale),
              wrote: g.wrote.set(dst, g.wrote.get(dst).union(Set((m.t2, vD)))),
              promise: g.promise.set(dst, ack(g.promise.get(dst), m.t2, vD, false, 0)),
              bad: g.bad.union(if (nb.buf == m.t2 and na.buf == m.t) Set() else Set("B'-submit-not-taken")) },
      if (stale == Set()) "uOp" else "uOpStale") }

  action deliverUp(rok: bool): bool = all { s.alive, s.up.length() > 0,
    val m = s.up.head()
    val rest = { ...s, up: s.up.tail() }
    all { not(locked(m.p)), m.kind != "op" or not(locked(m.q)),
      if (m.kind == "open") upOpen(rest, m, m.cur, rok)
      else if (m.kind == "close") upClose(rest, m)
      else if (m.kind == "discard") upDiscard(rest, m, m.cur, rok)
      else if (m.kind == "op") upOp(rest, m, m.cur, rok)
      else upSubmit(rest, m, m.cur) } }

  // ================================================================ the backend: reads, saves, drafts
  // A read of a held page (watcher hint or rescan) applied at once. A failed read changes nothing (stutter).
  // s2.1 (2026-10-09): any read that changes the page, also one equal to the last read (s2 required
  // d != obs, which skipped a read after Tine's own save of bytes another program then wrote back).
  action observe(p: int): bool = { val pg = s.pages.get(p); val d = s.disk.get(p); val v1 = g.vc + 1
    val nw = table(pg, d, v1)
    all { s.alive, pg.held, not(locked(p)), nw != pg,
      commit({ ...s, pages: s.pages.set(p, nw) }, { ...g, vc: if (nw.ver == v1) v1 else g.vc }, "observe") } }

  // §4.1 guarded publish: temp file written and fsynced, guard check, rename, directory fsync.
  action flush(p: int): bool = { val pg = s.pages.get(p)
    all { s.alive, not(s.job.on), pg.held, not(pg.conflict), pg.buf != ABSENT, dirty(pg),
      commit({ ...s, job: { on: true, p: p, bytes: pg.buf, base: pg.base, ver: pg.ver, phase: 1, ep: 0 } }, g, "flush") } }

  // The guard: the target must hold exactly the base; otherwise the read becomes an observation (Mismatch).
  // MC / MC1: the guard passes anyway.
  action check = all { s.alive, s.job.on, s.job.phase == 1,
    val p = s.job.p
    val d = s.disk.get(p)
    val v1 = g.vc + 1
    if (d != s.job.base and MUTANT != "MC" and MUTANT != "MC1") {
      val nw = table(s.pages.get(p), d, v1)
      commit({ ...s, job: NOJOB, pages: s.pages.set(p, nw) }, { ...g, vc: if (nw.ver == v1) v1 else g.vc }, "mismatch")
    } else commit({ ...s, job: { ...s.job, phase: 2 } }, { ...g, guard: d == s.job.base }, "check") }

  // C: the rename follows a guard check that found the base (R1 allows only a write after that check).
  // G: the bytes the guard check found (the save's base; R1 writes after the check are C's accepted race)
  // were shown to the user or written by Tine, unless the save writes those same bytes.
  action rename = all { s.alive, s.job.on, s.job.phase == 2,
    val p = s.job.p
    val violates = not(g.guard) or (s.disk.get(p) != s.job.base and not(RACES.contains("R1")))
    val b0 = s.job.base
    val unseen = g.guard and s.job.bytes != b0 and not(g.seen.get(p).contains(b0) or g.wrote.get(p).exists(w => w._1 == b0)
      or g.opRead.get(p).contains(b0))
    // s3: a save of "no file" (a delete) moves the file to the graph trash, in one rename of the file (MDT: it
    // removes it). The move becomes durable with the directory sync, like the path (dirSync, power).
    val del = s.job.bytes == ABSENT and s.disk.get(p) != ABSENT
    val ntrash = if (del and MUTANT != "MDT") s.trash.set(p, s.trash.get(p).union(Set(s.disk.get(p)))) else s.trash
    commit({ ...s, disk: s.disk.set(p, s.job.bytes), trash: ntrash, job: { ...s.job, phase: 3, ep: g.ext.get(p) } },
      { ...g, guard: false, wrote: g.wrote.set(p, g.wrote.get(p).union(Set((s.job.bytes, s.job.ver)))),
              removed: if (del) g.removed.set(p, g.removed.get(p).union(Set(s.disk.get(p)))) else g.removed,
              bad: g.bad.union(if (violates) Set("C-overwrote-external") else Set())
                            .union(if (unseen) Set("G-overwrote-unseen") else Set()) }, "rename") }

  // Directory fsync. On a weak filesystem it succeeds without making the rename durable (R3). A failure is
  // Uncertain. MRE: an Uncertain save is answered as clean.
  action dirSync(ok: bool): bool = all { s.alive, s.job.on, s.job.phase == 3,
    val j = s.job
    val p = j.p
    val d = s.disk.get(p)
    val durable = ok and not(RACES.contains("weak")) and d == j.bytes
    val st = if (durable) s.stable.set(p, d) else s.stable
    val tst = if (durable and MUTANT != "MTS") s.trashStable.set(p, s.trash.get(p)) else s.trashStable
    val nw = if (not(ok) and MUTANT == "MRE") { ...s.pages.get(p), base: j.bytes, typed: false, risk: false }
             else onReply(s.pages.get(p), if (ok) "Published" else "Uncertain", j.bytes)
    val pr = g.promise.get(p)
    commit({ ...s, stable: st, trashStable: tst, job: NOJOB, pages: s.pages.set(p, nw) },
      { ...g, promise: if (ok) g.promise.set(p, ack(pr, j.bytes, j.ver, true, j.ep)) else g.promise,
              delDurable: if (durable and j.bytes == ABSENT) g.delDurable.set(p, g.removed.get(p)) else g.delDurable }, "dirSync") }

  // A write error before the rename (temp write, sharing violation, failed read of the target).
  action saveFail = all { s.alive, s.job.on, s.job.phase <= 2,
    commit({ ...s, job: NOJOB, pages: s.pages.set(s.job.p, onReply(s.pages.get(s.job.p), "Failed", s.job.bytes)) },
      { ...g, guard: false }, "saveFail") }

  // The draft store follows the page (one file per page; the newest state is written). A write can fail
  // (disk full; Martin 2026-10-05 ruling 5): nothing changes and the user sees a sticky error (a stutter).
  // MBD: a text typed on an older version is recorded with the current disk bytes as its base.
  action draftSync(p: int): bool = { val pg = s.pages.get(p); val e0 = draftEntry(pg)
    val e = if (MUTANT == "MBD" and e0.base == UNKNOWN) { ...e0, base: s.disk.get(p) } else e0
    all { s.alive, s.drafts.get(p) != e,
      val ng = if (e.bytes != NONE) { ...g,
          wrote: g.wrote.set(p, g.wrote.get(p).union(Set((e.bytes, e.ver)))),
          promise: g.promise.set(p, ack(g.promise.get(p), e.bytes, e.ver, false, 0)) } else g
      commit({ ...s, drafts: s.drafts.set(p, e) }, ng, "draftWrite") } }

  // §6.4 graph switch / window close: every dirty page is put at risk (drafted); the switch happens only when
  // the window has sent everything, the backend has taken it, every page is clean or at risk, and the draft
  // store mirrors the pages exactly (NU2: a discarded edit is not kept). MX: a page at risk need not be drafted.
  action switchReq = all { s.alive, PAGES.exists(p => { val pg = s.pages.get(p); pg.held and dirty(pg) and not(pg.risk) }),
    commit({ ...s, pages: PAGES.mapBy(p => { val pg = s.pages.get(p) if (pg.held and dirty(pg)) { ...pg, risk: true } else pg }) }, g, "switchReq") }
  val canSwitch: bool = s.alive and not(s.job.on) and s.up.length() == 0 and PAGES.forall(p => {
    val pg = s.pages.get(p)
    val a = s.w.get(p)
    not(a.pend) and not(a.sent) and (not(pg.held) or clean(pg) or pg.risk)
      and (s.drafts.get(p) == draftEntry(pg) or (MUTANT == "MX" and pg.held and pg.risk)) })
  action switchFin = all { canSwitch, commit(down(s), gDown(g), "switchFin") }

  // ================================================================ s3: page operations
  // A page operation is requested by a client (the window or the CLI) and applied by the backend in one step,
  // under the locks of every page it changes (it waits while a save holds one). It reads each page from its
  // buffer if the backend holds it, else from disk (loading it). Every page it changes must be clean: the
  // client saves first, and until then the operation is refused (D4; a refusal changes nothing, so it is not
  // a step here). Its new texts become ordinary edits, saved by ordinary guarded saves; an interrupted
  // operation is left partly done, each page safe on its own, and running it again completes it (D5).
  pure def opCur(sys: Sys, p: int): int = { val pg = sys.pages.get(p) if (pg.held) pg.buf else sys.disk.get(p) }
  // The page an operation edits, at version v with text t: a held page keeps its reads; a page not held is
  // loaded from disk first.
  pure def opEdit(sys: Sys, p: int, t: int, v: int): Page = {
    val pg = sys.pages.get(p)
    val d = sys.disk.get(p)
    val base = if (pg.held) pg else { ...NOPAGE, held: true, buf: d, base: d, obs: d }
    { ...base, buf: t, typed: true, ver: v } }
  pure def opClean(sys: Sys, p: int): bool = { val pg = sys.pages.get(p) not(pg.held) or clean(pg) }
  def opFree(ps: Set[int]): bool = not(s.job.on and ps.contains(s.job.p))

  // Rename src to dst, rewriting the references in the pages refs to the texts rt. The page's text moves to
  // dst exactly like s2's move: the receiving page's new text is written to the draft store first and is at
  // risk until saved, and src's new text is "no file", also drafted and acknowledged, so an interrupted rename
  // completes at launch (MRN5: src's deletion neither drafted nor acknowledged, leaving a duplicate the rename
  // cannot be re-run over). Every page the operation changes is acknowledged: input the window typed on an
  // older version and sends later becomes a conflict over it (Lean s3 round 1 found B violated when src's
  // deletion was drafted but not acknowledged).
  // All the operation's drafts are written in one step: the implementation writes them as one file (one
  // versioned record per page; launch takes each page's newest record), so a crash or a failed write leaves all
  // of them or none. MRN1: dst not drafted. Each rewritten referring page is handled like dst: its
  // new text is drafted first and acknowledged, so an interrupted rename loses no rewrite that was applied
  // (MRN4: referrers not drafted). (A rewrite can still be superseded like any acknowledged text: input the
  // window typed on the older version and sends later becomes a conflict whose newer text replaces it.) dst must be free: no file and no text (no-clobber;
  // MRN3: an existing dst is overwritten). When src already has no file (a re-run after an interrupted
  // rename), only the references are rewritten. MRN1: no draft for dst. MRN2: a referring page that is not
  // clean is rewritten anyway.
  // Ghost: the operation may replace src, dst and the clean referring pages (D4: the client saved them; their
  // text is on disk). B checks any other page it replaces (MRN2: a referring page with unsaved input). Each
  // changed page records the bytes the operation computed its text from (G accepts replacing those): src and
  // the referrers their read; dst only "no file". Direct properties: the operation changes only clean pages
  // (D4-unclean-page-changed) and never a dst that exists (rename-clobbered-target).
  // What the model does not check (implementation obligations, SPEC-s3 §4): that each rewrite is computed from
  // the bytes read under the lock, and that every referring page is found. rt and refs are arbitrary here.
  action opRename(src: int, dst: int, refs: Set[int], rt: int -> int): bool = {
    val text = opCur(s, src)
    val full = text != ABSENT
    val named = if (full) Set(src, dst) else Set()
    val all3 = named.union(refs)
    val v = PAGES.mapBy(p => g.vc + 1 + p)
    val nd = { ...opEdit(s, dst, text, v.get(dst)), risk: true }
    val ns = { ...opEdit(s, src, ABSENT, v.get(src)), risk: true }
    val pages0 = if (full) s.pages.set(dst, nd).set(src, ns) else s.pages
    val nr = PAGES.mapBy(r => { ...opEdit(s, r, if (rt.keys().contains(r)) rt.get(r) else ABSENT, v.get(r)), risk: true })
    val pages = refs.fold(pages0, (acc, r) => acc.set(r, nr.get(r)))
    val drafted0 = if (not(full)) s.drafts
      else if (MUTANT == "MRN1") s.drafts.set(src, draftEntry(ns))
      else if (MUTANT == "MRN5") s.drafts.set(dst, draftEntry(nd))
      else s.drafts.set(dst, draftEntry(nd)).set(src, draftEntry(ns))
    val drafted = if (MUTANT == "MRN4") drafted0 else refs.fold(drafted0, (acc, r) => acc.set(r, draftEntry(nr.get(r))))
    val gotten = refs.map(r => (r, rt.get(r))).union(if (full) Set((dst, text)) else Set())
      .union(if (full and MUTANT != "MRN5") Set((src, ABSENT)) else Set())
    val read = PAGES.mapBy(p => g.opRead.get(p).union(
      if (p == dst and full) Set(opCur(s, dst)).filter(x => x == ABSENT)
      else if ((p == src and full) or refs.contains(p)) Set(s.pages.get(p)).map(pg => if (pg.held) pg.base else s.disk.get(p))
      else Set()))
    all { s.alive, src != dst, opFree(all3), refs.forall(r => r != src and r != dst),
      not(full) or opClean(s, src),
      not(full) or ((opCur(s, dst) == ABSENT and opClean(s, dst)) or MUTANT == "MRN3"),
      refs.forall(r => opClean(s, r) or MUTANT == "MRN2"),
      all3 != Set(),
      commit({ ...s, pages: pages, drafts: drafted },
        { ...g, vc: g.vc + PAGES.size(), mine: named.union(refs.filter(r => opClean(s, r))), opRead: read,
          wrote: gotten.fold(g.wrote, (acc, e) => acc.set(e._1, acc.get(e._1).union(Set((e._2, v.get(e._1)))))),
          promise: gotten.fold(g.promise, (acc, e) => acc.set(e._1, ack(acc.get(e._1), e._2, v.get(e._1), false, 0))),
          bad: g.bad.union(if (all3.forall(p => opClean(s, p))) Set() else Set("D4-unclean-page-changed"))
                    .union(if (full and opCur(s, dst) != ABSENT) Set("rename-clobbered-target") else Set()) },
        if (full) "opRename" else "opRenameRefs") } }

  // Delete: the page's new text is "no file", drafted and acknowledged like every page an operation changes
  // (so a crash before its save completes it at launch, and input the window typed on the older version and
  // sends later becomes a conflict over it; MDD: not drafted nor acknowledged, the Lean s3 round 1
  // counterexample to B). Its save moves the file to the graph trash. A page with no file has nothing to delete.
  action opDelete(p: int): bool = {
    val pg = s.pages.get(p)
    val v1 = g.vc + 1
    val np = { ...opEdit(s, p, ABSENT, v1), risk: MUTANT != "MDD" }
    all { s.alive, opFree(Set(p)), opClean(s, p), opCur(s, p) != ABSENT,
      commit({ ...s, pages: s.pages.set(p, np), drafts: if (MUTANT == "MDD") s.drafts else s.drafts.set(p, draftEntry(np)) },
        { ...g, vc: v1, mine: Set(p), opRead: g.opRead.set(p, g.opRead.get(p).union(Set(if (pg.held) pg.base else s.disk.get(p)))),
          wrote: if (MUTANT == "MDD") g.wrote else g.wrote.set(p, g.wrote.get(p).union(Set((ABSENT, v1)))),
          promise: if (MUTANT == "MDD") g.promise else g.promise.set(p, ack(g.promise.get(p), ABSENT, v1, false, 0)) },
        "opDelete") } }

  // The save of "no file": s2's guarded save (flush) with the delete as its file step (rename above).
  action flushDel(p: int): bool = { val pg = s.pages.get(p)
    all { s.alive, not(s.job.on), pg.held, not(pg.conflict), pg.buf == ABSENT, dirty(pg),
      commit({ ...s, job: { on: true, p: p, bytes: pg.buf, base: pg.base, ver: pg.ver, phase: 1, ep: 0 } }, g, "flush") } }

  // ================================================================ environment
  pure def window(sys: Sys, p: int): bool =     // R1: between the guard check and the rename
    sys.job.on and sys.job.phase == 2 and sys.job.p == p
  // Another program writes the path. dur: its write is already durable; otherwise a power cut may revert it
  // (Astra 2026-10-08: an honest editor or sync tool need not fsync), and Tine cannot tell which.
  action extWriteD(p: int, v: int, dur: bool): bool = all { v != s.disk.get(p), not(window(s, p)) or RACES.contains("R1"),
    commit({ ...s, disk: s.disk.set(p, v), stable: if (dur) s.stable.set(p, v) else s.stable },
      { ...g, ext: g.ext.set(p, g.ext.get(p) + 1) }, "extWrite") }
  action extWrite(p: int, v: int): bool = extWriteD(p, v, true)

  action crash = all { s.alive, RACES.contains("crash"), commit(down(s), gDown(g), "crash") }

  // The window reloads (webview crash, reload) while the backend runs on. Its unsent input is lost (never
  // acknowledged); requests it sent are still taken, and their answers go to nobody. MDQ: they are dropped.
  action windowCrash = all { s.alive, RACES.contains("crash"),
    val q = if (MUTANT == "MDQ") List() else s.up.foldl(List(), (acc, u) => acc.append({ ...u, cur: false }))
    commit({ ...s, w: PAGES.mapBy(_ => NOW), mb: PAGES.mapBy(_ => NOMAIL), up: q }, g, "windowCrash") }

  // Power cut: every path holds its old (stable) or its new bytes; memory is lost. On a weak filesystem an
  // acknowledged save may revert (R3): such a promise ends if no draft holds its text.
  action power(keep0: bool, keep1: bool): bool = powerK(PAGES.filter(p => (p == 0 and keep0) or (p == 1 and keep1)))
  action powerK(keep: Set[int]): bool = all { RACES.contains("power"),
    val nd = PAGES.mapBy(p => if (keep.contains(p)) s.disk.get(p) else s.stable.get(p))
    val np = PAGES.mapBy(p => {
      val pr = g.promise.get(p)
      val kept = Set(nd.get(p), s.drafts.get(p).bytes).contains(pr.bytes)
      if (RACES.contains("weak") and pr.on and pr.saved and not(kept)) endPromise(pr, pr.ver) else pr })
    // s3: a delete's move to the trash is one rename: it survives exactly when its path keeps its new state.
    val nt = PAGES.mapBy(p => if (keep.contains(p)) s.trash.get(p) else s.trashStable.get(p))
    // A delete the power cut reverted (not durable, path not kept) no longer removed anything: its bytes are back
    // at the path if they were ever durable there, or lost with another program's unflushed write (accepted).
    val nr = PAGES.mapBy(p => if (keep.contains(p)) g.removed.get(p) else g.delDurable.get(p))
    commit({ ...down(s), disk: nd, stable: nd, trash: nt, trashStable: nt },
      { ...gDown(g), promise: np, removed: nr, delDurable: PAGES.mapBy(p => g.delDurable.get(p).intersect(nr.get(p))) }, "power") }

  // Launch (also returning to the graph): every draft becomes its page's buffer, at risk, before anything
  // else touches the page. A restored buffer is a new version. Text typed on an older version is restored with its
  // unknown base (MRB: rebased onto the disk), so it is never saved until resolved or discarded.
  def restored: int -> Page = PAGES.mapBy(p => {
      val d = s.drafts.get(p)
      if (d.bytes == NONE) NOPAGE
      else { ...NOPAGE, held: true, buf: d.bytes, base: if (MUTANT == "MRB" and d.base == UNKNOWN) s.disk.get(p) else d.base,
             obs: NONE, ver: g.vc + 1 + PAGES.filter(q => q < p and s.drafts.get(q).bytes != NONE).size(), risk: true, typed: true } })
  action launch = all { not(s.alive),
    commit({ ...s, alive: true, pages: restored },
      { ...g, vc: g.vc + PAGES.filter(p => s.drafts.get(p).bytes != NONE).size(), }, "launch") }

  // ================================================================ the next-state relation
  pure val BOOLS = Set(true, false)
  action step = any {
    nondet p = PAGES.oneOf() any { wOpen(p), wSend(p), wDiscard(p), wClose(p), wRecv(p), flush(p), observe(p), draftSync(p) },
    nondet p = PAGES.oneOf() nondet v = TEXTS.oneOf() any { wEdit(p, v), wResolve(p, v) },
    nondet p = PAGES.oneOf() nondet q = PAGES.oneOf() nondet dS = TEXTS.oneOf() nondet dD = TEXTS.oneOf() wOpTo(p, q, dS, dD),
    check, rename, saveFail, switchReq, switchFin, crash, windowCrash,
    launch,
    nondet ok = BOOLS.oneOf() any { dirSync(ok), deliverUp(ok) },
    nondet p = PAGES.oneOf() nondet v = TEXTS.union(Set(ABSENT)).oneOf() nondet dur = BOOLS.oneOf() extWriteD(p, v, dur),
    nondet keep = PAGES.powerset().oneOf() powerK(keep),
    // s3
    nondet p = PAGES.oneOf() any { opDelete(p), flushDel(p) },
    nondet src = PAGES.oneOf() nondet dst = PAGES.oneOf() nondet refs = PAGES.exclude(Set(src, dst)).powerset().oneOf()
      nondet rt = refs.setOfMaps(TEXTS).oneOf() opRename(src, dst, refs, rt),
  }

  // ================================================================ THE GUARANTEE
  // A. No loss. Tine promises, per page, the newest text it has acknowledged: by a Published save (dated at
  //    its guarded rename), a durable draft, an applied move (its receiving page's new text), or any other
  //    non-user step that shows a page as saved (see commit). After any crash, power cut or graph switch,
  //    each promised text, or a newer text of that page that Tine wrote, is at its path or in the draft store.
  //    A promise ends only when: another program writes the path after Tine's own guarded rename put the
  //    promised bytes there (Q12); the user discards the page and the bytes shown differ; a weak-filesystem
  //    power cut (R3).
  //    Moves: with the receiver's text promised from the moment the move is applied, a move that removes text
  //    from one page can lose it only by losing that promise.
  def copies(p: int): Set[int] = Set(s.disk.get(p), s.drafts.get(p).bytes)
  val noLoss: bool = s.alive or PAGES.forall(p => {
    val pr = g.promise.get(p)
    not(pr.on) or (pr.saved and g.ext.get(p) > pr.ep) or
    copies(p).exists(c => c == pr.bytes or g.wrote.get(p).exists(w => w._1 == c and w._2 > pr.ver)) })

  // B. Unsaved is never replaced. Outside user actions, crash and power cut, a backend step that replaces or
  //    drops a buffer finds its text acknowledged, or holding no unpublished input and equal to the newest
  //    disk read. Closing the graph additionally accepts a buffer whose text is in the draft store.
  //    A user request may replace only the pages it names; a submit typed on an older version is not a user
  //    action here: it may only replace a buffer that is safe, whose bytes the user was shown (as G: the
  //    typing was done on top of them), or that holds no user input at all (only file bytes, which A and G
  //    govern; ind5: file bytes the user was never shown that a later external write already replaced).
  //    (Checked by each step in commit: a is the page before the step, b after, pr the promise after.)
  def keptB(p: int, nsys: Sys, np: int -> Promise, mine: Set[int], name: str): bool = {
    val a = s.pages.get(p)
    val b = nsys.pages.get(p)
    val replaced = a.held and (not(b.held) or b.buf != a.buf)
    val pr = np.get(p)
    val safe = (not(a.typed) and a.buf == a.obs) or (pr.ver >= a.ver and pr.bytes == a.buf)
    val shown = (name == "uStale" or name == "uOpStale") and (g.seen.get(p).contains(a.buf) or not(a.typed))
    not(replaced) or mine.contains(p) or name == "crash" or name == "power"
      or safe or shown or (name == "switchFin" and nsys.drafts.get(p).bytes == a.buf) }

  // B'. What the window sent is taken by the backend. (1) The window's text is replaced only once the
  //    backend has it: no unsent input, and no request on its way except the one being answered. (2) A
  //    submitted text becomes the page's buffer (checked by the request steps). (3) Until then it is in the
  //    backend's request queue, a window reload included (accepted). A window reload loses unsent input
  //    (never acknowledged). An answer replaces the window's text only once the backend has taken that text.
  pure val WUSER = Set("wEdit", "wResolve", "wOp", "windowCrash", "crash", "power")
  def keptW(p: int, nsys: Sys, owed: Set[(int, int)], name: str): bool = {
    val a = s.w.get(p)
    val b = nsys.w.get(p)
    val replaced = a.on and (not(b.on) or b.text != a.text)
    not(replaced) or WUSER.contains(name) or (not(a.pend) and (not(a.sent) or (name == "wAck" and not(owed.contains((p, a.text)))))) }
  val accepted: bool = not(s.alive) or g.owed.forall(o => s.up.foldl(false, (acc, u) => acc or carries(u).contains(o)))

  // C. A Tine write never replaces bytes other than those its guard check found (R1: another program may
  //    write between the check and the rename).
  // G. A Tine save never replaces file bytes the user was never shown, unless Tine wrote them itself; across
  //    crashes too. So text typed on a version the backend had moved past is never saved over the change it
  //    missed until the user has seen it (resolve) or dropped the text (discard). Bytes are compared as bytes,
  //    like the guard: content changed back to bytes the user has seen counts as seen. (C, G, B, B'(2) are
  //    recorded in g.bad.)
  val transitions: bool = g.bad == Set()

  // D (s3). Bytes a Tine delete removed from a path are at that path or in the trash, across power cuts too.
  val trashed: bool = PAGES.forall(p => g.removed.get(p).forall(b => s.trash.get(p).contains(b) or s.disk.get(p) == b))

  val guarantee: bool = noLoss and transitions and accepted and trashed

  // ---------------------------------------------------------------- TLC: bound and reachability witnesses
  val within: bool = g.vc <= VMAX and PAGES.forall(p => g.ext.get(p) <= EMAX) and s.up.length() <= QMAX
  // Each witness is FALSE once the run reaches the named behaviour.
  val wMove: bool = not(PAGES.exists(p => s.drafts.get(p).bytes != NONE and s.pages.get(p).held and PAGES.exists(q => q != p and s.pages.get(q).typed)))
  val wStale: bool = not(PAGES.exists(p => s.pages.get(p).base == UNKNOWN))
  val wStaleRestored: bool = not(s.alive and PAGES.exists(p => s.pages.get(p).base == UNKNOWN and s.pages.get(p).obs == NONE))
  val wCrashDuplicate: bool = not(not(s.alive) and PAGES.exists(p => s.drafts.get(p).bytes == 3 and PAGES.exists(q => q != p and s.disk.get(q) == 1)))
  val wWindowCrashQueued: bool = not(s.up.foldl(false, (acc, u) => acc or not(u.cur)))
  val wOpenFail: bool = not(PAGES.exists(p => s.mb.get(p).on and s.mb.get(p).ack >= 0 and not(s.mb.get(p).held)))
  val wHeldDiscard: bool = not(PAGES.exists(p => s.pages.get(p).held and s.pages.get(p).risk and not(s.pages.get(p).typed) and s.pages.get(p).buf == s.pages.get(p).base and s.pages.get(p).obs != NONE))
  // @@SCENARIOS@@
}
