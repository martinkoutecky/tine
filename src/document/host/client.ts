// The window client of the page host (STEP3-DESIGN §4; plan v3 §5; REVIEW-3b-plan2
// S4/S5/S7/R7). One instance per graph binding. It transcribes the model's window
// (`storage-s3.qnt` wOpen/wEdit/wSend/wResolve/wDiscard/wOpTo/wClose/wRecv) over a
// page-host port and a document port; it holds no base revisions, retries, drafts
// or conflict detection of its own (§4.3).
//
// Step 3b P2a: unwired. Old persistence (`save/engine.ts`) is the only authority
// until P2b; `boundary.guard.test.ts` ("P2a client is unwired") pins that nothing
// in production imports this folder.

import type { ClipboardSourcePage } from "../../clipboard";
import type { EditKind, EditKinds } from "../../editKind";
import type { Format, PageDto, PageKind } from "../../types";
import { cutSourceMatches } from "../cutSource";
import {
  STALE_VERSION, type DiskToken, type HostPort, type MailNotice, type MailPage, type OwedPage, type PageMail,
  type PageRefusal, type PublishedNeed,
} from "./protocol";

/** What the installed text's bytes were: a file revision, a proved absence, or unknown. */
export type BaselineRev = { kind: "file"; rev: string } | { kind: "no-file" } | { kind: "unknown" };

/** The document's facts about a loaded page instance, as installed. `rev` names
 * the bytes the installed text came from (a loader read or host content). */
export interface PageFacts {
  instance: number;
  path: string | null;
  kind: PageKind;
  format: Format;
  rev: BaselineRev;
}

export type Content = { kind: "page"; dto: PageDto } | { kind: "no-file" };

/** The document side (P2b wires it to `workingSet`/`convert`). */
export interface DocumentPort {
  facts(name: string): PageFacts | null;
  /** The page's current text as a DTO, captured synchronously at send. */
  dto(name: string): PageDto | null;
  /** Replace the page's text (host content, or a move's planned text). */
  install(name: string, content: Content): void;
  /** Content holds outside the client: a block move in flight, or an open
   * editor on a different file (today's `reloadDisposition` "skip"). */
  holds(name: string): boolean;
  /** "Always ask": a push for a clean page waits in the external-change bar. */
  holdsPush(name: string): boolean;
  /** The page was deleted in this window (delete tombstone). */
  tombstoned(name: string): boolean;
}

export interface AssetWrites {
  pending(): readonly Promise<unknown>[];
  started(): number;
}

type Request = "open" | "submit" | "move" | "discard" | "close";
type Phase =
  | { kind: "idle" }
  | { kind: "sending"; id: number; request: Request; editSeq: number }
  | { kind: "inFlight"; id: number; request: Request; editSeq: number };

export interface Answered { took: boolean; refusal: PageRefusal | string | null }

interface Acquisition { pin: boolean; commit?: () => Promise<boolean> | boolean }

interface Page {
  name: string;
  key: string | null;
  /** The host holds the page for this window (model `on`). */
  on: boolean;
  /** The host version the window's text is authored on (model `bv`); null: none. */
  version: number | null;
  editSeq: number;
  sentSeq: number;
  phase: Phase;
  /** Mail that arrived while a request was still `sending` (F1), replayed at admission. */
  stash: PageMail[];
  /** The authoring baseline an Open compares (S5): fixed when the Open is sent. */
  baseline: (PageFacts & { version: number | null; entry: boolean }) | null;
  observed: { version: number; disk: DiskToken | null; conflict: boolean; risk: boolean } | null;
  notice: MailNotice | null;
  /** Host content withheld from the text by the installation gate (S4). */
  held: { content: Content; version: number } | null;
  /** The latest version the host took from this window: what must publish (S1). */
  needed: number | null;
  refusal: PageRefusal | string | null;
  refs: Set<Acquisition>;
  kinds: EditKind[];
  timer: ReturnType<typeof setTimeout> | null;
  waiters: (() => void)[];
  answered: ((answer: Answered) => void)[];
}

