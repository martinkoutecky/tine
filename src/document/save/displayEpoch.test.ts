// R4 / I-20 (og-flow3 finding 2): the save owner's token is exactly the graph
// binding and the page instance. A display-only repaint (typography, journal
// title format, a rename of another page) bumps `graphEpoch` but keeps the same
// graph and the same page, so it must not retire a save already in flight:
// success advances the base revision (else Tine's next save conflicts with its
// own write) and failure keeps the page dirty with its toast (else the edit is
// silently no longer pending). Drives the literal save path: setRaw -> flushPage
// -> backend.savePages.
import { afterEach, beforeAll, beforeEach, expect, it, vi } from "vitest";
import { backend } from "../../backend";
import { initParser } from "../../render/parse";
import { setToasts, toasts } from "../../toasts";
import { setGraphMeta } from "../../graphSession";
import { changeJournalTitleFormat, setTypographyMode } from "../../ui";
import { refreshAfterRename } from "../../graph";
import { loadFeed, resetStore, setRaw } from "../index";
import { flushPage, isConflicted, isDirty } from "./engine";
import type { GraphMeta, PageRead } from "../../types";
import type { SavePagesResult } from "../../backend";

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
afterEach(() => { vi.unstubAllGlobals(); vi.restoreAllMocks(); });

const repaints: [string, () => void][] = [
  ["a typography toggle", () => setTypographyMode("off")],
  ["a journal-title-format change", () => {
    vi.spyOn(backend(), "setJournalTitleFormat").mockReturnValue(new Promise(() => {}));
    changeJournalTitleFormat("yyyy-MM-dd");
  }],
  ["a rename of another page", () => refreshAfterRename("Other", "Renamed")],
];

for (const [label, repaint] of repaints) {
  it(`a save that lands after ${label} advances the base revision`, async () => {
    loadFeed([page()]);
    let disk = "disk-0";
    let release!: () => void;
    const gate = new Promise<void>((resolve) => { release = resolve; });
    const save = vi.spyOn(backend(), "savePages").mockImplementation(async (entries) => {
      if (save.mock.calls.length === 1) await gate;
      if (entries[0].baseRev !== disk)
        return { failed: { index: 0, family: "conflict", diskRev: disk, undoFailed: [] } } as SavePagesResult;
      disk = `disk-${save.mock.calls.length}`;
      return { ok: [disk] };
    });
    setRaw("body", "first edit");
    const first = flushPage("Note");
    await vi.waitFor(() => expect(save).toHaveBeenCalledTimes(1));
    repaint();
    release();
    expect(await first).toBe(true);
    setRaw("body", "second edit");
    expect(await flushPage("Note")).toBe(true);
    expect(save.mock.calls[1][0][0].baseRev).toBe("disk-1");
    expect(isConflicted("Note")).toBe(false);
  });

  it(`a save that fails after ${label} stays pending and is reported`, async () => {
    loadFeed([page()]);
    let release!: () => void;
    const gate = new Promise<void>((resolve) => { release = resolve; });
    const save = vi.spyOn(backend(), "savePages").mockImplementation(async () => {
      await gate;
      return { failed: { index: 0, family: "not-found", undoFailed: [] } } as SavePagesResult;
    });
    setRaw("body", "unsaved edit");
    const first = flushPage("Note");
    await vi.waitFor(() => expect(save).toHaveBeenCalledTimes(1));
    repaint();
    release();
    expect(await first).toBe(false);
    expect(isDirty("Note")).toBe(true);
    expect(toasts().some((toast) => toast.kind === "error" && toast.message.includes("Couldn't save “Note”"))).toBe(true);
  });
}
