import type { PageMail } from "./document";
import type { PageDto, ResolvedPage } from "./types";

/** The browser mock's page host: one window, every request taken and
 * published at once, nothing written (the mock graph never changes on save).
 * Answers arrive as page mail after the command returns, as from the native
 * host. Tests that need refusals or held answers use the host fakes instead. */
export function mockPageHost(read: (path: string) => PageDto | null,
  resolve: (name: string, kind: "journal" | "page") => ResolvedPage) {
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
  const take = (key: string, id: number) => {
    const version = (versions.get(key) ?? 1) + 1;
    versions.set(key, version);
    mail(key, id, { version, conflict: false, risk: false, text: { kind: "unchanged" } }, true);
  };
  const view = (key: string): NonNullable<PageMail["page"]> => {
    const page = read(key);
    return { version: versions.get(key) ?? 1, conflict: false, risk: false,
      disk: page ? { kind: "file", rev: page.rev ?? "mock-rev" } : { kind: "no-file" },
      text: page ? { kind: "page", dto: { ...page, rev: page.rev ?? "mock-rev" } } : { kind: "no-file" } };
  };
  const admit = (id: number) => { nextId = Math.max(nextId, id + 1); };
  return {
    async pageWindowReloaded() { session += 1; return { session, nextId }; },
    async pageOpen(_session: number, id: number, page: { name: string; kind: "journal" | "page"; path: string | null }) {
      let path = page.path;
      if (path === null) {
        const resolved = resolve(page.name, page.kind);
        if (resolved.kind === "alias") return { reason: "alias" as const, owners: resolved.owners };
        path = resolved.id;
      }
      admit(id); mail(path, id, view(path), false); return { key: path, baselineEntry: true };
    },
    async pageSubmit(_session: number, id: number, key: string) { admit(id); take(key, id); return null; },
    async pageMove(_session: number, id: number, source: [string, PageDto, number], receiver: [string, PageDto, number]) {
      admit(id); take(source[0], id); take(receiver[0], id); return null;
    },
    async pageDiscard(_session: number, id: number, key: string) { admit(id); mail(key, id, view(key), false); return null; },
    // A close is never answered (model upClose), as on the native host.
    async pageClose(_session: number, id: number) { admit(id); return null; },
    async pageDelete() { return "applied" as const; },
    async pageWait() { return true; },
    async pageSaveNow() {},
    async pageOwed() { return []; },
    async pageDraftsRetry() {},
    async onPageMail(cb: (mail: PageMail) => void) { listeners.add(cb); return () => { listeners.delete(cb); }; },
    async onGraphOpenWaiting() { return () => {}; },
  };
}