/** A cut source stamped with the host page it was cut from (R7). */
export interface HostCutSource extends ClipboardSourcePage { key: string | null; session: number }

/** A Concord review's identity, taken when the user reviewed (S7). */
export interface ReviewTicket { name: string; instance: number; session: number; editSeq: number;
  version: number | null; disk: DiskToken }

const SEND_DEBOUNCE_MS = 400;

function sameToken(a: DiskToken | null | undefined, b: DiskToken | null | undefined): boolean {
  return !!a && !!b && a.kind === b.kind && (a.kind === "no-file" || a.rev === (b as { rev: string }).rev);
}

/** Concord's reviewed disk revision as a typed token: `"absent"` is a proved
 * missing file (`NoFile`), never `File("absent")` (S7). */
export function reviewedDiskToken(conflictRev: string): DiskToken {
  return conflictRev === "absent" ? { kind: "no-file" } : { kind: "file", rev: conflictRev };
}

export class HostClient {
  session = 0;
  private nextId = 1;
  private readonly pages = new Map<string, Page>();
  private readonly byKey = new Map<string, Page>();
  /** Mail for a key no Open has named yet (an answer can beat the Open's reply). */
  private unrouted: PageMail[] = [];
  /** Mail that arrived while the session was being replaced (S8). */
  private rebinding: PageMail[] | null = null;
  /** What this client last installed or had taken on a page instance: the
   * revision and host version its text is (outlives a close; S5). */
  private readonly installed = new Map<string, { instance: number; rev: BaselineRev; version: number | null }>();
  private readonly frozen = new Set<string>();
  /** Every local edit anywhere: a change across a barrier means new work (S1). */
  editClock = 0;

  constructor(
    readonly host: HostPort,
    readonly doc: DocumentPort,
    readonly assets: AssetWrites,
  ) {}

  /** Bind to a session (`page_window_reloaded`): request ids continue above the host's watermark. */
  async bind(): Promise<void> {
    const { session, nextId } = await this.host.windowReloaded();
    this.session = session;
    this.nextId = nextId;
  }

  /** Rebind after a restore or relaunch (S8): drop every page's client state and
   * held mail, take the new session, then reopen the pages still acquired. Mail
   * that arrives meanwhile is replayed only if it carries the new session. */
  async rebind(): Promise<void> {
    const pages = [...this.pages.values()];
    const acquired = pages.filter((page) => page.refs.size).map((page) => [page.name, page.refs] as const);
    for (const page of pages) this.drop(page);
    this.unrouted = [];
    this.installed.clear();
    this.rebinding = [];
    try {
      await this.bind();
    } finally {
      const early = this.rebinding;
      this.rebinding = null;
      for (const [name, refs] of acquired) {
        const page = this.state(name);
        for (const ref of refs) page.refs.add(ref);
        this.open(page);
      }
      for (const mail of early) this.receive(mail);
    }
  }

  // ------------------------------------------------------------- acquisition (S5)

  /** Edit intent: a block enters editing, a component draft activates (`pin`), or
   * an operation targets the page. The first reference opens the page; the page
   * stays open until every reference is released and no input is pending. A pin
   * also holds host content back from the text (S4), and `commit` is what a
   * freeze awaits before it counts the pin's input (a failed commit fails it). */
  acquire(name: string, options: { pin?: boolean; commit?: () => Promise<boolean> | boolean } = {}): () => void {
    const page = this.state(name);
    const ref: Acquisition = { pin: !!options.pin, commit: options.commit };
    page.refs.add(ref);
    this.open(page);
    let released = false;
    return () => {
      if (released) return;
      released = true;
      const current = this.pages.get(name);
      if (!current?.refs.delete(ref)) return;
      if (ref.pin) this.reevaluate(current);
      this.maybeClose(current);
    };
  }

