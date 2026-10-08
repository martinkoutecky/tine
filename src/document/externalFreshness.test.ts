// Family 10 (master 1229f32fb, Concord P5): a checkout-sized watcher batch is
// applied once, and "always ask" holds the one silent reload for the page's
// bar. Driven through the real entry points App wires to the native events.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { initParser } from "../render/parse";
import { backend, type GraphChange } from "../backend";
import { applyGraphChange, applyGraphChangesBulk, ensurePageLoaded, flushPage, installExternalChangeUiHandler, isConflicted, loadFeed, pageByName, resetStore, setRaw } from "./index";
import { doc } from "./model";
import { dataRev } from "../graphSession";
import { setToasts, toasts } from "../toasts";
import { applyHeldExternalChange, dismissHeldExternalChange, heldExternalChangeFor, setConflictPolicyAlwaysAsk } from "../conflictPolicy";
import type { BlockDto, PageDto } from "../types";

let serial = 0;
const block = (raw: string): BlockDto => ({ id: `xf-${++serial}`, raw, collapsed: false, children: [] });
const page = (name: string, raws: string[]): PageDto & { id: string; rev: string } => ({
  id: `pages/${name}.md`, name, title: name, kind: "page", pre_block: null, rev: `rev-${++serial}`, blocks: raws.map(block),
});
const raws = (name: string) => pageByName(name)?.roots.map((id) => doc.byId[id].raw) ?? [];
const changed = (name: string, kind: "page" | "journal" = "page"): GraphChange => ({ name, kind, created: false, removed: false });

let disk: Record<string, PageDto & { id: string }>;
let reads: string[];
let feedRestarts: number;
beforeAll(() => initParser());
beforeEach(() => {
  serial = 0; resetStore(); setToasts([]); disk = {}; reads = []; feedRestarts = 0;
  vi.spyOn(backend(), "getPage").mockImplementation(async (name) => { reads.push(name); return (disk[name] ?? null) as never; });
  vi.spyOn(backend(), "setAppBool").mockResolvedValue(undefined);
  installExternalChangeUiHandler(() => ({
    pageOpen: (name) => name === "Open", journalsOpen: true, leaveRemovedPage: () => {}, restartJournalFeed: () => { feedRestarts++; },
  }));
});
afterEach(() => { setConflictPolicyAlwaysAsk(false); vi.restoreAllMocks(); });

describe("a checkout-sized batch (graph-changed-bulk)", () => {
  it("reads only loaded or shown pages, moves revisions once, restarts the feed once and says so once", async () => {
    loadFeed([page("Jan 1st, 2026", ["j"])]);
    ensurePageLoaded(page("Open", ["old open"]));
    ensurePageLoaded(page("Dirty", ["old dirty"]));
    setRaw(pageByName("Dirty")!.roots[0], "mine");
    disk = { Open: page("Open", ["new open"]), Dirty: page("Dirty", ["theirs"]), "Jan 1st, 2026": page("Jan 1st, 2026", ["j2"]) };
    const changes = [changed("Open"), changed("Dirty"), changed("Jan 1st, 2026", "journal"), changed("Jan 2nd, 2026", "journal"),
      ...Array.from({ length: 36 }, (_, i) => changed(`Elsewhere ${i}`))];
    vi.spyOn(backend(), "getPageByPath").mockImplementation(async (path) => (Object.values(disk).find((p) => p.id === path) ?? null) as never);
    const before = dataRev();
    await applyGraphChangesBulk({ changes });
    expect(dataRev()).toBe(before + 1);
    expect(reads.filter((name) => name.startsWith("Elsewhere"))).toEqual([]);
    expect(raws("Open")).toEqual(["new open"]);
    expect(raws("Dirty")).toEqual(["mine"]);
    expect(isConflicted("Dirty")).toBe(true);
    expect(feedRestarts).toBe(1);
    expect(toasts().map((t) => t.message)).toEqual(["40 pages updated externally · 1 conflict to review"]);
  });
});

describe("always ask", () => {
  it("holds the silent reload of a loaded clean page until Reload from disk", async () => {
    ensurePageLoaded(page("Open", ["old"]));
    disk = { Open: page("Open", ["new"]) };
    setConflictPolicyAlwaysAsk(true);
    await applyGraphChange(changed("Open"));
    expect(raws("Open")).toEqual(["old"]);
    expect(heldExternalChangeFor("Open")).toBe(true);
    applyHeldExternalChange("Open");
    await vi.waitFor(() => expect(raws("Open")).toEqual(["new"]));
    expect(heldExternalChangeFor("Open")).toBe(false);
  });

  it("Keep mine writes nothing and a dirty page still takes the conflict path", async () => {
    ensurePageLoaded(page("Open", ["old"]));
    ensurePageLoaded(page("Dirty", ["old"]));
    setRaw(pageByName("Dirty")!.roots[0], "mine");
    const save = vi.spyOn(backend(), "savePages");
    disk = { Open: page("Open", ["new"]), Dirty: page("Dirty", ["theirs"]) };
    setConflictPolicyAlwaysAsk(true);
    await applyGraphChange(changed("Open"));
    dismissHeldExternalChange("Open");
    expect(heldExternalChangeFor("Open")).toBe(false);
    expect(raws("Open")).toEqual(["old"]);
    await applyGraphChange(changed("Dirty"));
    expect(heldExternalChangeFor("Dirty")).toBe(false);
    expect(isConflicted("Dirty")).toBe(true);
    expect(save).not.toHaveBeenCalled();
  });

  it("off by default: the clean page reloads silently", async () => {
    ensurePageLoaded(page("Open", ["old"]));
    disk = { Open: page("Open", ["new"]) };
    await applyGraphChange(changed("Open"));
    expect(raws("Open")).toEqual(["new"]);
    expect(heldExternalChangeFor("Open")).toBe(false);
  });
});

describe("Concord winner hydration (master ba80a151e9a2)", () => {
  it("a same-content hydration adopts the newer disk revision, so the next edit is not a false conflict", async () => {
    loadFeed([page("Jan 1st, 2026", ["j"])]);
    ensurePageLoaded(page("Winner", ["winner content"]));
    ensurePageLoaded({ ...page("Winner", ["winner content"]), rev: "resolved-winner-rev" });
    const save = vi.spyOn(backend(), "savePages").mockImplementation(async (entries) => ({ ok: entries.map(() => "next") }));
    setRaw(pageByName("Winner")!.roots[0], "an ordinary edit");
    expect(await flushPage("Winner")).toBe(true);
    expect(save.mock.calls[0][0][0].baseRev).toBe("resolved-winner-rev");
  });
});
