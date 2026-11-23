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
 * use the host fakes instead. */
export function mockPageHost(graph: PageHostGraph) {
  const listeners = new Set<(mail: PageMail) => void>();
  const versions = new Map<string, number>();
  const notice = { failures: 0, saveError: false, operation: null, osError: null, draftError: false, dropped: false, conflictReported: false, custodyError: false,
    indexError: false, observeError: false, twin: null };
  let nextId = 1;
  let session = 1;
  const mail = (key: string, id: number, page: PageMail["page"], took: boolean) => queueMicrotask(() => {
    for (const listener of listeners) listener({ session, key, page, answer: { id, version: page?.version ?? 0, took,
      outcome: { kind: "applied" } }, notice });
  });
  const take = async (key: string, id: number, dto: PageDto): Promise<PageRefusal | null> => {
    const answer = await graph.submit(key, dto, versions.get(key) ?? 1);
    if ("refused" in answer) return answer.refused;
    versions.set(key, answer.version);
    mail(key, id, { version: answer.version, conflict: false, risk: false, text: { kind: "unchanged" } }, true);
    return null;
  };
  const view = async (key: string): Promise<NonNullable<PageMail["page"]>> => {
    const page = await graph.page(key);
    const rev = page?.rev ?? "mock-rev";
    return { version: versions.get(key) ?? 1, conflict: false, risk: false,
      disk: page ? { kind: "file", rev } : { kind: "no-file" },
      text: page ? { kind: "page", dto: { ...page, rev } } : { kind: "no-file" } };
  };
  const admit = (id: number) => { nextId = Math.max(nextId, id + 1); };
  const remove = async (path: string) => {
    await graph.write(path, null);
    versions.set(path, (versions.get(path) ?? 1) + 1);
    return "applied" as const;
  };
  return {
    async pageWindowReloaded() { session += 1; versions.clear(); graph.reset(); return { session, nextId }; },
    async pageOpen(_session: number, id: number, page: { name: string; kind: "journal" | "page"; path: string | null }) {
      let path = page.path;
      if (path === null) {
        const resolved = await graph.resolve(page.name, page.kind);
        if (resolved.kind === "alias") return { reason: "alias" as const, owners: resolved.owners };
        path = resolved.id;
      }
      admit(id); mail(path, id, await view(path), false); return { key: path, baselineEntry: true };
    },
    async pageSubmit(_session: number, id: number, key: string, dto: PageDto) { admit(id); return take(key, id, dto); },
    async pageMove(_session: number, id: number, source: [string, PageDto, number], receiver: [string, PageDto, number]) {
      admit(id);
      return (await take(source[0], id, source[1])) ?? take(receiver[0], id, receiver[1]);
    },
    async pageDiscard(_session: number, id: number, key: string) { admit(id); mail(key, id, await view(key), false); return null; },
    // A close is never answered (model upClose), as on the native host.
    async pageClose(_session: number, id: number) { admit(id); return null; },
    async pageDelete(_session: number, name: string, kind: "journal" | "page", expectedPath?: string) {
      const resolved = await graph.resolve(name, kind);
      if (resolved.kind !== "existing" || (expectedPath !== undefined && expectedPath !== resolved.id)) return "refused" as const;
      return remove(resolved.id);
    },
    async pageDeleteFile(_session: number, path: string, baseRev: string) {
      const page = await graph.page(path);
      if (!page || page.rev !== baseRev) return "refused" as const;
      return remove(path);
    },
    async pageWait(_session: number, needs: readonly PublishedNeed[]) {
      const published = [];
      for (const { key, version } of needs) {
        const current = versions.get(key);
        if (current !== undefined && current >= version) published.push({ key, version: current, rev: (await graph.page(key))?.rev ?? null });
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
      await graph.write(path, text);
      versions.set(path, (versions.get(path) ?? 1) + 1);
      const page = await view(path);
      for (const listener of listeners) listener({ session, key: path, page, answer: null, notice });
    },
    async onPageMail(cb: (mail: PageMail) => void) { listeners.add(cb); return () => { listeners.delete(cb); }; },
    async onGraphOpenWaiting() { return () => {}; },
    async onGraphOpenOwnerWaiting() { return () => {}; },
  };
}
