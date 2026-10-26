// R4 / I-20 (og-flow3 finding 2): the save owner's token is exactly the graph
// binding and the page instance. A display-only repaint (typography, journal
// title format, a rename of another page) bumps `graphEpoch` but keeps the same
// graph and the same page, so it must not retire a request already in flight:
// its answer still advances the page's base version (else Tine's next send
// would be stale against its own write) and a host save failure keeps the page
// unsaved with its toast (else the edit is silently no longer pending). Drives
// the literal path: setRaw -> flushPage -> the host's pageSubmit, answered as
// page mail after the repaint (STEP3 §4).
import { afterEach, beforeAll, beforeEach, expect, it, vi, type MockInstance } from "vitest";
import { backend, type Backend } from "../../backend";
import { initParser } from "../../render/parse";
import { setToasts, toasts } from "../../toasts";
import { setGraphMeta } from "../../graphSession";
import { changeJournalTitleFormat, setTypographyMode } from "../../ui";
import { refreshAfterRename } from "../../graph";
import { startEditing } from "../../editorController";
import { flushPage, isConflicted, loadFeed, resetStore, setRaw, unsavedDrafts } from "../index";
import { bindTestHost, submittedPages, type TestHost } from "../host/wiring.test.support";
import type { GraphMeta, PageRead } from "../../types";

const KEY = "pages/Note.md";
const page = (): PageRead => ({
  id: "pages/Note.md", name: "Note", title: "Note", kind: "page", pre_block: null, rev: "disk-0",
  blocks: [{ id: "body", raw: "original", collapsed: false, children: [] }],
});

const store = new Map<string, string>();
beforeAll(() => initParser());
beforeEach(() => {
  store.clear();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => store.get(key) ?? null,
    setItem: (key: string, value: string) => void store.set(key, value),
    removeItem: (key: string) => void store.delete(key),
  });
  resetStore();
  setToasts([]);
  setGraphMeta({ root: "/graph", journal_page_title_format: "MMM do, yyyy" } as GraphMeta);
});
afterEach(() => { vi.unstubAllGlobals(); vi.restoreAllMocks(); resetStore(); });

/** Note loaded and bound, the user typing in it; its first submit is held
 * until `release()`, then taken by the host. */
async function heldFirstSubmit(): Promise<{ host: TestHost; submit: MockInstance<Backend["pageSubmit"]>; release: () => void }> {
  loadFeed([page()]);
  const host = await bindTestHost();
  const api = backend();
  const take = api.pageSubmit.bind(api);
  let release!: () => void;
  const gate = new Promise<void>((resolve) => { release = resolve; });
  const submit = vi.spyOn(api, "pageSubmit").mockImplementationOnce(async (...args) => { await gate; return take(...args); });
  startEditing("body");
  return { host, submit, release };
}

const repaints: [string, () => void][] = [
  ["a typography toggle", () => setTypographyMode("off")],
  ["a journal-title-format change", () => {
    vi.spyOn(backend(), "setJournalTitleFormat").mockReturnValue(new Promise(() => {}));
    changeJournalTitleFormat("yyyy-MM-dd");
  }],
  ["a rename of another page", () => refreshAfterRename("Other", "Renamed")],
];

for (const [label, repaint] of repaints) {
  it(`a save that lands after ${label} advances the base version`, async () => {
    const { submit, release } = await heldFirstSubmit();
    const answered: number[] = [];
    const stop = await backend().onPageMail((mail) => {
      if (mail.key === KEY && mail.answer?.took) answered.push(mail.answer.version);
    });
    setRaw("body", "first edit");
    const first = flushPage("Note");
    await vi.waitFor(() => expect(submit).toHaveBeenCalledTimes(1));
    repaint();
    release();
    expect(await first).toBe(true);
    setRaw("body", "second edit");
    expect(await flushPage("Note")).toBe(true);
    expect(submittedPages(submit).map((dto) => dto.blocks[0].raw)).toEqual(["first edit", "second edit"]);
    // The second send's base is the version the first answer left.
    expect(submit.mock.calls[1][4]).toBe(answered[0]);
    expect(isConflicted("Note")).toBe(false);
    stop();
  });

  it(`a save that fails after ${label} stays pending and is reported`, async () => {
    const { host, submit, release } = await heldFirstSubmit();
    // The host took the input; every save attempt fails (three, then the notice).
    vi.spyOn(backend(), "pageOwed").mockResolvedValue([{ key: KEY, version: 2 }]);
    vi.spyOn(backend(), "pageWait").mockResolvedValue(false);
    vi.spyOn(backend(), "pageSaveNow").mockImplementation(async () => {
      host.notice(KEY, { failures: 3, saveError: true }, { version: 2, risk: true });
    });
    setRaw("body", "unsaved edit");
    const first = flushPage("Note");
    await vi.waitFor(() => expect(submit).toHaveBeenCalledTimes(1));
    repaint();
    release();
    expect(await first).toBe(false);
    expect(unsavedDrafts().map((draft) => [draft.name, draft.state])).toEqual([["Note", "Not saved"]]);
    await vi.waitFor(() => expect(toasts().some((toast) => toast.kind === "error"
      && toast.message.includes("Couldn't save “Note”"))).toBe(true));
  });
}