  /** The dirty funnel (`markDirty`/`addDirty`): every programmatic mutation. An
   * edit on a page no surface acquired opens it here (the fallback). */
  noteEdit(name: string, kinds: EditKind | EditKinds, schedule = true): void {
    const page = this.state(name);
    page.editSeq += 1;
    this.editClock += 1;
    for (const kind of typeof kinds === "string" ? [kinds] : kinds) if (!page.kinds.includes(kind)) page.kinds.push(kind);
    this.open(page);
    if (schedule) this.schedule(page);
  }

  // ------------------------------------------------------------- queries

  isDirty(name: string): boolean {
    const page = this.pages.get(name);
    return !!page && page.editSeq > page.sentSeq;
  }

  /** Dirty, a request outstanding, or mail not yet replayed: work the host has not answered. */
  busy(name: string): boolean {
    const page = this.pages.get(name);
    return !!page && (page.editSeq > page.sentSeq || page.phase.kind !== "idle" || page.stash.length > 0);
  }

  conflicted(name: string): boolean {
    return !!this.pages.get(name)?.observed?.conflict;
  }

  /** The authoring version of the page's text (for tests and the UI's state line). */
  versionOf(name: string): number | null {
    return this.pages.get(name)?.version ?? null;
  }

  keyOf(name: string): string | null {
    return this.pages.get(name)?.key ?? null;
  }

  isOpen(name: string): boolean {
    return !!this.pages.get(name)?.on;
  }

  heldVersion(name: string): number | null {
    return this.pages.get(name)?.held?.version ?? null;
  }

  isFrozen(name: string): boolean {
    return this.frozen.has(name);
  }

  names(): string[] {
    return [...this.pages.keys()];
  }

  /** Each scoped page's edit sequence, for the barrier's work identity (S1). */
  editSeqs(names: readonly string[]): number[] {
    return names.map((name) => this.pages.get(name)?.editSeq ?? 0);
  }

  /** The versions the host took from this window for these pages: what must publish. */
  needs(names: readonly string[]): { key: string; version: number }[] {
    return names.flatMap((name) => {
      const page = this.pages.get(name);
      return page?.key && page.needed !== null ? [{ key: page.key, version: page.needed }] : [];
    });
  }

  nameOfKey(key: string): string | null {
    return this.byKey.get(key)?.name ?? null;
  }

  // ------------------------------------------------------------- sending (§4.2)

  /** Send the page's unsent input now (hide, switch, a barrier). */
  sendNow(name: string): void {
    const page = this.pages.get(name);
    if (page) this.send(page);
  }

  /** Resolves when the page has no request outstanding (or the client dropped it). */
  whenIdle(name: string): Promise<void> {
    const page = this.pages.get(name);
    if (!page || page.phase.kind === "idle") return Promise.resolve();
    return new Promise((resolve) => page.waiters.push(resolve));
  }

  /** One drain pass: send the scoped pages' input and wait until each is answered,
   * following an Open with its input (bounded; the barrier's final check decides). */
  async drain(names: readonly string[]): Promise<void> {
    await Promise.all(names.map(async (name) => {
      for (let step = 0; step < 3; step += 1) {
        const page = this.pages.get(name);
        if (!page) return;
        if (page.phase.kind !== "idle") await this.whenIdle(name);
        else if (page.on && page.editSeq > page.sentSeq) this.send(page);
        else return;
      }
    }));
  }

  /** Take disk (R4): wait for the outstanding request, consume the unsent input
   * (as the user asked), then discard. */
  async discard(name: string): Promise<Answered> {
    const page = this.pages.get(name);
    if (!page?.on) return { took: false, refusal: "not-open" };
    await this.whenIdle(name);
    if (!page.on || this.pages.get(name) !== page) return { took: false, refusal: "not-open" };
    page.sentSeq = page.editSeq;
    page.kinds = [];
    return this.request(page, "discard", (id) => this.host.discard(this.session, id, page.key!, page.version ?? STALE_VERSION));
  }

