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

afterEach(() => {
  resetStore(); resetTabsToJournals(); setGraphMeta(null);
  vi.restoreAllMocks(); document.body.replaceChildren();
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

it("two invalidated route answers leave the current loaded text visible after the bounded retry", async () => {
  const opened = await backend().loadGraph("");
  if (opened.kind === "focused_existing") throw new Error("missing graph");
  setGraphMeta({ ...opened.meta, root: "/synthetic-read-generation" });
  const page = (text: string): PageRead => ({ name: "Read generation", title: "Read generation", kind: "page", id: "pages/Read generation.md", rev: text,
    pre_block: null, blocks: [{ id: "generation-block", raw: text, collapsed: false, children: [] }] });
  const pending: Array<(dto: PageRead) => void> = [];
  const read = vi.spyOn(backend(), "getPageByPath").mockImplementation(() => new Promise(resolve => pending.push(resolve)));
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
    expect(read).toHaveBeenCalledTimes(2);
  } finally { dispose(); }
});
