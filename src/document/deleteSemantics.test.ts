// master 695527da9 ("preserve delete semantics after rebase"): a by-name delete
// of a page that is not loaded must still delete it, and deleting a page the user
// has left in conflict must not first write its draft over the external bytes.
// og's deletePage (document/workingSet.ts) never had master's quiescence gate,
// so these pin the equivalent outcome rather than a fix. Writes go through the
// page host (step 3b): a page's text is sent with `pageSubmit`, a delete is `pageDelete`.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { initParser } from "../render/parse";
import { backend } from "../backend";
import { deletePage, flushPage, isConflicted, isDirty, pageByName, resetStore, setRaw } from ".";
import { loadSingle } from "./workingSet";
import { bindTestHost, type TestHost } from "./host/wiring.test.support";

let host: TestHost;
beforeAll(() => initParser());
beforeEach(async () => {
  resetStore();
  vi.restoreAllMocks();
  host = await bindTestHost();
});
afterEach(() => {
  resetStore();
  vi.restoreAllMocks();
});

describe("delete semantics (master 695527da9)", () => {
  it("deletes a page that is not loaded, by name", async () => {
    const remove = vi.spyOn(backend(), "pageDelete");
    const save = vi.spyOn(backend(), "pageSubmit");
    expect(pageByName("Elsewhere")).toBeUndefined();
    expect(await deletePage("Elsewhere", "page")).toBe(true);
    expect(remove).toHaveBeenCalledWith(host.session, "Elsewhere", "page", undefined);
    expect(save).not.toHaveBeenCalled();
  });

  it("deletes a conflicted page without first writing its draft over the disk copy", async () => {
    loadSingle({
      name: "Clash", kind: "page", title: "Clash", pre_block: null, id: "pages/Clash.md", rev: "r1",
      blocks: [{ id: "c1", raw: "base", collapsed: false, children: [] }],
    } as never);
    // The host took the draft, then the file changed on disk under it: a conflict
    // the user has left unresolved (the host keeps "my draft" unsaved).
    const admitted: Array<{ id: number; key: string }> = [];
    const submit = vi.spyOn(backend(), "pageSubmit").mockImplementation(async (_session, id, key) => { admitted.push({ id, key }); return null; });
    setRaw("c1", "my draft");
    void flushPage("Clash");
    await vi.waitFor(() => expect(admitted).toHaveLength(1));
    host.deliver({ key: admitted[0].key, answer: { id: admitted[0].id, version: 9, took: true, outcome: { kind: "applied" } },
      notice: { conflictReported: true },
      page: { version: 9, conflict: true, risk: false, disk: { kind: "file", rev: "external" }, text: { kind: "unchanged" } } });
    expect(isConflicted("Clash")).toBe(true);
    submit.mockClear();
    const remove = vi.spyOn(backend(), "pageDelete");
    expect(await deletePage("Clash", "page", "pages/Clash.md")).toBe(true);
    expect(submit).not.toHaveBeenCalled();
    expect(remove).toHaveBeenCalledWith(host.session, "Clash", "page", "pages/Clash.md");
    expect(pageByName("Clash")).toBeUndefined();
    expect(isDirty("Clash")).toBe(false);
  });
});