  /** A two-page move (model `wOpTo`): both endpoints open, idle and clean. The
   * planned texts become both pages' text, sent on their versions. */
  move(source: string, receiver: string, sourceDto: PageDto, receiverDto: PageDto, kinds: EditKinds): Promise<Answered> {
    const a = this.pages.get(source);
    const b = this.pages.get(receiver);
    const ready = (page: Page | undefined): page is Page =>
      !!page?.on && page.phase.kind === "idle" && page.editSeq === page.sentSeq && page.stash.length === 0;
    if (source === receiver || !ready(a) || !ready(b)) return Promise.resolve({ took: false, refusal: "endpoint-busy" });
    const before = [this.doc.dto(source), this.doc.dto(receiver)] as const;
    this.doc.install(source, { kind: "page", dto: sourceDto });
    this.doc.install(receiver, { kind: "page", dto: receiverDto });
    for (const page of [a, b]) { page.editSeq += 1; this.editClock += 1; }
    const id = this.nextId++;
    for (const page of [a, b]) page.phase = { kind: "sending", id, request: "move", editSeq: page.editSeq };
    const answers = Promise.all([a, b].map((page) => new Promise<Answered>((resolve) => page.answered.push(resolve))));
    void this.host.move(this.session, id, [a.key!, sourceDto, a.version ?? STALE_VERSION],
      [b.key!, receiverDto, b.version ?? STALE_VERSION], kinds).then((refusal) => {
      if (refusal) {
        // Not admitted: nothing was sent. Put the endpoints back as they were
        // (both were clean), so no half of the move is ever sent as a submit.
        [a, b].forEach((page, i) => {
          if (before[i]) this.doc.install(page.name, { kind: "page", dto: before[i]! });
          page.sentSeq = page.editSeq;
        });
      }
      for (const page of [a, b]) this.admitted(page, id, refusal);
    });
    return answers.then(([x, y]) => ({ took: x.took && y.took, refusal: x.refusal ?? y.refusal }));
  }

  // ------------------------------------------------------------- freeze (S4, §8)

  /** Freeze pages for a transition: await every acquisition's commit hook (a sheet
   * cell, a title rename, an IME composition ending after blur) and honour its
   * failure: one that cannot commit keeps its input and fails the freeze. */
  async freeze(names: readonly string[]): Promise<{ ok: boolean; failed: string[]; unfreeze: () => void }> {
    for (const name of names) this.frozen.add(name);
    const unfreeze = () => { for (const name of names) this.frozen.delete(name); };
    const hooks = names.flatMap((name) => [...(this.pages.get(name)?.refs ?? [])]
      .filter((ref) => ref.commit).map((ref) => ({ name, commit: ref.commit! })));
    const results = await Promise.all(hooks.map(async ({ name, commit }) => {
      try { return (await commit()) ? null : name; } catch { return name; }
    }));
    const failed = [...new Set(results.filter((name): name is string => name !== null))];
    if (failed.length) unfreeze();
    return { ok: failed.length === 0, failed, unfreeze };
  }

  // ------------------------------------------------------------- cut grants (R7)

  /** Stamp a cut's sources with their host page and this session at the cut. */
  stampCutSources(sources: readonly ClipboardSourcePage[]): HostCutSource[] {
    return sources.map((source) => ({ ...source, key: this.keyOf(source.name), session: this.session }));
  }

  /** The grant still names the same instance, file, host page and session, and
   * the page is neither deleted nor conflicted (preflight and after the barrier). */
  cutSourcesUsable(sources: readonly HostCutSource[]): boolean {
    return sources.length > 0 && new Set(sources.map((source) => source.name)).size === sources.length
      && sources.every((source) => {
        const facts = this.doc.facts(source.name);
        const key = this.keyOf(source.name);
        return cutSourceMatches(source, facts ? { name: source.name, kind: facts.kind, id: facts.path } : undefined,
          facts?.instance ?? null)
          && source.session === this.session
          // Like the file id: a source cut before its Open answered gets its key
          // on the same instance, which the generation check pins.
          && (source.key === null || key === source.key)
          && !this.doc.tombstoned(source.name) && !this.conflicted(source.name);
      });
  }

