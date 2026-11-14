import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { setGraphMeta } from "../graphSession";
import { resetStore } from "../document";
import { installPageContent, loadSingle } from "../document/workingSet";
import { mainPaneRouter, resetTabsToJournals } from "../router";
import { PageView } from "./Page";
import type { JournalFeedPage, PageRead } from "../types";
import { appNow, journalTitle, localDayKey } from "../journal";
import { invalidatePageRead } from "../document/readAdmission";
import { setToasts, toasts } from "../toasts";

afterEach(() => {
  resetStore(); resetTabsToJournals(); setGraphMeta(null);
  vi.restoreAllMocks(); setToasts([]); document.body.replaceChildren();
});

for (const invalidations of [2, 5]) it(`${invalidations} invalidated initial feed reads show newest content without an error`, async () => {
  const opened = await backend().loadGraph("");
  if (opened.kind === "focused_existing") throw new Error("missing graph");
  setGraphMeta({ ...opened.meta, root: `/synthetic-feed-invalidations-${invalidations}` });
  const name = journalTitle(appNow());
  const saved: PageRead = { name, title: name, kind: "journal", id: "journals/today.md", rev: "newest", pre_block: null,
    blocks: [{ id: "newest-today", raw: "newest saved journal content", collapsed: false, children: [] }] };
  const read = vi.spyOn(backend(), "journalFeedPage").mockImplementation(async () => {
    if (read.mock.calls.length <= invalidations) invalidatePageRead(name);
    return { pages: [saved], next_before_day: null, done: true, as_of_day: localDayKey(), references_pending: false };
  });
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <PageView />, root);
  try {
    await vi.waitFor(() => expect(root.textContent).toMatch(/newest saved journal content|Journal feed read failed/));
    expect(root.textContent).not.toContain("Journal feed read failed");
    expect(root.querySelector(".page-blocks")?.textContent).toContain(saved.blocks[0].raw);
    expect(toasts().filter(toast => toast.kind === "error")).toEqual([]);
    expect(read).toHaveBeenCalledTimes(invalidations + 1);
  } finally { dispose(); }
});

it("a delayed empty feed cannot replace today's newly saved text with its synthesized blank day", async () => {
  const opened = await backend().loadGraph("");
  if (opened.kind === "focused_existing") throw new Error("missing graph");
  setGraphMeta({ ...opened.meta, root: "/synthetic-empty-feed-generation" });
  const name = journalTitle(appNow());
  const saved: PageRead = { name, title: name, kind: "journal", id: "journals/today.md", rev: "B", pre_block: null,
    blocks: [{ id: "saved-today", raw: "today saved while feed was pending", collapsed: false, children: [] }] };
  const response = (pages: PageRead[]): JournalFeedPage => ({ pages, next_before_day: null, done: true,
    as_of_day: localDayKey(), references_pending: false });
  let deliver!: (value: JournalFeedPage) => void;
  const read = vi.spyOn(backend(), "journalFeedPage")
    .mockImplementationOnce(() => new Promise(resolve => { deliver = resolve; }))
    .mockResolvedValue(response([saved]));
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <PageView />, root);
  try {
    await vi.waitFor(() => expect(read).toHaveBeenCalledOnce());
    installPageContent(name, saved);
    deliver(response([]));
    await vi.waitFor(() => expect(root.querySelector(".page-blocks")?.textContent).toContain(saved.blocks[0].raw));
    expect(read).toHaveBeenCalledTimes(2);
  } finally { dispose(); }
});

it("two invalidated route answers recover the current loaded text within the invalidation bound", async () => {
  const opened = await backend().loadGraph("");
  if (opened.kind === "focused_existing") throw new Error("missing graph");
  setGraphMeta({ ...opened.meta, root: "/synthetic-read-generation" });
  const page = (text: string): PageRead => ({ name: "Read generation", title: "Read generation", kind: "page", id: "pages/Read generation.md", rev: text,
    pre_block: null, blocks: [{ id: "generation-block", raw: text, collapsed: false, children: [] }] });
  const pending: Array<(dto: PageRead) => void> = [];
  const read = vi.spyOn(backend(), "getPageByPath").mockImplementation(() => pending.length < 2
    ? new Promise(resolve => pending.push(resolve)) : Promise.resolve(page("current saved text")));
  loadSingle(page("A"));
  mainPaneRouter.openPage("Read generation", "page", { inPlace: true });
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <PageView />, root);
  try {
    await vi.waitFor(() => expect(read).toHaveBeenCalledTimes(1));
    installPageContent("Read generation", page("B"));
    pending[0](page("A"));
    await vi.waitFor(() => expect(read).toHaveBeenCalledTimes(2));
    installPageContent("Read generation", page("current saved text"));
    pending[1](page("B"));
    await vi.waitFor(() => expect(root.querySelector(".page-blocks")?.textContent).toContain("current saved text"));
    expect(read).toHaveBeenCalledTimes(3);
  } finally { dispose(); }
});

it("an actual initial feed read failure remains visible", async () => {
  const opened = await backend().loadGraph("");
  if (opened.kind === "focused_existing") throw new Error("missing graph");
  setGraphMeta({ ...opened.meta, root: "/synthetic-feed-read-error" });
  const read = vi.spyOn(backend(), "journalFeedPage").mockRejectedValue(new Error("synthetic disk read failure"));
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <PageView />, root);
  try {
    await vi.waitFor(() => expect(root.textContent).toContain("synthetic disk read failure"));
    expect(toasts().some(toast => toast.kind === "error")).toBe(true);
    expect(read).toHaveBeenCalledOnce();
  } finally { dispose(); }
});
