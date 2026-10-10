// Send cadence, conflict choices and page-header drafts on the wired page host
// (STEP3 §4). The window sends input (debounced, §4.2) and the host owns the
// save cadence, retries and conflict detection (§4.3); conflicts reach the
// window as host mail, and Use disk / Keep mine are a discard and a reviewed
// submit on the disk state the banner showed.
import { afterEach, beforeAll, beforeEach, expect, it, vi, type MockInstance } from "vitest";
import { backend, type Backend } from "../../backend";
import { initParser } from "../../render/parse";
import { setToasts, toasts } from "../../toasts";
import { startEditing } from "../../editorController";
import { beginPageHeaderEdit, finishPageHeaderEdit, flushPage, isConflicted, isDirty, loadFeed, pageByName,
  resetStore, resolveConflict, setRaw } from "../index";
import { doc } from "../model";
import { bindTestHost, mailPage, submittedPages, type TestHost } from "../host/wiring.test.support";
import type { PageDto, PageRead } from "../../types";

const page = (id = "pages/Note.md", rev = "original"): PageRead => ({
  id, name: "Note", title: "Note", kind: "page", pre_block: null, rev,
  blocks: [{ id: "body", raw: "original", collapsed: false, children: [] }],
});
const errors = () => toasts().filter((toast) => toast.kind === "error");

beforeAll(() => initParser());
beforeEach(() => { resetStore(); setToasts([]); });
afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks(); resetStore(); });

/** The user is editing Note's block and typed `text`; the host took it. */
async function typedAndTaken(text: string): Promise<MockInstance<Backend["pageSubmit"]>> {
  startEditing("body");
  const submit = vi.spyOn(backend(), "pageSubmit");
  setRaw("body", text);
  await vi.waitFor(() => expect(submit).toHaveBeenCalledOnce(), { timeout: 1000 });
  return submit;
}

/** The host reports a conflict on the held page: the file changed to `rev`. */
function conflict(host: TestHost, key: string, version: number, rev: string): void {
  host.notice(key, { conflictReported: true }, { version, conflict: true, risk: true, disk: { kind: "file", rev } });
}

it("bounds a continuous 300 ms typing burst and coalesces a short burst", async () => {
  loadFeed([page()]);
  await bindTestHost();
  const submit = vi.spyOn(backend(), "pageSubmit");
  vi.useFakeTimers();
  for (let i = 0; i < 40; i++) {
    setRaw("body", `typing ${i}`);
    await vi.advanceTimersByTimeAsync(300);
  }
  expect(submit.mock.calls.length).toBeGreaterThanOrEqual(3);
  await vi.advanceTimersByTimeAsync(400);
  const before = submit.mock.calls.length;
  setRaw("body", "short 1");
  await vi.advanceTimersByTimeAsync(100);
  setRaw("body", "short 2");
  await vi.advanceTimersByTimeAsync(400);
  expect(submit.mock.calls.length).toBe(before + 1);
  expect(submittedPages(submit).at(-1)?.blocks[0].raw).toBe("short 2");
});

it("Use disk reloads the pinned file when another file has the same page name", async () => {
  const pinned = page("pages/folder/Note.md");
  loadFeed([pinned]);
  const host = await bindTestHost();
  await typedAndTaken("my edit");
  conflict(host, pinned.id, 5, "external");
  await vi.waitFor(() => expect(isConflicted("Note")).toBe(true));
  const canonical = vi.spyOn(backend(), "getPage");
  const onDisk: PageDto = { ...pinned, rev: "external", blocks: [{ id: "right", raw: "pinned file", collapsed: false, children: [] }] };
  // The host answers a discard with the text of the file the key names.
  const discard = vi.spyOn(backend(), "pageDiscard").mockImplementation(async (_session, id, key) => {
    queueMicrotask(() => host.deliver({ key, page: mailPage(6, onDisk),
      answer: { id, version: 6, took: false, outcome: { kind: "applied" } } }));
    return null;
  });
  expect(await resolveConflict("Note", "disk")).toBe(true);
  expect(discard).toHaveBeenCalledOnce();
  expect(discard.mock.calls[0][2]).toBe(pinned.id);
  expect(canonical).not.toHaveBeenCalled();
  expect(pageByName("Note")?.id).toBe(pinned.id);
  await vi.waitFor(() => expect(doc.byId[pageByName("Note")!.roots[0]]?.raw).toBe("pinned file"));
  expect(isConflicted("Note")).toBe(false);
});