  /** The final synchronous check immediately before identity insertion: usable,
   * and nothing on any source is unsent or unanswered. */
  cutSourcesRetired(sources: readonly HostCutSource[]): boolean {
    return this.cutSourcesUsable(sources) && sources.every((source) => !this.busy(source.name));
  }

  // ------------------------------------------------------------- Concord review (S7)

  /** The review's identity when the user looked at the conflict. */
  reviewTicket(name: string, conflictRev: string): ReviewTicket | null {
    const page = this.pages.get(name);
    const facts = this.doc.facts(name);
    if (!page?.on || !page.observed?.conflict || !facts) return null;
    return { name, instance: facts.instance, session: this.session, editSeq: page.editSeq, version: page.version,
      disk: reviewedDiskToken(conflictRev) };
  }

  /** Apply a reviewed merge. `merge` is the read-only native producer (it may
   * take a while); the review is rechecked only after it returns and the page's
   * own request is answered, immediately before the synchronous capture, so input
   * typed during the merge is never replaced by it. */
  async submitReviewed(ticket: ReviewTicket, merge: () => Promise<PageDto>): Promise<Answered | "review-stale"> {
    const merged = await merge();
    await this.whenIdle(ticket.name);
    const page = this.pages.get(ticket.name);
    if (!page?.on || page.editSeq !== ticket.editSeq || page.version !== ticket.version
      || this.doc.facts(ticket.name)?.instance !== ticket.instance || this.session !== ticket.session) return "review-stale";
    this.doc.install(page.name, { kind: "page", dto: merged });
    page.editSeq += 1;
    this.editClock += 1;
    return this.request(page, "submit", (id) =>
      this.host.submit(this.session, id, page.key!, merged, ticket.version ?? STALE_VERSION, ticket.disk, ["replace-page"]));
  }

  // ------------------------------------------------------------- creation (R4)

  /** Create a page: open it, require the disk state the caller expects (none by
   * default: an existing file is "exists" and nothing is written; an `expected`
   * token that no longer holds sends the text stale, so it becomes a conflict),
   * submit due now, and return once the answered version is published. */
  async createPage(name: string, dto: PageDto, options: { kinds?: EditKinds; expected?: DiskToken } = {}):
    Promise<{ kind: "created"; key: string; version: number } | { kind: "exists" } | { kind: "unpublished" }
      | { kind: "refused"; refusal: PageRefusal | string | null }> {
    const release = this.acquire(name);
    try {
      await this.whenIdle(name);
      const page = this.pages.get(name);
      if (!page?.on) return { kind: "refused", refusal: page?.refusal ?? "not-open" };
      const disk = page.observed?.disk ?? null;
      const expected = options.expected ?? { kind: "no-file" };
      const current = sameToken(disk, expected);
      if (!current && !options.expected) return { kind: "exists" };
      this.doc.install(name, { kind: "page", dto });
      page.editSeq += 1;
      this.editClock += 1;
      const answer = await this.request(page, "submit", (id) => this.host.submit(this.session, id, page.key!, dto,
        current ? page.version ?? STALE_VERSION : STALE_VERSION, null, options.kinds ?? ["create-page"]));
      if (!answer.took || page.needed === null) return { kind: "refused", refusal: answer.refusal };
      const version = page.needed;
      await this.saveNow([page.key!]);
      return await this.published([{ key: page.key!, version }])
        ? { kind: "created", key: page.key!, version } : { kind: "unpublished" };
    } finally {
      release();
    }
  }

  // ------------------------------------------------------------- publication (§4.4, F1)

  saveNow(keys: readonly string[]): Promise<void> {
    return this.host.saveNow(this.session, keys);
  }

  /** Whether every need published under this session: asks again while each
   * bounded host wait passes, false on a terminal notice or a new session. */
  async published(needs: readonly PublishedNeed[]): Promise<boolean> {
    const session = this.session;
    for (;;) {
      const result = await this.host.waitPublished(session, needs);
      if (this.session !== session) return false;
      if (result !== null) return result;
    }
  }

