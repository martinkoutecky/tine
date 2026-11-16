// "Changed since you last looked" on the routed page (ADR 0079): the backend's
// zero-based block paths land on the rendered rows, the header counts them, a
// removed block leaves a marker, a stale reply is dropped, and nothing re-reads
// on the page's own edits (F4).
import { afterEach, beforeAll, beforeEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import type { SeenPageChanges } from "../backendTypes";
import { initParser } from "../render/parse";
import { resetStore, setRaw } from "../document";
import { setGraphMeta } from "../graphSession";
import { mainPaneRouter, resetTabsToJournals } from "../router";
import { setToasts, toasts } from "../toasts";
import { resetSeenForTests } from "../seen/seenState";
import { PageView } from "./Page";

beforeAll(async () => { await initParser(); });

const blocks = () => [
  { id: "seen-0", has_id: false, raw: "Alpha", children: [{ id: "seen-0a", has_id: false, raw: "Alpha child", children: [], collapsed: false }], collapsed: false },
  { id: "seen-1", has_id: false, raw: "Beta\nstatus:: open", children: [], collapsed: false },
  { id: "seen-2", has_id: false, raw: "Gamma", children: [], collapsed: false },
];
const reply = (over: Partial<SeenPageChanges> = {}): SeenPageChanges => ({
  available: true, reason: null, head_commit: "h", baseline_commit: "b", current_text_sha256: "x",
  identity_known: true, unattributed: false, blocks: [], removed: [], ...over,
});

function openPage() {
  vi.spyOn(backend(), "getPage").mockResolvedValue({
    id: "pages/Seen.md", name: "Seen", title: "Seen", kind: "page", pre_block: null, blocks: blocks(),
  });
  mainPaneRouter.replaceActiveRoute({ kind: "page", name: "Seen", pageKind: "page" });
  const host = document.createElement("div");
  document.body.append(host);
  return { host, dispose: render(() => <PageView />, host) };
}
const rowOf = (host: Element, id: string) => host.querySelector(`[data-block-id="${id}"] > .block-main`);
const ready = (host: Element) => vi.waitFor(() => expect(host.querySelectorAll(".ls-block").length).toBeGreaterThanOrEqual(4));

beforeEach(() => { resetSeenForTests(); setGraphMeta({ root: "/graphs/seen" } as never); });
afterEach(() => {
  vi.restoreAllMocks();
  resetStore();
  resetTabsToJournals();
  setGraphMeta(null);
  setToasts([]);
  document.body.replaceChildren();
});

it("sends the page's key, path and complete text, and maps block paths onto the rendered rows", async () => {
  const read = vi.spyOn(backend(), "getPageChanges").mockResolvedValue(reply({
    blocks: [{ block_path: [0, 0], persisted_id: null }, { block_path: [1], persisted_id: null }],
  }));
  const { host, dispose } = openPage();
  try {
    await ready(host);
    await vi.waitFor(() => expect(host.querySelector("[data-seen-header]")).not.toBeNull());
    expect(read).toHaveBeenCalledTimes(1);
    expect(read).toHaveBeenCalledWith("seen", "pages/Seen.md", "- Alpha\n\t- Alpha child\n- Beta\n  status:: open\n- Gamma\n");
    expect([...host.querySelectorAll(".block-main.seen-changed")]).toEqual([rowOf(host, "seen-0a"), rowOf(host, "seen-1")]);
    expect(host.querySelector("[data-seen-header]")?.textContent).toContain("2 changes since you last looked");
  } finally { dispose(); }
});

it("an unchanged page shows neither bar nor header", async () => {
  vi.spyOn(backend(), "getPageChanges").mockResolvedValue(reply());
  const { host, dispose } = openPage();
  try {
    await ready(host);
    await Promise.resolve();
    expect(host.querySelector(".seen-changed, [data-seen-header], [data-seen-removed]")).toBeNull();
  } finally { dispose(); }
});

it("a removed block leaves a marker after its previous sibling that unfolds the removed text", async () => {
  vi.spyOn(backend(), "getPageChanges").mockResolvedValue(reply({
    removed: [{ text: "Deleted by Jo\nstatus:: done", baseline_block_path: [1], parent_path: null, previous_sibling_path: [0] }],
  }));
  const { host, dispose } = openPage();
  try {
    await ready(host);
    await vi.waitFor(() => expect(host.querySelector("[data-seen-removed]")).not.toBeNull());
    expect(host.querySelector("[data-seen-header]")?.textContent).toContain("1 change since you last looked");
    const marker = host.querySelector("[data-seen-removed]")!;
    expect(rowOf(host, "seen-0")?.parentElement?.contains(marker)).toBe(true);
    expect(marker.querySelector(".seen-removed-text")).toBeNull();
    (marker.querySelector("button") as HTMLButtonElement).click();
    expect(marker.querySelector(".seen-removed-text")?.textContent).toBe("Deleted by Jo\nstatus:: done");
  } finally { dispose(); }
});

it("the header explains an unknown identity and an unattributed history", async () => {
  vi.spyOn(backend(), "getPageChanges").mockResolvedValue(reply({
    identity_known: false, unattributed: true, blocks: [{ block_path: [2], persisted_id: null }],
  }));
  const { host, dispose } = openPage();
  try {
    await ready(host);
    await vi.waitFor(() => expect(host.querySelector("[data-seen-header]")).not.toBeNull());
    const title = host.querySelector(".seen-header-text")?.getAttribute("title") ?? "";
    expect(title).toMatch(/placeholder name count as yours/);
    expect(title).toMatch(/long history/);
  } finally { dispose(); }
});

it("a heavy edit (marker at the changed block's place) counts once", async () => {
  vi.spyOn(backend(), "getPageChanges").mockResolvedValue(reply({
    blocks: [{ block_path: [1], persisted_id: null }],
    removed: [{ text: "Old text", baseline_block_path: [1], parent_path: null, previous_sibling_path: [0] }],
  }));
  const { host, dispose } = openPage();
  try {
    await ready(host);
    await vi.waitFor(() => expect(host.querySelector("[data-seen-header]")).not.toBeNull());
    expect(host.querySelector("[data-seen-header]")?.textContent).toContain("1 change since you last looked");
    expect(host.querySelector("[data-seen-removed]")).not.toBeNull();
    expect(rowOf(host, "seen-1")?.classList.contains("seen-changed")).toBe(true);
  } finally { dispose(); }
});

it("shows the backend's reason in the tooltip and a muted line when unattributed", async () => {
  vi.spyOn(backend(), "getPageChanges").mockResolvedValue(reply({
    reason: "The seen baseline is not on this branch's main line.", unattributed: true,
    blocks: [{ block_path: [2], persisted_id: null }],
  }));
  const { host, dispose } = openPage();
  try {
    await ready(host);
    await vi.waitFor(() => expect(host.querySelector("[data-seen-header]")).not.toBeNull());
    expect(host.querySelector(".seen-header-text")?.getAttribute("title")).toContain("not on this branch's main line");
    expect(host.querySelector("[data-seen-unattributed]")?.textContent).toBe("Some of these may be your own edits.");
  } finally { dispose(); }
});

it("a known-attribution answer has no muted line", async () => {
  vi.spyOn(backend(), "getPageChanges").mockResolvedValue(reply({ blocks: [{ block_path: [2], persisted_id: null }] }));
  const { host, dispose } = openPage();
  try {
    await ready(host);
    await vi.waitFor(() => expect(host.querySelector("[data-seen-header]")).not.toBeNull());
    expect(host.querySelector("[data-seen-unattributed]")).toBeNull();
  } finally { dispose(); }
});

it("F4: the page's own edits never read again; an edited marked block loses its mark and its count", async () => {
  const read = vi.spyOn(backend(), "getPageChanges").mockResolvedValue(reply({
    blocks: [{ block_path: [1], persisted_id: null }, { block_path: [2], persisted_id: null }],
  }));
  const { host, dispose } = openPage();
  try {
    await ready(host);
    await vi.waitFor(() => expect(host.querySelectorAll(".block-main.seen-changed")).toHaveLength(2));
    setRaw("seen-1", "Beta\nstatus:: done");
    await vi.waitFor(() => expect(host.querySelectorAll(".block-main.seen-changed")).toHaveLength(1));
    expect(host.querySelector("[data-seen-header]")?.textContent).toContain("1 change since you last looked");
    setRaw("seen-2", "Gamma, edited");
    await vi.waitFor(() => expect(host.querySelector("[data-seen-header]")).toBeNull());
    expect(read).toHaveBeenCalledTimes(1);
  } finally { dispose(); }
});

it("returning to the app re-reads, and a late reply to an older request is discarded", async () => {
  const pending: ((value: SeenPageChanges) => void)[] = [];
  const read = vi.spyOn(backend(), "getPageChanges").mockImplementation(() => new Promise((resolve) => { pending.push(resolve); }));
  const { host, dispose } = openPage();
  try {
    await ready(host);
    await vi.waitFor(() => expect(read).toHaveBeenCalledTimes(1));
    window.dispatchEvent(new Event("focus"));
    await vi.waitFor(() => expect(read).toHaveBeenCalledTimes(2));
    pending[1](reply({ blocks: [{ block_path: [2], persisted_id: null }] }));
    await vi.waitFor(() => expect(rowOf(host, "seen-2")?.classList.contains("seen-changed")).toBe(true));
    pending[0](reply({ blocks: [{ block_path: [1], persisted_id: null }] })); // older request answers last
    await new Promise((r) => setTimeout(r, 20));
    expect(rowOf(host, "seen-1")?.classList.contains("seen-changed")).toBe(false);
    expect(host.querySelectorAll(".block-main.seen-changed")).toHaveLength(1);
  } finally { dispose(); }
});

it("a reply that arrives after the page was left changes nothing", async () => {
  let answer!: (value: SeenPageChanges) => void;
  vi.spyOn(backend(), "getPageChanges").mockImplementation(() => new Promise((resolve) => { answer = resolve; }));
  const { host, dispose } = openPage();
  await ready(host);
  await vi.waitFor(() => expect(answer).toBeTypeOf("function"));
  dispose();
  answer(reply({ blocks: [{ block_path: [0], persisted_id: null }] }));
  await new Promise((r) => setTimeout(r, 20));
  expect(host.querySelector(".seen-changed")).toBeNull();
});

it("leaving the page marks it seen once", async () => {
  const read = vi.spyOn(backend(), "getPageChanges").mockResolvedValue(reply({ blocks: [{ block_path: [1], persisted_id: null }] }));
  const mark = vi.spyOn(backend(), "markPageSeen").mockResolvedValue(undefined);
  const { host, dispose } = openPage();
  try {
    await ready(host);
    await vi.waitFor(() => expect(host.querySelector("[data-seen-header]")).not.toBeNull());
    mainPaneRouter.replaceActiveRoute({ kind: "journals" });
    await vi.waitFor(() => expect(mark).toHaveBeenCalledTimes(1));
    expect(mark).toHaveBeenCalledWith("seen", "pages/Seen.md");
    expect(read).toHaveBeenCalledTimes(1);
  } finally { dispose(); }

});

it("unavailable graph: no mark, no toast (F5)", async () => {
  const mark = vi.spyOn(backend(), "markPageSeen").mockResolvedValue(undefined);
  resetSeenForTests();
  const unavailable = vi.spyOn(backend(), "getPageChanges").mockResolvedValue(reply({ available: false, reason: "not a repository" }));
  mark.mockClear();
  const second = openPage();
  try {
    await ready(second.host);
    await vi.waitFor(() => expect(unavailable).toHaveBeenCalled());
    mainPaneRouter.replaceActiveRoute({ kind: "journals" });
    await new Promise((r) => setTimeout(r, 20));
    expect(mark).not.toHaveBeenCalled();
    expect(toasts()).toEqual([]);
    expect(second.host.querySelector("[data-seen-header]")).toBeNull();
  } finally { second.dispose(); }
});

it("coming back to an evicted page reads it again and marks it on the next leave", async () => {
  const read = vi.spyOn(backend(), "getPageChanges").mockResolvedValue(reply({ blocks: [{ block_path: [2], persisted_id: null }] }));
  const mark = vi.spyOn(backend(), "markPageSeen").mockResolvedValue(undefined);
  const { host, dispose } = openPage();
  try {
    await ready(host);
    await vi.waitFor(() => expect(host.querySelector("[data-seen-header]")).not.toBeNull());
    mainPaneRouter.replaceActiveRoute({ kind: "journals" });
    await vi.waitFor(() => expect(mark).toHaveBeenCalledTimes(1));
    read.mockResolvedValue(reply());
    resetStore(); // the working set evicted it
    mainPaneRouter.replaceActiveRoute({ kind: "page", name: "Seen", pageKind: "page" });
    await vi.waitFor(() => expect(read).toHaveBeenCalledTimes(2));
    await ready(host);
    await Promise.resolve();
    expect(host.querySelector("[data-seen-header]")).toBeNull();
    mainPaneRouter.replaceActiveRoute({ kind: "journals" });
    await vi.waitFor(() => expect(mark).toHaveBeenCalledTimes(2));
  } finally { dispose(); }
});

it("reading stays off pages that are not the routed single page", async () => {
  const read = vi.spyOn(backend(), "getPageChanges").mockResolvedValue(reply());
  mainPaneRouter.replaceActiveRoute({ kind: "journals" });
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <PageView />, host);
  try {
    await new Promise((r) => setTimeout(r, 50));
    expect(read).not.toHaveBeenCalled();
  } finally { dispose(); }
});
