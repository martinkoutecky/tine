import type { PageMail, PageRefusal, PublishedNeed, PublishedRevision } from "./document";
import { RetiredGraph, type MockGraph, type MockGraphSession } from "./mockGraph";
import type { PageDto } from "./types";

/** The graph a mock page host writes: the browser core's memory graph (`mockGraph.ts`). */
export type PageHostGraph = Pick<MockGraph, "current" | "reset">;

/** The browser mock's page host: one window, every request answered at once.
 * A submit writes through the browser core (ADR 0081), so a later read or Open
 * answers the saved text, until a window reload reopens the fixture graph. The
 * host keeps only the mail's listeners. Answers arrive as page mail after the command
 * returns, as from the native host. Tests that need refusals or held answers
 * use the host fakes instead.
 *
 * - A file's revision is the session and the core's version of the file, read
 *   when it is named (the core's page reads carry none). Open, submit mail, the
 *   publication wait and the exact deletion all name it; the core refuses a
 *   deletion typed on a version it has moved past.
 * - A request captures its graph session when it starts and makes every graph
 *   call through it: after a window reload, a pending one refuses and sends no mail.
 * - The preview writes one page per request (ADR 0081 Decision 4): a
 *   cross-page move is refused before either endpoint is written. */
export function mockPageHost(graph: PageHostGraph, refusal: () => string) {
  const listeners = new Set<(mail: PageMail) => void>();
  const notice = { failures: 0, saveError: false, operation: null, osError: null, draftError: false, dropped: false, conflictReported: false, custodyError: false,
    indexError: false, observeError: false, twin: null };
  let nextId = 1;
  const NOT_ADMITTED: PageRefusal = { reason: "not-admitted" };
  const revision = (s: MockGraphSession, version: number) => `preview-${s.session}-${version}`;
  /** Run a request of session `s`; `outcome` once a reload retired `s`. */
  const request = <T>(outcome: T, run: (s: MockGraphSession) => Promise<T>) =>
    run(graph.current()).catch((error: unknown) => { if (error instanceof RetiredGraph) return outcome; throw error; });
  /** Send session `s`'s mail (answering `id`, or unanswered): whether it went, as it does unless a reload retired `s`. */
  const send = (s: MockGraphSession, key: string, page: NonNullable<PageMail["page"]>, id: number | null, took = false) =>
    new Promise<boolean>(resolve => queueMicrotask(() => {
      if (s.retired) return resolve(false);
      const answer = id === null ? null : { id, version: page.version, took, outcome: { kind: "applied" as const } };
      for (const listener of listeners) listener({ session: s.session, key, page, answer, notice });
      resolve(true);
    }));
  const view = async (s: MockGraphSession, key: string): Promise<NonNullable<PageMail["page"]>> => {
    const { page, version } = await s.page(key);
    const rev = revision(s, version);
    return { version, conflict: false, risk: false,
      disk: page ? { kind: "file", rev } : { kind: "no-file" },
      text: page ? { kind: "page", dto: { ...page, rev } } : { kind: "no-file" } };
  };
  const admit = (id: number) => { nextId = Math.max(nextId, id + 1); };
  const deleted = (ok: boolean) => ok ? "applied" as const : "refused" as const;
  return {
    async pageWindowReloaded() { graph.reset(); return { session: graph.current().session, nextId }; },
    async pageOpen(_session: number, id: number, page: { name: string; kind: "journal" | "page"; path: string | null }) {
      admit(id);
      return request(NOT_ADMITTED, async (s) => {
        let path = page.path;
        if (path === null) {
          const resolved = await s.resolve(page.name, page.kind);
          if (resolved.kind === "alias") return { reason: "alias" as const, owners: resolved.owners };
          path = resolved.id;
        }
        return await send(s, path, await view(s, path), id) ? { key: path, baselineEntry: true } : NOT_ADMITTED;
      });
    },
    async pageSubmit(_session: number, id: number, key: string, dto: PageDto) {
      admit(id);
      return request(NOT_ADMITTED, async (s) => {
        const took = await s.submit(key, dto);
        if ("refused" in took) return took.refused;
        const rev = revision(s, took.version);
        return await send(s, key, { version: took.version, conflict: false, risk: false, text: { kind: "unchanged", rev } }, id, true)
          ? null : NOT_ADMITTED;
      });
    },
    // Refused before either submit, so no half of a move is ever written.
    async pageMove(_session: number, id: number, _source: [string, PageDto, number], _receiver: [string, PageDto, number]): Promise<PageRefusal> {
      admit(id);
      return { reason: "failed", message: refusal() };
    },
    async pageDiscard(_session: number, id: number, key: string) {
      admit(id);
      return request(NOT_ADMITTED, async (s) => await send(s, key, await view(s, key), id) ? null : NOT_ADMITTED);
    },
    // A close is never answered (model upClose), as on the native host.
    async pageClose(_session: number, id: number) { admit(id); return null; },
    // The core resolves and deletes in one call: only the name's one claimant,
    // so another file of the same name (a Markdown and an Org page, a day's
    // two journals) is never taken.
    async pageDelete(_session: number, name: string, kind: "journal" | "page", expectedPath?: string) {
      return request("refused" as const, async (s) => deleted(await s.deleteName(name, kind, expectedPath)));
    },
    async pageDeleteFile(_session: number, path: string, baseRev: string) {
      const [, session, version] = /^preview-(\d+)-(\d+)$/.exec(baseRev) ?? [];
      return request("refused" as const, async (s) =>
        Number(session) === s.session ? deleted(await s.deleteFile(path, Number(version))) : "refused");
    },
    async pageWait(_session: number, needs: readonly PublishedNeed[]) {
      return request<PublishedRevision[] | false>(false, async (s) => {
        const published: PublishedRevision[] = [];
        for (const { key, version: needed } of needs) {
          const { page, version } = await s.page(key);
          if (version >= needed) published.push({ key, version, rev: page ? revision(s, version) : null });
        }
        return published;
      });
    },
    async pageSaveNow() {},
    async pageOwed() { return []; },
    /** The preview runs no renames, so it keeps no originals. */
    async pageKeptOriginals() { return []; },
    async pageKeptText() { return null; },
    async pageRestoreKept() { return { kind: "refused" as const, reason: "notKept" as const }; },
    async pageDraftsRetry() {},
    /** The preview's stand-in for the desktop's file watcher, which scripts
     * drive (`scripts/repro-carry-blur.mjs`): another program writes `text`
     * to `path`, the core recaptures it, and the window gets the new text as
     * unanswered page mail, as from a native external change. */
    async externalWrite(path: string, text: string) {
      await request(false, async (s) => { await s.write(path, text); return send(s, path, await view(s, path), null); });
    },
    async onPageMail(cb: (mail: PageMail) => void) { listeners.add(cb); return () => { listeners.delete(cb); }; },
    async onGraphOpenWaiting() { return () => {}; },
    async onGraphOpenOwnerWaiting() { return () => {}; },
  };
}