  /** The host's publication debt under this session; null after a new session. */
  async owed(paths: readonly string[] | null): Promise<OwedPage[] | null> {
    const session = this.session;
    const owed = await this.host.owed(session, paths);
    return this.session === session ? owed : null;
  }

  // ------------------------------------------------------------- mail (§3.3, model wRecv)

  receive(mail: PageMail): void {
    if (this.rebinding) { this.rebinding.push(mail); return; }
    if (mail.session !== this.session) return;
    const page = this.byKey.get(mail.key);
    if (!page) {
      if ([...this.pages.values()].some((p) => p.phase.kind === "sending" && p.phase.request === "open"))
        this.unrouted.push(mail);
      return;
    }
    if (page.phase.kind === "sending") { page.stash.push(mail); return; }
    const answer = mail.answer;
    if (answer && page.phase.kind === "inFlight" && answer.id === page.phase.id) this.answer(page, mail);
    else this.push(page, mail);
  }

  /** The answer to the page's outstanding request, consumed exactly once (S4). */
  private answer(page: Page, mail: PageMail): void {
    const answer = mail.answer!;
    const request = (page.phase as { request: Request }).request;
    page.phase = { kind: "idle" };
    page.notice = mail.notice;
    if (answer.outcome.kind === "refused") page.refusal = answer.outcome.reason;
    const result: Answered = { took: answer.took, refusal: answer.outcome.kind === "refused" ? answer.outcome.reason : null };
    if (!mail.page || request === "close") {
      // The host released the page, or this window's view of it (a close
      // unsubscribes at admission; an Open whose read failed holds nothing).
      // Input typed or acquired meanwhile reopens it; a failed Open keeps the
      // text and shows the refusal.
      page.on = false;
      if (page.key) this.byKey.delete(page.key);
      page.key = null;
      page.version = null;
      this.settled(page, result);
      if (request === "close" && (page.refs.size || page.editSeq > page.sentSeq)) this.open(page);
      else if (!page.refs.size && page.editSeq === page.sentSeq) this.drop(page);
      return;
    }
    if (request === "open") page.on = true;
    this.observe(page, mail.page);
    if (answer.took) {
      page.needed = answer.version;
      this.remember(page, mail.page.text.kind === "unchanged" ? mail.page.text.rev : undefined, answer.version);
    }
    const content = this.contentOf(mail.page);
    const newer = page.editSeq > page.sentSeq;
    if (newer || this.holds(page)) {
      // Local input (or a component draft) newer than this request keeps its
      // text. It moves to the answer's version only when the host took the
      // request's text; an Open grants a version only by the baseline rule (D-b);
      // otherwise it keeps the version it was typed on.
      if (answer.took) page.version = answer.version;
      else if (request === "open" && this.openGrants(page, mail.page)) page.version = mail.page.version;
      page.held = !newer && content ? { content, version: mail.page.version } : null;
    } else if (content) {
      this.install(page, content, mail.page.version);
    } else if (answer.took) {
      page.version = answer.version;
    }
    this.settled(page, result);
    if (page.editSeq > page.sentSeq) this.schedule(page);
    this.maybeClose(page);
  }

  /** Mail without an answer for this page: observation always; text only for an
   * idle, clean, unheld page. Notice-only mail never releases held content. */
  private push(page: Page, mail: PageMail): void {
    page.notice = mail.notice;
    if (!mail.page || !page.on) return;
    this.observe(page, mail.page);
    const content = this.contentOf(mail.page);
    if (!content) return;
    if (page.phase.kind === "idle" && page.editSeq === page.sentSeq && !this.holds(page) && !this.doc.holdsPush(page.name))
      this.install(page, content, mail.page.version);
    else page.held = { content, version: mail.page.version };
  }

  /** A hold ended (unpin, an editor left, a block move finished, or the user took
   * the held external change when `acceptPush`): install held content if no local
   * input exists and nothing is outstanding; input typed meanwhile keeps its
   * version, so it is sent stale and becomes a conflict. */
  release(name: string, acceptPush = false): void {
    const page = this.pages.get(name);
    if (page) this.reevaluate(page, acceptPush);
  }

