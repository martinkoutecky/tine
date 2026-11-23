import type { PageMail, PageRefusal, PublishedNeed } from "./document";
import type { PageDto, PageRead, ResolvedPage } from "./types";

/** The graph a mock page host writes: the browser core's memory graph (`mockGraph.ts`). */
export interface PageHostGraph {
  page(path: string): Promise<PageRead | null>;
  resolve(name: string, kind: "journal" | "page"): Promise<ResolvedPage>;
  /** Serialize `dto` over the file's text with the store's page serializer and
   * replace the file: the key's new version, or the desktop host's refusal. */
  submit(key: string, dto: PageDto, version: number): Promise<{ version: number } | { refused: PageRefusal }>;
  /** Replace (null: delete) one file, then recapture the pages it holds. */
  write(path: string, text: string | null): Promise<void>;
  /** Forget every edit (a window reload reloads the preview's graph). */
  reset(): void;
}

/** The browser mock's page host: one window, every request answered at once.
 * A submit writes through the browser core (ADR 0081), so a later read or Open
 * answers the saved text, until a window reload reopens the fixture graph. The
 * host keeps only session bookkeeping: each key's version and the page mail. Answers arrive as page mail after the command
 * returns, as from the native host. Tests that need refusals or held answers
 * use the host fakes instead.
 *
 * - A file's revision is the host's: the session and the key's version, which
 *   every write of the file moves (the core's page reads carry none). Open,
 *   submit mail, the publication wait and the exact deletion all name it.
 * - A request answers only in the session it was made in: after a window
 *   reload, a pending one sends no mail and moves no version.
 * - The preview writes one page per request (ADR 0081 Decision 4): a
 *   cross-page move is refused before either endpoint is written. */
export function mockPageHost(graph: PageHostGraph, refusal: () => string) {
  const listeners = new Set<(mail: PageMail) => void>();
  const versions = new Map<string, number>();
  const notice = { failures: 0, saveError: false, operation: null, osError: null, draftError: false, dropped: false, conflictReported: false, custodyError: false,
    indexError: false, observeError: false, twin: null };
  let nextId = 1;
  let session = 1;
  const version = (key: string) => versions.get(key) ?? 1;
  const revision = (key: string) => `preview-${session}-${version(key)}`;
  /** Mail of the request made in session `from`; none once a reload ended it. */
  const mail = (from: number, key: string, id: number, page: PageMail["page"], took: boolean) => queueMicrotask(() => {
    if (from !== session) return;
    for (const listener of listeners) listener({ session: from, key, page, answer: { id, version: page?.version ?? 0, took,
      outcome: { kind: "applied" } }, notice });
  });
  const take = async (key: string, id: number, dto: PageDto): Promise<PageRefusal | null> => {
    const from = session;
    const answer = await graph.submit(key, dto, version(key));
    if (from !== session) return { reason: "not-admitted" };
    if ("refused" in answer) return answer.refused;
    versions.set(key, answer.version);
    mail(from, key, id, { version: answer.version, conflict: false, risk: false, text: { kind: "unchanged", rev: revision(key) } }, true);
    return null;
  };
  const view = async (key: string): Promise<NonNullable<PageMail["page"]>> => {
    const page = await graph.page(key);
    const rev = revision(key);
    return { version: version(key), conflict: false, risk: false,
      disk: page ? { kind: "file", rev } : { kind: "no-file" },
      text: page ? { kind: "page", dto: { ...page, rev } } : { kind: "no-file" } };
  };
  const admit = (id: number) => { nextId = Math.max(nextId, id + 1); };
  const remove = async (path: string) => {
    const from = session;
    await graph.write(path, null);
    if (from !== session) return "refused" as const;
    versions.set(path, version(path) + 1);
    return "applied" as const;
  };
  return {
    async pageWindowReloaded() { session += 1; versions.clear(); graph.reset(); return { session, nextId }; },
    async pageOpen(_session: number, id: number, page: { name: string; kind: "journal" | "page"; path: string | null }) {
      const from = session;
      let path = page.path;
      if (path === null) {
        const resolved = await graph.resolve(page.name, page.kind);
        if (resolved.kind === "alias") return { reason: "alias" as const, owners: resolved.owners };
        path = resolved.id;
      }
      admit(id); mail(from, path, id, await view(path), false); return { key: path, baselineEntry: true };
    },
    async pageSubmit(_session: number, id: number, key: string, dto: PageDto) { admit(id); return take(key, id, dto); },
    // Refused before either submit, so no half of a move is ever written.
    async pageMove(_session: number, id: number, _source: [string, PageDto, number], _receiver: [string, PageDto, number]): Promise<PageRefusal> {
      admit(id);
      return { reason: "failed", message: refusal() };
    },
    async pageDiscard(_session: number, id: number, key: string) {
      const from = session;
      admit(id); mail(from, key, id, await view(key), false); return null;
    },
    // A close is never answered (model upClose), as on the native host.
    async pageClose(_session: number, id: number) { admit(id); return null; },
    // By name: only the name's one claimant, so another file of the same
    // name (a Markdown and an Org page, a day's two journals) is never taken.
    async pageDelete(_session: number, name: string, kind: "journal" | "page", expectedPath?: string) {
      const resolved = await graph.resolve(name, kind);
      if (resolved.kind !== "existing" || resolved.others.length > 0
        || (expectedPath !== undefined && expectedPath !== resolved.id)) return "refused" as const;
      return remove(resolved.id);
    },
    async pageDeleteFile(_session: number, path: string, baseRev: string) {
      const rev = revision(path);
      if (!(await graph.page(path)) || rev !== baseRev || rev !== revision(path)) return "refused" as const;
      return remove(path);
    },
    async pageWait(_session: number, needs: readonly PublishedNeed[]) {
      const published = [];
      for (const { key, version: needed } of needs) {
        const current = versions.get(key);
        if (current !== undefined && current >= needed)
          published.push({ key, version: current, rev: (await graph.page(key)) ? revision(key) : null });
      }
      return published;
    },
    async pageSaveNow() {},
    async pageOwed() { return []; },
    async pageDraftsRetry() {},
    /** The preview's stand-in for the desktop's file watcher, which scripts
     * drive (`scripts/repro-carry-blur.mjs`): another program writes `text`
     * to `path`, the core recaptures it, and the window gets the new text as
     * unanswered page mail, as from a native external change. */
    async externalWrite(path: string, text: string) {
      const from = session;
      await graph.write(path, text);
      if (from !== session) return;
      versions.set(path, version(path) + 1);
      const page = await view(path);
      if (from !== session) return;
      for (const listener of listeners) listener({ session, key: path, page, answer: null, notice });
    },
    async onPageMail(cb: (mail: PageMail) => void) { listeners.add(cb); return () => { listeners.delete(cb); }; },
    async onGraphOpenWaiting() { return () => {}; },
    async onGraphOpenOwnerWaiting() { return () => {}; },
  };
}
