// GH #254 family (master 7bd793bd0, og J1): a page load whose replacement the
// working set declines is a refusal every caller must act on. The name slot is
// held by ANOTHER file (a duplicate day or a same-named page opened by path)
// that holds uncommitted input, so the requested file was not installed. Each
// test drives one caller class through the door it calls in production and
// asserts the user-visible harm master's fix names: the refusal reported as
// success, content landing in the second file, or a refresh silently dropped.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { initParser } from "../render/parse";
import { backend } from "../backend";
import { appendFeed, appendToTodayJournal, ensurePageLoaded, loadFeed, pageByName, pinPageWhileDrafting, reloadHlsIfLoaded, resetStore, restoreTodayJournalInFeed, setRaw } from "./index";
import { baseRevFor } from "./save/engine";
import { registerPaneRouteProvider } from "./workingSet";
import { doc } from "./model";
import { endEdit, startEditing } from "../editorController";
import { journalTitle, appNow } from "../journal";
import { setToasts, toasts } from "../toasts";
import type { BlockDto, PageDto, PageKind } from "../types";

let serial = 0;
const block = (raw: string): BlockDto => ({ id: `rr-${++serial}`, raw, collapsed: false, children: [] });
const file = (name: string, id: string, raws: string[], kind: PageKind = "journal"): PageDto & { id: string; rev: string } => ({
  id, name, title: name, kind, pre_block: null, rev: `rev-${++serial}`, blocks: raws.map(block),
});
const raws = (name: string) => pageByName(name)?.roots.map((id) => doc.byId[id].raw) ?? [];
const refusalToast = () => toasts().find((toast) => toast.kind === "error" && toast.message.includes("pages/stray.md"));

let unpin: (() => void) | null = null;
beforeAll(() => initParser());
beforeEach(() => {
  serial = 0;
  resetStore();
  setToasts([]);
  vi.spyOn(backend(), "savePages").mockImplementation(async (entries) => ({ ok: entries.map((_, i) => `saved-${i}`) }));
});
afterEach(() => {
  unpin?.(); unpin = null;
  endEdit("graph-switch");
  registerPaneRouteProvider(() => []);
  vi.restoreAllMocks();
});

/** A second file holding `name`'s slot, with each kind of uncommitted input. */
const holds: Array<[string, (name: string) => void]> = [
  ["a draft pin", (name) => { unpin = pinPageWhileDrafting(() => name); }],
  ["an active block editor", (name) => startEditing(pageByName(name)!.roots[0], 0)],
  ["an unsaved edit", (name) => setRaw(pageByName(name)!.roots[0], "stray typed")],
];
/** Install `dto` into an empty or safe slot (setup, not the property under test). */
function install(dto: PageDto & { id: string }) {
  ensurePageLoaded(dto);
  expect(pageByName(dto.name)?.id).toBe(dto.id);
}
function strayHolding(name: string, hold: (name: string) => void) {
  install(file(name, "pages/stray.md", ["stray text"]));
  hold(name);
}

describe.each(holds)("feed publication follows installation (slot held by %s)", (_label, hold) => {
  it("a refresh keeps the old feed instead of publishing the stray as the journal", () => {
    const day = "Sep 1st, 2026";
    expect(loadFeed([file("Aug 31st, 2026", "journals/2026_08_31.md", ["older"])])).toBe("published");
    strayHolding(day, hold);
    expect(loadFeed([file(day, "journals/2026_09_01.md", ["canonical"])], { endEdit: false }))
      .toMatchObject({ page: day, holder: "pages/stray.md", requested: "journals/2026_09_01.md" });
    expect(doc.feed).toEqual(["Aug 31st, 2026"]);
    expect(pageByName(day)!.id).toBe("pages/stray.md");
  });

  it("a rollover and infinite scroll leave the refused day out", () => {
    const day = "Sep 1st, 2026";
    expect(loadFeed([file("Sep 2nd, 2026", "journals/2026_09_02.md", ["newer"])])).toBe("published");
    strayHolding(day, hold);
    const appended = appendFeed([file(day, "journals/2026_09_01.md", ["canonical"])]);
    expect(doc.feed).toEqual(["Sep 2nd, 2026"]);
    expect(appended).toMatchObject([{ page: day, holder: "pages/stray.md" }]);
    expect(loadFeed([file(day, "journals/2026_09_01.md", ["canonical"])], { endEdit: false, preserveExisting: true }))
      .toMatchObject({ page: day, holder: "pages/stray.md" });
    expect(doc.feed).toEqual(["Sep 2nd, 2026"]);
  });

  it("restoring today after a delete does not publish a stray holding today's name", () => {
    const today = journalTitle(appNow());
    expect(loadFeed([file("Aug 31st, 2026", "journals/2026_08_31.md", ["older"])])).toBe("published");
    strayHolding(today, hold);
    const restored = restoreTodayJournalInFeed();
    expect(doc.feed).toEqual(["Aug 31st, 2026"]);
    expect(restored).toMatchObject({ page: today, holder: "pages/stray.md" });
  });
});