  // ------------------------------------------------------------- internals

  private state(name: string): Page {
    let page = this.pages.get(name);
    if (!page) {
      page = { name, key: null, on: false, version: null, editSeq: 0, sentSeq: 0, phase: { kind: "idle" }, stash: [],
        baseline: null, observed: null, notice: null, held: null, needed: null, refusal: null, refs: new Set(),
        kinds: [], timer: null, waiters: [], answered: [] };
      this.pages.set(name, page);
    }
    return page;
  }

  private holds(page: Page): boolean {
    return [...page.refs].some((ref) => ref.pin) || this.doc.holds(page.name);
  }

  private reevaluate(page: Page, acceptPush = false): void {
    if (!page.held || page.phase.kind !== "idle") return;
    if (page.editSeq > page.sentSeq) { page.held = null; return; }
    if (this.holds(page) || (!acceptPush && this.doc.holdsPush(page.name))) return;
    const { content, version } = page.held;
    this.install(page, content, version);
  }

  private open(page: Page): void {
    if (page.on || page.phase.kind !== "idle" || this.rebinding) return;
    const facts = this.doc.facts(page.name);
    const known = this.installed.get(page.name);
    const mine = known && facts && known.instance === facts.instance ? known : null;
    const baseline = facts ? { ...facts, rev: mine?.rev ?? facts.rev, version: mine?.version ?? null, entry: false } : null;
    page.baseline = baseline;
    page.refusal = null;
    const id = this.nextId++;
    page.phase = { kind: "sending", id, request: "open", editSeq: page.sentSeq };
    void this.host.open(this.session, id, { name: page.name, kind: facts?.kind ?? "page", path: facts?.path ?? null })
      .then((reply) => {
      if (this.pages.get(page.name) !== page || page.phase.kind !== "sending" || page.phase.id !== id) return;
      if ("reason" in reply) { this.admitted(page, id, reply); return; }
      page.key = reply.key;
      if (page.baseline) page.baseline.entry = reply.baselineEntry;
      this.byKey.set(reply.key, page);
      const mine = this.unrouted.filter((mail) => mail.key === reply.key);
      this.unrouted = this.unrouted.filter((mail) => mail.key !== reply.key);
      page.stash.unshift(...mine);
      this.admitted(page, id, null);
    });
  }

  /** The baseline rule (D-b; R5, S5): input typed before the Open's answer is
   * granted the answered version when the host still holds the page at the
   * version this instance's text is (a reopen), or when the answer is clean, its
   * buffer is the disk's bytes, and those bytes are the ones the input's text
   * was installed from, on the same entry, kind, format and page instance.
   * Anything else (unknown revision, a held or notice-only observation, another
   * file with the same bytes) keeps no version: the input is sent stale. */
  private openGrants(page: Page, mail: MailPage): boolean {
    const base = page.baseline;
    if (!base || this.doc.facts(page.name)?.instance !== base.instance) return false;
    if (base.version !== null && mail.version === base.version) return true;
    if (base.rev.kind === "unknown" || mail.conflict || mail.risk) return false;
    const disk = mail.disk ?? null;
    if (mail.text.kind === "no-file") return base.rev.kind === "no-file" && disk?.kind === "no-file";
    if (mail.text.kind !== "page" || base.rev.kind !== "file" || !base.entry) return false;
    const dto = mail.text.dto;
    return dto.kind === base.kind && (dto.format ?? "md") === base.format
      && disk?.kind === "file" && disk.rev === base.rev.rev && dto.rev === disk.rev;
  }

  private observe(page: Page, mail: MailPage): void {
    page.observed = { version: mail.version, disk: mail.disk ?? null, conflict: mail.conflict, risk: mail.risk };
  }

  private contentOf(mail: MailPage): Content | null {
    if (mail.text.kind === "page") return { kind: "page", dto: mail.text.dto };
    if (mail.text.kind === "no-file") return { kind: "no-file" };
    return null;
  }

