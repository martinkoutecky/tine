import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { createEffect, createRoot } from "solid-js";
import { backend, type GraphAnswersChange } from "../backend";
import { initParser } from "../render/parse";
import { setConflictPolicyAlwaysAsk } from "../conflictPolicy";
import { applyGraphChange, applyGraphChangesBulk, installExternalChangeUiHandler } from "./external";
import { ensurePageLoaded, loadFeed, pinPageWhileDrafting, resetStore } from "./workingSet";
import { doc, pageByName } from "./model";
import type { PageRead } from "../types";
import * as pageIdentity from "../pageIdentity";

beforeAll(() => initParser());
afterEach(() => { resetStore(); setConflictPolicyAlwaysAsk(false); vi.restoreAllMocks(); });
const page = (text: string): PageRead => ({ name: "Loaded", title: "Loaded", kind: "journal", id: "journals/exact.md", rev: text,
  pre_block: null, blocks: [{ id: "loaded-block", raw: text, collapsed: false, children: [] }] });
const publication = (rev: string, pages: string[]): GraphAnswersChange => ({ rev, inventoryChanged: false, blockRefCounts: {},
  summary: { rev, pages, blocks: [], linksTo: [], props: {}, inventoryChanged: false, full: false } });
const publish = (rev: string, pages: string[]) => applyGraphChangesBulk({ changes: [], answers: publication(rev, pages) });
const shown = () => pageByName("Loaded")?.roots.map(id => doc.byId[id].raw);

it("an unloaded own publication costs zero page reads, feed restarts and loaded-page renders", async () => {
  ensurePageLoaded(page("A"));
  const read = vi.spyOn(backend(), "getPage");
  const path = vi.spyOn(backend(), "getPageByPath");
  const restart = vi.fn();
  installExternalChangeUiHandler(() => ({ pageOpen: () => false, journalsOpen: true, restartJournalFeed: restart, leaveRemovedPage: () => {} }));
  let renders = 0;
  const dispose = createRoot(dispose => { createEffect(() => { pageByName("Loaded")?.roots; renders++; }); return dispose; });
  try {
    const initial = renders;
    await publish("1", ["unloaded"]);
    expect(read).not.toHaveBeenCalled();
    expect(path).not.toHaveBeenCalled();
    expect(restart).not.toHaveBeenCalled();
    expect(renders).toBe(initial);
  } finally { dispose(); }
});

it("a loaded feed page takes one exact read, without a feed restart or duplicate delivery", async () => {
  expect(loadFeed([page("A")])).toBe("published");
  const read = vi.spyOn(backend(), "getPage");
  const path = vi.spyOn(backend(), "getPageByPath").mockResolvedValue(page("B"));
  const restart = vi.fn();
  installExternalChangeUiHandler(() => ({ pageOpen: () => true, journalsOpen: true, restartJournalFeed: restart, leaveRemovedPage: () => {} }));
  await publish("1", ["loaded", "unloaded"]);
  await publish("1", ["loaded", "unloaded"]);
  expect(shown()).toEqual(["B"]);
  expect(path).toHaveBeenCalledExactlyOnceWith("journals/exact.md");
  expect(read).not.toHaveBeenCalled();
  expect(restart).not.toHaveBeenCalled();
});

it("an own publication deferred by a draft still bypasses the outside-change policy on release", async () => {
  ensurePageLoaded(page("A"));
  setConflictPolicyAlwaysAsk(true);
  const path = vi.spyOn(backend(), "getPageByPath").mockResolvedValue(page("B"));
  const release = pinPageWhileDrafting(() => "Loaded");
  await publish("1", ["loaded"]);
  expect(shown()).toEqual(["A"]);
  expect(path).not.toHaveBeenCalled();
  release();
  await vi.waitFor(() => expect(shown()).toEqual(["B"]));
  expect(path).toHaveBeenCalledOnce();
});

