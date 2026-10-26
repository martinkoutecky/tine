// Save review regressions that survive the page host (STEP3 §4, §8, §12):
// Use disk after a cross-page move, and the visible refusals of a page with no
// file (a twin, a failed resolution). The save-group cases (B-1, S-1, S-2,
// R-5, group R-6) left with save groups; the ledger names their carriers.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../../backend";
import { flushAll, isConflicted, isDirty, loadFeed, moveBlock, pageByName, resetStore, resolveConflict, setRaw } from "../index";
import { doc } from "../model";
import { setToasts, toasts } from "../../toasts";
import { initParser } from "../../render/parse";
import { startEditing } from "../../editorController";
import { bindTestHost, mailPage, submittedPages } from "../host/wiring.test.support";
import type { BlockDto, PageRead } from "../../types";

let serial = 0;
const block = (raw: string): BlockDto => ({ id: `review-${++serial}`, raw, collapsed: false, children: [] });
const page = (name: string, raws: string[]): PageRead => ({
  id: `pages/${name}.md`, name, title: name, kind: "page", pre_block: null,
  rev: `initial-${name}`, blocks: raws.map(block),
});
const memory = (name: string) => pageByName(name)?.roots.map((id) => doc.byId[id].raw) ?? [];
const pathless = (name: string, raws: string[]) => ({ ...page(name, raws), id: undefined, rev: undefined });

beforeAll(() => initParser());
beforeEach(() => { serial = 0; resetStore(); setToasts([]); });
afterEach(() => { vi.restoreAllMocks(); resetStore(); });

describe("save review regressions on the page host", () => {
  it("S-3: use disk leaves that page clean and never rewrites its disk version", async () => {
    const moved = block("X"), stay = block("stay");
    loadFeed([{ ...page("A", []), blocks: [moved] }, { ...page("B", []), blocks: [stay] }]);
    const host = await bindTestHost();
    startEditing(stay.id); // the user is on B: the window holds it
    const move = vi.spyOn(backend(), "pageMove");
    const submit = vi.spyOn(backend(), "pageSubmit");
    await moveBlock(moved.id, null, 0, "B");
    await vi.waitFor(() => expect(move).toHaveBeenCalledOnce());
    // The host found B's file changed under the move's text: a conflict.
    host.notice("pages/B.md", { conflictReported: true }, { version: 9, conflict: true, risk: true, disk: { kind: "file", rev: "ext" } });
    await vi.waitFor(() => expect(isConflicted("B")).toBe(true));
    const onDisk = { ...page("B", ["external"]), rev: "ext" };
    const discard = vi.spyOn(backend(), "pageDiscard").mockImplementation(async (_session, id, key) => {
      queueMicrotask(() => host.deliver({ key, page: mailPage(10, onDisk), answer: { id, version: 10, took: false, outcome: { kind: "applied" } } }));
      return null;
    });
    expect(await resolveConflict("B", "disk")).toBe(true);
    expect(discard).toHaveBeenCalledOnce();
    await vi.waitFor(() => expect(memory("B")).toEqual(["external"]));
    expect(isConflicted("B")).toBe(false);
    expect(isDirty("B")).toBe(false);
    const before = submit.mock.calls.length + move.mock.calls.length;
    expect(await flushAll()).toBe(true);
    expect(submit.mock.calls.length + move.mock.calls.length).toBe(before);
  });

  it("R-6: a twin refusal on a page with no file keeps its text and names the existing file", async () => {
    loadFeed([pathless("B", ["B"])]);
    await bindTestHost();
    const api = backend();
    const submit = vi.spyOn(api, "pageSubmit").mockResolvedValue({ reason: "twin", existing: "pages/b.md" });
    setRaw(pageByName("B")!.roots[0], "typed in B");
    await vi.waitFor(() => expect(toasts().map((t) => t.message))
      .toContain("Couldn't save “B”: pages/b.md is already this page. Your edits stay in the editor."), { timeout: 1000 });
    expect(submittedPages(submit, "B")[0].blocks[0].raw).toBe("typed in B");
    expect(memory("B")).toEqual(["typed in B"]);
    // Not a conflict: there is no disk version to keep "mine" over.
    expect(await resolveConflict("B", "mine")).toBe(false);
    expect(await flushAll()).toBe(false);
  });

  it("a failed resolution names the page whose resolution failed and sends nothing for it", async () => {
    loadFeed([page("A", ["A"]), pathless("C", ["C"])]);
    await bindTestHost();
    const api = backend();
    const open = api.pageOpen.bind(api);
    vi.spyOn(api, "pageOpen").mockImplementation(async (session, id, target) =>
      target.name === "C" && target.path === null ? Promise.reject(new Error("unavailable")) : open(session, id, target));
    const submit = vi.spyOn(api, "pageSubmit");
    setRaw(pageByName("A")!.roots[0], "A edited");
    setRaw(pageByName("C")!.roots[0], "C edited");
    await vi.waitFor(() => expect(toasts().some((t) => t.message.startsWith("Couldn't save “C”"))).toBe(true), { timeout: 1000 });
    expect(toasts().find((t) => t.message.startsWith("Couldn't save “C”"))?.message).toContain("unavailable");
    expect(await flushAll()).toBe(false);
    expect(submittedPages(submit, "C")).toEqual([]);
    expect(submittedPages(submit, "A").at(-1)?.blocks[0].raw).toBe("A edited");
  });
});