  private install(page: Page, content: Content, version: number): void {
    this.doc.install(page.name, content);
    page.version = version;
    page.held = null;
    this.remember(page, content.kind === "no-file" ? null : content.dto.rev ?? undefined, version);
  }

  /** The text's revision and version after host content or a took answer:
   * `null` is no file, `undefined` unknown. Held and notice-only mail never
   * reach here, so they never advance the baseline (S5). */
  private remember(page: Page, rev: string | null | undefined, version: number): void {
    const instance = this.doc.facts(page.name)?.instance;
    if (instance === undefined) return;
    this.installed.set(page.name, { instance, version, rev: rev === null ? { kind: "no-file" }
      : rev === undefined ? { kind: "unknown" } : { kind: "file", rev } });
  }

  private schedule(page: Page): void {
    if (!page.on || page.phase.kind !== "idle") return;
    if (page.timer) clearTimeout(page.timer);
    page.timer = setTimeout(() => { page.timer = null; this.send(page); }, SEND_DEBOUNCE_MS);
  }

  private send(page: Page): void {
    if (page.timer) { clearTimeout(page.timer); page.timer = null; }
    if (!page.on || page.phase.kind !== "idle" || page.editSeq === page.sentSeq || this.pages.get(page.name) !== page) return;
    const dto = this.doc.dto(page.name);
    if (!dto) return;
    const kinds = page.kinds.length ? page.kinds : ["replace-page" as const];
    page.kinds = [];
    void this.request(page, "submit", (id) => this.host.submit(this.session, id, page.key!, dto,
      page.version ?? STALE_VERSION, null, [kinds[0], ...kinds.slice(1)]), kinds);
  }

  /** Capture synchronously and send; resolves with the request's answer. */
  private request(page: Page, request: Request, call: (id: number) => Promise<PageRefusal | null>,
    kinds: readonly EditKind[] = []): Promise<Answered> {
    const id = this.nextId++;
    page.phase = { kind: "sending", id, request, editSeq: page.editSeq };
    const answered = new Promise<Answered>((resolve) => page.answered.push(resolve));
    void call(id).then((refusal) => {
      if (refusal) for (const kind of kinds) if (!page.kinds.includes(kind)) page.kinds.push(kind);
      this.admitted(page, id, refusal);
    });
    return answered;
  }

  /** The command returned: admitted (the captured input is now the host's), or
   * refused (nothing was sent; the text stays and the reason is shown). */
  private admitted(page: Page, id: number, refusal: PageRefusal | null): void {
    if (page.phase.kind !== "sending" || page.phase.id !== id) return;
    if (refusal) {
      page.phase = { kind: "idle" };
      page.refusal = refusal;
      const stash = page.stash;
      page.stash = [];
      this.settled(page, { took: false, refusal });
      for (const mail of stash) this.receive(mail);
      return;
    }
    if (page.phase.request === "submit" || page.phase.request === "move") page.sentSeq = page.phase.editSeq;
    page.phase = { kind: "inFlight", id, request: page.phase.request, editSeq: page.phase.editSeq };
    const stash = page.stash;
    page.stash = [];
    for (const mail of stash) this.receive(mail);
  }

  private settled(page: Page, answer: Answered): void {
    for (const resolve of page.answered.splice(0)) resolve(answer);
    for (const resolve of page.waiters.splice(0)) resolve();
  }

  private maybeClose(page: Page): void {
    if (this.pages.get(page.name) !== page || page.refs.size || page.phase.kind !== "idle"
      || page.editSeq > page.sentSeq || page.timer) return;
    if (!page.on) { this.drop(page); return; }
    void this.request(page, "close", (id) => this.host.close(this.session, id, page.key!));
  }

  private drop(page: Page): void {
    if (page.timer) clearTimeout(page.timer);
    if (page.key) this.byKey.delete(page.key);
    this.pages.delete(page.name);
    page.phase = { kind: "idle" };
    this.settled(page, { took: false, refusal: "rebound" });
  }
}