it("Keep mine resolves against the disk state the banner showed; a later change conflicts again", async () => {
  loadFeed([page()]);
  const host = await bindTestHost();
  const submit = await typedAndTaken("my edit");
  conflict(host, "pages/Note.md", 5, "seen");
  await vi.waitFor(() => expect(isConflicted("Note")).toBe(true));
  expect(await resolveConflict("Note", "mine")).toBe(true);
  expect(submit.mock.calls[1][5]).toEqual({ kind: "file", rev: "seen" });
  expect(submittedPages(submit)[1].blocks[0].raw).toBe("my edit");
  // The file changed again after the banner: the host's save guard finds it
  // and reports a new conflict on the newer disk state.
  conflict(host, "pages/Note.md", 9, "newer");
  await vi.waitFor(() => expect(isConflicted("Note")).toBe(true));
  expect(await resolveConflict("Note", "mine")).toBe(true);
  expect(submit.mock.calls[2][5]).toEqual({ kind: "file", rev: "newer" });
});

it("a queued Keep mine cannot borrow a later conflict decision", async () => {
  loadFeed([page()]);
  const host = await bindTestHost();
  const submit = await typedAndTaken("my edit");
  conflict(host, "pages/Note.md", 5, "seen");
  await vi.waitFor(() => expect(isConflicted("Note")).toBe(true));
  // The user types on the conflicted page; that send is in flight when Keep
  // mine is chosen, so the choice waits for it. Before the window sees the
  // answer, the host observes the file again: a newer conflict.
  let finish!: () => void;
  const gate = new Promise<void>((resolve) => { finish = resolve; });
  submit.mockImplementationOnce(async (_session, id, key) => {
    await gate;
    queueMicrotask(() => {
      host.deliver({ key, answer: { id, version: 6, took: true, outcome: { kind: "applied" } }, notice: { conflictReported: true },
        page: { version: 6, conflict: true, risk: true, disk: { kind: "file", rev: "seen" }, text: { kind: "unchanged" } } });
      conflict(host, key, 7, "newer");
    });
    return null;
  });
  setRaw("body", "more");
  await vi.waitFor(() => expect(submit).toHaveBeenCalledTimes(2), { timeout: 1000 });
  const mine = resolveConflict("Note", "mine");
  finish();
  expect(await mine).toBe(false);
  // No Keep mine went out: only the two typed sends, neither resolving a disk state.
  expect(submit).toHaveBeenCalledTimes(2);
  expect(submit.mock.calls.map((call) => call[5])).toEqual([null, null]);
  await vi.waitFor(() => expect(isConflicted("Note")).toBe(true));
  expect(toasts().some((toast) => toast.message === "“Note” changed meanwhile. Choose again.")).toBe(true);
});

it("an incomplete page header stays unsent without repeated autosave toasts", async () => {
  loadFeed([{ ...page(), pre_block: "tags:: old\n" }]);
  await bindTestHost();
  const header = beginPageHeaderEdit("Note")!;
  const submit = vi.spyOn(backend(), "pageSubmit");
  vi.useFakeTimers();
  setRaw(header, "tags:");
  await vi.advanceTimersByTimeAsync(1200);
  expect(submit).not.toHaveBeenCalled();
  expect(isDirty("Note")).toBe(true);
  expect(errors()).toHaveLength(0);
  finishPageHeaderEdit(header);
  expect(errors()).toHaveLength(1);
  vi.useRealTimers();
  setRaw(header, "tags:: new");
  expect(await flushPage("Note")).toBe(true);
  expect(submit).toHaveBeenCalledOnce();
  expect(submittedPages(submit)[0].pre_block).toBe("tags:: new\n");
});

it("a folded first-root header remains a header on the next edit", async () => {
  loadFeed([{ ...page(), blocks: [{ id: "body", raw: "tags:: old", collapsed: false, children: [] }] }]);
  await bindTestHost();
  const submit = vi.spyOn(backend(), "pageSubmit");
  setRaw("body", "tags:: updated");
  expect(await flushPage("Note")).toBe(true);
  expect(submittedPages(submit)[0].pre_block).toBe("tags:: updated");
  expect(doc.byId.body.originatedFromPageHeader).toBe(true);
  setRaw("body", "tags:");
  expect(await flushPage("Note")).toBe(false);
  expect(submit).toHaveBeenCalledOnce();
});
