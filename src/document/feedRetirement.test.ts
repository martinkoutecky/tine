import { afterEach, beforeAll, expect, it, vi } from "vitest";
import * as workingSet from "./workingSet";
import { doc, pageByName } from "./model";
import { initParser } from "../render/parse";
import { endEdit, startEditing } from "../editorController";
import { selectBlock, clearSelection } from "./edits/selection";
import { tryFreezeGraphRewrite } from "./graphRewriteState";
import { setRaw } from "./edits/blocks";
import { withBlockMoving } from "./edits/moves";
import { flushPage, isConflicted, isDirty } from "./host/wiring";
import { bindTestHost } from "./host/wiring.test.support";
import { backend } from "../backend";
import type { PageDto } from "../types";

const day = (i: number): PageDto => ({ name: `Day${i}`, title: `Day${i}`, kind: "journal", format: "md",
  pre_block: null, blocks: Array.from({ length: 30 }, (_, b) => ({ id: `day-${i}-${b}`, raw: `Day ${i} content ${b}`,
    collapsed: false, children: [] })) });
// The pre-fix boundary has no retirement operation: the workload still proves
// its user-visible retention harm rather than failing on a missing import.
const retire = (names: string[]) => (workingSet as unknown as {
  retireFeedPages?: (names: readonly string[]) => string[];
}).retireFeedPages?.(names) ?? [];
beforeAll(initParser);
afterEach(() => { clearSelection(); endEdit("graph-switch"); workingSet.registerPaneRouteProvider(() => []); workingSet.resetStore(); vi.restoreAllMocks(); });

it("bounds loaded nodes across hundreds of appended, measured offscreen days", () => {
  for (let i = 0; i < 200; i++) {
    workingSet.appendFeed([day(i)]);
    retire([...doc.feed].slice(0, -3));
  }
  expect(doc.feed).toHaveLength(200);
  expect(doc.pages.length).toBeLessThanOrEqual(12);
  expect(Object.keys(doc.byId).length).toBeLessThanOrEqual(360);
});

it("keeps editing, selected, drafting, routed and frozen days, and reloads retired content", () => {
  workingSet.loadFeed(Array.from({ length: 20 }, (_, i) => day(i)));
  selectBlock(pageByName("Day1")!.roots[0]);
  const unpin = workingSet.pinPageWhileDrafting(() => "Day2");
  workingSet.registerPaneRouteProvider(() => [{ kind: "page", name: "Day3", pageKind: "journal", block: "target" }]);
  const thaw = tryFreezeGraphRewrite()!;
  expect(retire([...doc.feed])).toEqual([]);
  thaw();
  const retired = retire([...doc.feed]);
  expect(retired.length).toBeGreaterThan(0);
  for (const name of ["Day1", "Day2", "Day3"]) expect(pageByName(name)).toBeDefined();
  const i = Number(retired[0].slice(3));
  expect(pageByName(retired[0])).toBeUndefined();
  expect(workingSet.ensurePageLoaded(day(i))).toBeNull();
  expect(doc.byId[pageByName(retired[0])!.roots[4]].raw).toBe(`Day ${i} content 4`);
  unpin(); workingSet.registerPaneRouteProvider(() => []);
});

it("never retires an actively edited page", () => {
  workingSet.loadFeed(Array.from({ length: 20 }, (_, i) => day(i)));
  startEditing(pageByName("Day0")!.roots[0], 0);
  retire([...doc.feed]);
  expect(pageByName("Day0")).toBeDefined();
});

it("keeps dirty, saving and conflicted input and every page during a transfer", async () => {
  const host = await bindTestHost();
  workingSet.loadFeed(Array.from({ length: 20 }, (_, i) => day(i)));
  const sent: Array<{ id: number; key: string }> = [];
  vi.spyOn(backend(), "pageSubmit").mockImplementation(async (_session, id, key) => { sent.push({ id, key }); return null; });
  setRaw(pageByName("Day0")!.roots[0], "unsaved input");
  expect(isDirty("Day0")).toBe(true);
  retire([...doc.feed]);
  expect(pageByName("Day0")).toBeDefined();
  // Restore capacity pressure before testing the in-flight and conflict pins.
  workingSet.appendFeed(Array.from({ length: 10 }, (_, i) => day(i + 20)));
  void flushPage("Day0");
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  retire([...doc.feed]);
  expect(doc.byId[pageByName("Day0")!.roots[0]].raw).toBe("unsaved input");
  host.deliver({ key: sent[0].key, answer: { id: sent[0].id, version: 5, took: true, outcome: { kind: "applied" } },
    notice: { conflictReported: true }, page: { version: 5, conflict: true, risk: false,
      disk: { kind: "file", rev: "external" }, text: { kind: "unchanged" } } });
  expect(isConflicted("Day0")).toBe(true);
  workingSet.appendFeed(Array.from({ length: 10 }, (_, i) => day(i + 30)));
  await withBlockMoving("Day1", () => expect(retire([...doc.feed])).toEqual([]));
  retire([...doc.feed]);
  expect(doc.byId[pageByName("Day0")!.roots[0]].raw).toBe("unsaved input");
});