describe("capture never lands in a second file holding the destination's name", () => {
  it("stops when a stray took the slot while today's file was being read", async () => {
    const today = journalTitle(appNow());
    expect(loadFeed([file("Aug 31st, 2026", "journals/2026_08_31.md", ["older"])])).toBe("published");
    let finish!: (page: PageDto) => void;
    vi.spyOn(backend(), "getPage").mockImplementation(() => new Promise((resolve) => { finish = resolve as never; }));
    const save = vi.spyOn(backend(), "savePages");
    const capturing = appendToTodayJournal("- captured thought");
    await vi.waitFor(() => expect(finish).toBeTypeOf("function"));
    strayHolding(today, holds[0][1]);
    finish(file(today, "journals/today.md", []));
    expect(await capturing).toBe(false);
    expect(JSON.stringify(save.mock.calls)).not.toContain("captured thought");
    expect(raws(today)).toEqual(["stray text"]);
    expect(refusalToast()?.message).toContain("journals/today.md");
  });

  it("refuses when a second file already holds today's name, naming both files", async () => {
    const today = journalTitle(appNow());
    expect(loadFeed([file("Aug 31st, 2026", "journals/2026_08_31.md", ["older"])])).toBe("published");
    install(file(today, "pages/stray.md", ["stray text"]));
    vi.spyOn(backend(), "getPage").mockResolvedValue(file(today, "journals/today.md", []) as never);
    const save = vi.spyOn(backend(), "savePages");
    expect(await appendToTodayJournal("- captured thought")).toBe(false);
    expect(JSON.stringify(save.mock.calls)).not.toContain("captured thought");
    expect(refusalToast()?.message).toContain("journals/today.md");
  });
});

describe("the PDF-notes refresh never drops a highlight write it declined", () => {
  it("defers while a block on the notes page is edited, then applies disk content and baseline", async () => {
    const notes = "hls__paper";
    expect(loadFeed([file("Aug 31st, 2026", "journals/2026_08_31.md", ["older"])])).toBe("published");
    install(file(notes, "pages/hls__paper.md", ["old note"], "page"));
    startEditing(pageByName(notes)!.roots[0], 0);
    const disk = file(notes, "pages/hls__paper.md", ["old note", "new highlight"], "page");
    vi.spyOn(backend(), "getPage").mockResolvedValue(disk as never);
    const applied = await reloadHlsIfLoaded(notes);
    expect(raws(notes)).toEqual(["old note"]); // the caret is never stolen
    endEdit("blur");
    await vi.waitFor(() => expect(raws(notes)).toEqual(["old note", "new highlight"]));
    expect(baseRevFor(notes)).toBe(disk.rev);
    expect(applied).toBe(false);
  });

  it("applies at once when the notes page is idle", async () => {
    const notes = "hls__paper";
    install(file(notes, "pages/hls__paper.md", ["old note"], "page"));
    vi.spyOn(backend(), "getPage").mockResolvedValue(file(notes, "pages/hls__paper.md", ["old note", "hl"], "page") as never);
    expect(await reloadHlsIfLoaded(notes)).toBe(true);
    expect(raws(notes)).toEqual(["old note", "hl"]);
  });
});

describe("ensurePageLoaded answers whether the requested file holds the slot", () => {
  it.each(holds)("refuses, typed, when another file holding %s occupies the name", (_label, hold) => {
    strayHolding("P", hold);
    const refusal = ensurePageLoaded(file("P", "pages/P.md", ["requested"], "page"));
    expect(refusal).toMatchObject({ page: "P", holder: "pages/stray.md", requested: "pages/P.md" });
    expect(pageByName("P")!.id).toBe("pages/stray.md");
  });

  it("is not a refusal when the incumbent is the requested file itself", () => {
    install(file("P", "pages/P.md", ["mine"], "page"));
    setRaw(pageByName("P")!.roots[0], "typed");
    install(file("P", "pages/P.md", ["disk"], "page"));
    expect(raws("P")).toEqual(["typed"]);
  });
});
