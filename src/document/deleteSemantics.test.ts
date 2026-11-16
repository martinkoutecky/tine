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
import { pageWritable } from "./edits/properties";

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
  it("A3: an unanswered conflicted drain times out, unlocks input and never deletes after a late answer", async () => {
    loadSingle({ name: "Clash", kind: "page", title: "Clash", pre_block: null, id: "pages/Clash.md", rev: "r1",
      blocks: [{ id: "c1", raw: "base", collapsed: false, children: [] }] });
    const pending: Array<{ id: number; key: string }> = [];
    vi.spyOn(backend(), "pageSubmit").mockImplementation(async (_session, id, key) => { pending.push({ id, key }); return null; });
    const answer = (i: number) => host.deliver({ key: pending[i].key,
      answer: { id: pending[i].id, version: 9 + i, took: true, outcome: { kind: "applied" } },
      page: { version: 9 + i, conflict: true, risk: false, disk: { kind: "file", rev: "external" }, text: { kind: "unchanged" } } });
    setRaw("c1", "first draft");
    void flushPage("Clash");
    await vi.waitFor(() => expect(pending).toHaveLength(1));
    answer(0);
    await Promise.resolve();
    setRaw("c1", "last irreplaceable keystroke");
    const remove = vi.spyOn(backend(), "pageDelete");
    vi.useFakeTimers();
    try {
      let result: boolean | undefined;
      void deletePage("Clash", "page", "pages/Clash.md").then(value => { result = value; });
      expect(pageWritable("Clash")).toBe(false);
      await vi.runAllTimersAsync();
      expect(result).toBe(false);
      expect(pageWritable("Clash")).toBe(true);
      const { doc } = await import("./model");
      expect(doc.byId.c1.raw).toBe("last irreplaceable keystroke");
      answer(1);
      await vi.runAllTimersAsync();
      expect(remove).not.toHaveBeenCalled();
      setRaw("c1", "editable after timeout");
      expect(doc.byId.c1.raw).toBe("editable after timeout");
    } finally { vi.useRealTimers(); }
  });
  it.each([{ inFlight: false, rejected: false }, { inFlight: true, rejected: false }, { inFlight: false, rejected: true }])(
    "FU1: recovery includes debounced input (in-flight $inFlight, rejected $rejected)", async ({ inFlight, rejected }) => {
    loadSingle({ name: "Clash", kind: "page", title: "Clash", pre_block: null, id: "pages/Clash.md", rev: "r1",
      blocks: [{ id: "c1", raw: "base", collapsed: false, children: [] }] });
    const pending: Array<{ id: number; key: string; raw: string }> = [];
    vi.spyOn(backend(), "pageSubmit").mockImplementation(async (_session, id, key, dto) => {
      pending.push({ id, key, raw: dto.blocks[0].raw });
      return rejected && pending.length > 1 ? { reason: "not-admitted" } : null;
    });
    const answer = (i: number) => host.deliver({ key: pending[i].key,
      answer: { id: pending[i].id, version: 9 + i, took: true, outcome: { kind: "applied" } },
      page: { version: 9 + i, conflict: true, risk: false, disk: { kind: "file", rev: "external" }, text: { kind: "unchanged" } } });
    setRaw("c1", "first draft");
    void flushPage("Clash");
    await vi.waitFor(() => expect(pending).toHaveLength(1));
    answer(0);
    await Promise.resolve();
    if (inFlight) {
      setRaw("c1", "in-flight draft");
      void flushPage("Clash");
      await vi.waitFor(() => expect(pending).toHaveLength(2));
    }
    setRaw("c1", "last irreplaceable keystroke");
    let recovery = "";
    const remove = vi.spyOn(backend(), "pageDelete").mockImplementation(async () => {
      recovery = pending.at(-1)!.raw; return "applied";
    });
    const deleting = deletePage("Clash", "page", "pages/Clash.md");
    expect(pageWritable("Clash")).toBe(false);
    setRaw("c1", "typing during drain must be refused");
    if (rejected) {
      expect(await deleting).toBe(false);
      expect(remove).not.toHaveBeenCalled();
      expect(pageByName("Clash")).toBeDefined();
      expect(pageWritable("Clash")).toBe(true);
      const { doc } = await import("./model");
      expect(doc.byId.c1.raw).toBe("last irreplaceable keystroke");
      return;
    }
    if (inFlight) { expect(remove).not.toHaveBeenCalled(); answer(1); }
    await vi.waitFor(() => expect(pending.at(-1)!.raw).toBe("last irreplaceable keystroke"));
    expect(remove).not.toHaveBeenCalled();
    answer(pending.length - 1);
    expect(await deleting).toBe(true);
    expect(recovery).toBe("last irreplaceable keystroke");
  });
  it.each(["refused", "waiting", "pending", "uncertain", "superseded", "throw"])("L2-04: a %s delete retains conflicted input", async (outcome) => {
    loadSingle({ name: "Clash", kind: "page", title: "Clash", pre_block: null, id: "pages/Clash.md", rev: "r1",
      blocks: [{ id: "c1", raw: "base", collapsed: false, children: [] }] });
    const admitted: Array<{ id: number; key: string }> = [];
    vi.spyOn(backend(), "pageSubmit").mockImplementation(async (_session, id, key) => { admitted.push({ id, key }); return null; });
    setRaw("c1", "my irreplaceable draft");
    void flushPage("Clash");
    await vi.waitFor(() => expect(admitted).toHaveLength(1));
    host.deliver({ key: admitted[0].key, answer: { id: admitted[0].id, version: 9, took: true, outcome: { kind: "applied" } },
      notice: { conflictReported: true },
      page: { version: 9, conflict: true, risk: false, disk: { kind: "file", rev: "external" }, text: { kind: "unchanged" } } });
    const discard = vi.spyOn(backend(), "pageDiscard");
    vi.spyOn(backend(), "pageDelete").mockImplementation(async () => {
      if (outcome === "throw") throw new Error("disk error");
      return outcome as "refused";
    });
    expect(await deletePage("Clash", "page", "pages/Clash.md")).toBe(false);
    expect(discard).not.toHaveBeenCalled();
    expect(isConflicted("Clash")).toBe(true);
    const { doc } = await import("./model");
    expect(doc.byId.c1.raw).toBe("my irreplaceable draft");
  });
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