it.each([false, true])("a retired physical target's publication read cannot replace or remove its new owner (missing: %s)", async missing => {
  ensurePageLoaded(page("A"));
  let deliver!: (dto: PageRead | null) => void;
  const read = vi.spyOn(backend(), "getPageByPath")
    .mockImplementationOnce(() => new Promise(resolve => { deliver = resolve; }))
    .mockResolvedValue(page("B"));
  const pending = publish("1", ["loaded"]);
  await vi.waitFor(() => expect(read).toHaveBeenCalledOnce());
  const replacement = { ...page("selected twin"), id: "journals/selected-twin.md" };
  expect(ensurePageLoaded(replacement)).toBeNull();
  deliver(missing ? null : page("B"));
  await pending;
  expect(pageByName("Loaded")?.id).toBe(replacement.id);
  expect(shown()).toEqual(["selected twin"]);
  expect(read).toHaveBeenCalledOnce();
});

it("a publication carries unchanged loaded pages' native identity keys without recomputing them", async () => {
  ensurePageLoaded(page("A"));
  for (let i = 0; i < 20; i++) ensurePageLoaded({ ...page("untouched"), name: `Untouched ${i}`, id: `pages/untouched-${i}.md` });
  const normalize = vi.spyOn(pageIdentity, "pageIdentityKey");
  vi.spyOn(backend(), "getPageByPath").mockResolvedValue(page("B"));
  await publish("1", ["loaded"]);
  expect(shown()).toEqual(["B"]);
  expect(normalize.mock.calls.filter(([name]) => name.startsWith("Untouched "))).toHaveLength(0);
});

it.each(["own", "external", "external-retitled"] as const)("a missing loaded physical owner uses removal handling after an %s publication", async origin => {
  loadFeed([page("stray")]);
  const exact = vi.spyOn(backend(), "getPageByPath").mockResolvedValue(null);
  const named = vi.spyOn(backend(), "getPage").mockResolvedValue(null);
  const leave = vi.fn(), restart = vi.fn();
  installExternalChangeUiHandler(() => ({ pageOpen: () => true, journalsOpen: true, leaveRemovedPage: leave, restartJournalFeed: restart }));
  if (origin === "own") await publish("1", ["loaded"]);
  else await applyGraphChange({ name: origin === "external-retitled" ? "Retitled" : "Loaded", kind: "journal", path: "journals/exact.md", created: false, removed: false });
  expect(pageByName("Loaded")).toBeUndefined();
  expect(leave).toHaveBeenCalledExactlyOnceWith("Loaded");
  expect(restart).toHaveBeenCalledOnce();
  expect(exact).toHaveBeenCalledTimes(origin !== "external" ? 1 : 0);
  expect(named).toHaveBeenCalledTimes(origin === "external" ? 1 : 0);
});

it("a draft opened during a missing-owner read preserves input until release", async () => {
  ensurePageLoaded(page("stray"));
  let deliver!: (dto: PageRead | null) => void;
  const read = vi.spyOn(backend(), "getPageByPath")
    .mockImplementationOnce(() => new Promise(resolve => { deliver = resolve; }))
    .mockResolvedValue(null);
  const leave = vi.fn();
  installExternalChangeUiHandler(() => ({ pageOpen: () => true, journalsOpen: false, leaveRemovedPage: leave, restartJournalFeed: () => {} }));
  const pending = publish("1", ["loaded"]);
  await vi.waitFor(() => expect(read).toHaveBeenCalledOnce());
  const release = pinPageWhileDrafting(() => "Loaded");
  try {
    deliver(null);
    await pending;
    expect(shown()).toEqual(["stray"]);
    expect(leave).not.toHaveBeenCalled();
  } finally { release(); }
  await vi.waitFor(() => expect(pageByName("Loaded")).toBeUndefined());
  expect(leave).toHaveBeenCalledExactlyOnceWith("Loaded");
  expect(read).toHaveBeenCalledTimes(2);
});

it("a different physical file returned for an exact publication read cannot retire or replace its owner", async () => {
  ensurePageLoaded(page("stray"));
  vi.spyOn(backend(), "getPageByPath").mockResolvedValue({ ...page("keeper"), id: "journals/keeper.md" });
  const leave = vi.fn(), restart = vi.fn();
  installExternalChangeUiHandler(() => ({ pageOpen: () => true, journalsOpen: true, leaveRemovedPage: leave, restartJournalFeed: restart }));
  await publish("1", ["loaded"]);
  expect(shown()).toEqual(["stray"]);
  expect(pageByName("Loaded")?.id).toBe("journals/exact.md");
  expect(leave).not.toHaveBeenCalled();
  expect(restart).not.toHaveBeenCalled();
});
