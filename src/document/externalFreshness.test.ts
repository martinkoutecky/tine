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
import { STALE_VERSION } from "./host/protocol";
import { answerOpensFromDocument } from "./host/documentHost.test.support";
import { bindTestHost, mailPage, submittedPages, type TestHost } from "./host/wiring.test.support";

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
let host: TestHost;
beforeAll(() => initParser());
beforeEach(async () => {
  serial = 0; resetStore(); setToasts([]); disk = {}; reads = []; feedRestarts = 0;
  host = await bindTestHost();
  answerOpensFromDocument(host);
  vi.spyOn(backend(), "getPage").mockImplementation(async (name) => { reads.push(name); return (disk[name] ?? null) as never; });
  vi.spyOn(backend(), "setAppBool").mockResolvedValue(undefined);
  installExternalChangeUiHandler(() => ({
    pageOpen: (name) => name === "Open", journalsOpen: true, leaveRemovedPage: () => {}, restartJournalFeed: () => { feedRestarts++; },
  }));
});
afterEach(() => { setConflictPolicyAlwaysAsk(false); resetStore(); vi.restoreAllMocks(); });

/** `name` edited to `raw`: the host took the input, then saw the file change
 * under it and mailed the conflict (the host, not the window, detects it). */
async function editedThenConflicted(name: string, raw: string): Promise<void> {
  const admitted: Array<{ id: number; key: string }> = [];
  const submit = vi.spyOn(backend(), "pageSubmit").mockImplementation(async (_session, id, key) => { admitted.push({ id, key }); return null; });
  setRaw(pageByName(name)!.roots[0], raw);
  void flushPage(name);
  await vi.waitFor(() => expect(admitted).toHaveLength(1));
  const { id, key } = admitted[0];
  host.deliver({ key, answer: { id, version: 50, took: true, outcome: { kind: "applied" } }, notice: { conflictReported: true },
    page: { version: 50, conflict: true, risk: false, disk: { kind: "file", rev: "theirs-rev" }, text: { kind: "unchanged" } } });
  expect(isConflicted(name)).toBe(true);
  submit.mockRestore();
}

describe("a checkout-sized batch (graph-changed-bulk)", () => {
  it("reads only loaded or shown pages, moves revisions once, restarts the feed once and says so once", async () => {
    loadFeed([page("Jan 1st, 2026", ["j"])]);
    ensurePageLoaded(page("Open", ["old open"]));
    ensurePageLoaded(page("Dirty", ["old dirty"]));
    await editedThenConflicted("Dirty", "mine");
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
    await editedThenConflicted("Dirty", "mine");
    const save = vi.spyOn(backend(), "pageSubmit");
    disk = { Open: page("Open", ["new"]), Dirty: page("Dirty", ["theirs"]) };
    setConflictPolicyAlwaysAsk(true);
    await applyGraphChange(changed("Open"));
    dismissHeldExternalChange("Open");
    expect(heldExternalChangeFor("Open")).toBe(false);
    expect(raws("Open")).toEqual(["old"]);
    await applyGraphChange(changed("Dirty"));
    expect(heldExternalChangeFor("Dirty")).toBe(false);
    expect(isConflicted("Dirty")).toBe(true);
    expect(raws("Dirty")).toEqual(["mine"]);
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
  /** Winner's file on disk, as the host reads it at Open: the hydrated content at the resolved revision. */
  function diskHolds(dto: PageDto & { id: string; rev: string }) {
    vi.spyOn(backend(), "pageOpen").mockImplementation(async (_session, id, request) => {
      const key = request.path ?? dto.id;
      queueMicrotask(() => host.deliver({ key, page: mailPage(7, dto), answer: { id, version: 7, took: false, outcome: { kind: "applied" } } }));
      return { key, baselineEntry: true };
    });
  }

  it("a same-content hydration adopts the newer disk revision, so the next edit is not a false conflict", async () => {
    loadFeed([page("Jan 1st, 2026", ["j"])]);
    ensurePageLoaded(page("Winner", ["winner content"]));
    const resolved = { ...page("Winner", ["winner content"]), rev: "resolved-winner-rev" };
    ensurePageLoaded(resolved);
    diskHolds(resolved);
    const save = vi.spyOn(backend(), "pageSubmit");
    setRaw(pageByName("Winner")!.roots[0], "an ordinary edit");
    expect(await flushPage("Winner")).toBe(true);
    expect(submittedPages(save, "Winner").map((dto) => dto.blocks.map((b) => b.raw))).toEqual([["an ordinary edit"]]);
    // The Open granted the host's version from the hydrated baseline: the edit is not sent stale.
    expect(save.mock.calls[0][4]).not.toBe(STALE_VERSION);
    expect(isConflicted("Winner")).toBe(false);
  });

  it("without the hydration the same edit is sent stale (the property above is not vacuous)", async () => {
    loadFeed([page("Jan 1st, 2026", ["j"])]);
    ensurePageLoaded(page("Winner", ["winner content"]));
    diskHolds({ ...page("Winner", ["winner content"]), rev: "resolved-winner-rev" });
    const save = vi.spyOn(backend(), "pageSubmit");
    setRaw(pageByName("Winner")!.roots[0], "an ordinary edit");
    await flushPage("Winner");
    expect(save.mock.calls[0][4]).toBe(STALE_VERSION);
  });
});
