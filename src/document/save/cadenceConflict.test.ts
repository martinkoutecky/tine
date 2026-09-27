import { afterEach, beforeAll, beforeEach, expect, it, vi } from "vitest";
import { backend } from "../../backend";
import { initParser } from "../../render/parse";
import { setToasts, toasts } from "../../toasts";
import { beginPageHeaderEdit, finishPageHeaderEdit, loadFeed, pageByName, resetStore, setRaw } from "../index";
import { doc } from "../model";
import { flushPage, isConflicted, isDirty, markConflict, resolveConflict } from "./engine";
import type { PageRead } from "../../types";

const page = (id = "pages/Note.md", rev = "original"): PageRead => ({
  id, name: "Note", title: "Note", kind: "page", pre_block: null, rev,
  blocks: [{ id: "body", raw: "original", collapsed: false, children: [] }],
});

beforeAll(() => initParser());
beforeEach(() => { resetStore(); setToasts([]); });
afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks(); });

it("bounds a continuous 300 ms typing burst and coalesces a short burst", async () => {
  loadFeed([page()]);
  const save = vi.spyOn(backend(), "savePages").mockImplementation(async () => ({ ok: ["saved"] }));
  vi.useFakeTimers();
  for (let i = 0; i < 40; i++) {
    setRaw("body", `typing ${i}`);
    await vi.advanceTimersByTimeAsync(300);
  }
  expect(save.mock.calls.length).toBeGreaterThanOrEqual(3);
  const before = save.mock.calls.length;
  setRaw("body", "short 1");
  await vi.advanceTimersByTimeAsync(100);
  setRaw("body", "short 2");
  await vi.advanceTimersByTimeAsync(400);
  expect(save.mock.calls.length).toBe(before + 1);
});

it("Use disk reloads the pinned file when another file has the same page name", async () => {
  const pinned = page("pages/folder/Note.md");
  loadFeed([pinned]);
  markConflict("Note");
  const canonical = vi.spyOn(backend(), "getPage").mockResolvedValue({ ...page(), blocks: [{ id: "wrong", raw: "other file", collapsed: false, children: [] }] });
  const byPath = vi.spyOn(backend(), "getPageByPath").mockResolvedValue({ ...pinned, rev: "external", blocks: [{ id: "right", raw: "pinned file", collapsed: false, children: [] }] });
  expect(await resolveConflict("Note", "disk")).toBe(true);
  expect(byPath).toHaveBeenCalledWith(pinned.id);
  expect(canonical).not.toHaveBeenCalled();
  expect(pageByName("Note")?.id).toBe(pinned.id);
  expect(doc.byId[pageByName("Note")!.roots[0]].raw).toBe("pinned file");
});

it("Keep mine conflicts again if disk changed after the banner", async () => {
  loadFeed([page()]);
  setRaw("body", "my edit");
  let diskRev = "seen";
  const save = vi.spyOn(backend(), "savePages").mockImplementation(async (entries) => {
    if (save.mock.calls.length === 1) return { failed: { index: 0, family: "conflict", diskRev: "seen", undoFailed: [] } };
    if (entries[0].force || entries[0].baseRev !== diskRev)
      return entries[0].force ? { ok: ["clobbered"] } : { failed: { index: 0, family: "conflict", diskRev, undoFailed: [] } };
    return { ok: ["saved"] };
  });
  expect(await flushPage("Note")).toBe(false);
  expect(isConflicted("Note")).toBe(true);
  diskRev = "newer";
  expect(await resolveConflict("Note", "mine")).toBe(false);
  expect(isConflicted("Note")).toBe(true);
  expect(save.mock.calls[1][0][0].baseRev).toBe("seen");
  expect(await resolveConflict("Note", "mine")).toBe(true);
  expect(save.mock.calls[2][0][0].baseRev).toBe("newer");
});

it("a queued Keep mine cannot borrow a later conflict decision", async () => {
  loadFeed([page()]);
  setRaw("body", "my edit");
  let finish!: (result: { ok: string[] }) => void;
  const pending = new Promise<{ ok: string[] }>((resolve) => { finish = resolve; });
  const save = vi.spyOn(backend(), "savePages").mockImplementationOnce(() => pending)
    .mockResolvedValue({ ok: ["unexpected overwrite"] });
  const first = flushPage("Note");
  await vi.waitFor(() => expect(save).toHaveBeenCalledTimes(1));
  markConflict("Note", { kind: "disk-changed" }, "seen");
  const mine = resolveConflict("Note", "mine");
  markConflict("Note", { kind: "disk-changed" }, "newer");
  finish({ ok: ["first"] });
  await first;
  expect(await mine).toBe(false);
  expect(save).toHaveBeenCalledTimes(1);
  expect(isConflicted("Note")).toBe(true);
});

it("an incomplete page header stays dirty without repeated autosave toasts", async () => {
  loadFeed([{ ...page(), pre_block: "tags:: old\n" }]);
  const header = beginPageHeaderEdit("Note")!;
  vi.useFakeTimers();
  setRaw(header, "tags:");
  const save = vi.spyOn(backend(), "savePages").mockResolvedValue({ ok: ["saved"] });
  await vi.advanceTimersByTimeAsync(1200);
  expect(save).not.toHaveBeenCalled();
  expect(isDirty("Note")).toBe(true);
  expect(toasts().filter((toast) => toast.kind === "error")).toHaveLength(0);
  finishPageHeaderEdit(header);
  expect(toasts().filter((toast) => toast.kind === "error")).toHaveLength(1);
  setRaw(header, "tags:: new");
  expect(await flushPage("Note")).toBe(true);
  expect(save).toHaveBeenCalledTimes(1);
});

it("a folded first-root header remains a header on the next edit", async () => {
  loadFeed([{ ...page(), blocks: [{ id: "body", raw: "tags:: old", collapsed: false, children: [] }] }]);
  const save = vi.spyOn(backend(), "savePages").mockResolvedValue({ ok: ["folded"] });
  setRaw("body", "tags:: updated");
  expect(await flushPage("Note")).toBe(true);
  expect(save.mock.calls[0][0][0].page.pre_block).toBe("tags:: updated");
  expect(doc.byId.body.originatedFromPageHeader).toBe(true);
  setRaw("body", "tags:");
  expect(await flushPage("Note")).toBe(false);
  expect(save).toHaveBeenCalledTimes(1);
});
