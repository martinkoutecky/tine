// Audit L9-06: refuse input at the store boundary while deletion is in flight,
// then resume tracking ordinary edits when deletion does not apply.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { initParser } from "../render/parse";
import { backend } from "../backend";
import { applyGraphChange, beginPageHeaderEdit, blockPageReadOnly, blockWritable, deletePage, flushAll, insertOutlineChildren, isDirty, pageByName, pasteClipboardPayload, resetStore, setBlockProperty, setPageProperty, setRaw, unsavedDrafts } from "./index";
import { pageWritable } from "./edits/properties";
import { doc } from "./model";
import { loadRoutedPage } from "./workingSet";
import { bindTestHost, submittedPages } from "./host/wiring.test.support";
import type { PageOperation } from "./host/protocol";
import { writeField } from "../sheet/fields";

const PATH = "pages/Doomed.md";
beforeAll(() => initParser());
beforeEach(async () => {
  resetStore();
  vi.restoreAllMocks();
  loadRoutedPage({ name: "Doomed", kind: "page", title: "Doomed", id: PATH, rev: "r1", pre_block: null,
    blocks: [{ id: "d1", raw: "before", collapsed: false, children: [] }] } as never);
  await bindTestHost();
});
afterEach(() => { resetStore(); vi.restoreAllMocks(); });

async function typeDuringRefusedDelete(outcome: PageOperation | "throw") {
  let answer!: (op: PageOperation) => void, fail!: (e: unknown) => void;
  const started = vi.fn();
  vi.spyOn(backend(), "pageDelete").mockResolvedValue("waiting").mockImplementationOnce(() => {
    started();
    return new Promise<PageOperation>((res, rej) => { answer = res; fail = rej; });
  });
  const submit = vi.spyOn(backend(), "pageSubmit");
  const deleting = deletePage("Doomed", "page", PATH);
  await vi.waitFor(() => expect(started).toHaveBeenCalled());
  expect(pageWritable("Doomed")).toBe(false);
  expect(blockWritable("d1")).toBe(false);
  expect(blockPageReadOnly("d1")).toBe(true);
  // Textarea and plugin replacement both use setRaw. Properties, sheets and
  // outline paste must reject their edits through the same write boundary.
  setRaw("d1", "typed while deleting", { timetracking: false });
  setBlockProperty("d1", "color", "red");
  setPageProperty("Doomed", "tags", "blocked");
  expect(beginPageHeaderEdit("Doomed")).toBeNull();
  expect(writeField("d1", "state", "TODO")).toBe(false);
  expect(insertOutlineChildren("d1", [{ raw: "blocked child", children: [] }])).toBeNull();
  expect(await pasteClipboardPayload("d1", { op: "copy", generation: 1, graph: "", text: "pasted", sourcePages: [],
    blocks: [{ raw: "pasted", sourceFormat: "md", children: [] }] })).toBeNull();
  expect(doc.byId.d1.raw).toBe("before");
  expect(doc.byId.d1.children).toEqual([]);
  expect(pageByName("Doomed")!.preBlock).toBeNull();
  expect(pageByName("Doomed")!.roots).toEqual(["d1"]);
  if (outcome === "throw") fail(new Error("EIO")); else answer(outcome);
  expect(await deleting).toBe(false);
  expect(pageByName("Doomed")).toBeDefined();
  expect(doc.byId.d1.raw).toBe("before");
  expect(pageWritable("Doomed")).toBe(true);
  expect(blockWritable("d1")).toBe(true);
  expect(blockPageReadOnly("d1")).toBe(false);
  setRaw("d1", "typed after delete failed", { timetracking: false });
  return submit;
}

describe("L9-06: typing during a delete that does not apply", () => {
  for (const outcome of ["refused", "pending", "uncertain", "waiting", "superseded", "throw"] as const) {
    it(`blocks input during deletion and tracks it again after ${outcome}`, async () => {
      const submit = await typeDuringRefusedDelete(outcome);
      // Either the window still owes the text (close prompt / recovery panel) ...
      const tracked = isDirty("Doomed") || unsavedDrafts().some((d) => d.name === "Doomed");
      // ... or a flush sends it.
      await flushAll();
      const sent = submittedPages(submit, "Doomed").some((dto) => JSON.stringify(dto).includes("typed after delete failed"));
      expect({ tracked, sent }).not.toEqual({ tracked: false, sent: false });
    });
  }

  it("keeps resumed input recoverable through an external change after a refused delete", async () => {
    await typeDuringRefusedDelete("refused");
    const external = { name: "Doomed", kind: "page", title: "Doomed", id: PATH, rev: "r2", pre_block: null,
      blocks: [{ id: "d1", raw: "external edit", collapsed: false, children: [] }] };
    vi.spyOn(backend(), "getPage").mockResolvedValue(external as never);
    vi.spyOn(backend(), "getPageByPath").mockResolvedValue(external as never);
    await applyGraphChange({ name: "Doomed", kind: "page", path: PATH } as never);
    await new Promise((r) => setTimeout(r, 50));
    const raws = Object.values(doc.byId).filter((n) => n.page === "Doomed").map((n) => n.raw);
    const owed = unsavedDrafts().some((d) => d.name === "Doomed");
    // The user's typed text must survive somewhere: on the page, or as an unsaved draft.
    const onPage = raws.some((raw) => raw.includes("typed after delete failed"));
    expect({ onPage, owed }).not.toEqual({ onPage: false, owed: false });
  });
  it("blocks input during a successful delete and then retires the page", async () => {
    let answer!: (op: PageOperation) => void;
    const remove = vi.spyOn(backend(), "pageDelete").mockImplementation(() => new Promise((done) => { answer = done; }));
    const deleting = deletePage("Doomed", "page", PATH);
    await vi.waitFor(() => expect(remove).toHaveBeenCalledOnce());
    expect(pageWritable("Doomed")).toBe(false);
    setRaw("d1", "blocked", { timetracking: false });
    expect(doc.byId.d1.raw).toBe("before");
    answer("applied");
    expect(await deleting).toBe(true);
    expect(pageByName("Doomed")).toBeUndefined();
  });
  it("control: the same typing with no delete in flight is tracked", async () => {
    const submit = vi.spyOn(backend(), "pageSubmit");
    setRaw("d1", "typed while deleting", { timetracking: false });
    const tracked = isDirty("Doomed") || unsavedDrafts().some((d) => d.name === "Doomed");
    await flushAll();
    const sent = submittedPages(submit, "Doomed").some((dto) => JSON.stringify(dto).includes("typed while deleting"));
    expect(tracked || sent).toBe(true);
  });
});
