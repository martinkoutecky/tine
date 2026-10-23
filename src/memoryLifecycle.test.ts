import { afterEach, expect, it, vi } from "vitest";
import { createComputed, createRoot, createSignal } from "solid-js";
import { backend } from "./backend";
import { doc, setDoc } from "./document/model";
import { ensurePageLoaded, forgetPage, loadSingle, reloadPage, resetStore } from "./document/workingSet";
import { facetsOf, seededFacetCount } from "./render/facets";
import { parseBlock, parseCacheStats } from "./render/parse";
import { resolveBlockBatched, resolutionCacheStats } from "./resolveBatch";
import { changed, changeKeyCount, flushChangeKeys, stageChangeKeys } from "./changeKeys";
import type { PageDto } from "./types";

afterEach(() => { resetStore(); vi.restoreAllMocks(); });
function page(name: string, count = 30, version = 0): PageDto {
  return { name, title: name, kind: "page", format: "md", pre_block: null,
    blocks: Array.from({ length: count }, (_, i) => ({ id: `${name}-${i}`, raw: `${name} ${version} ${i} ${"x".repeat(500)}`, has_id: false, marker: "TODO", collapsed: false, children: [] })) };
}

it("L10-01: 5,000 visits retain only loaded-node seeds and an evicted page reloads", () => {
  for (let i = 0; i < 5000; i++) loadSingle(page(`P${i}`));
  const nodes = Object.keys(doc.byId).length;
  console.log("L10-01 visits", { pages: doc.pages.length, nodes, seeds: seededFacetCount() });
  expect(doc.pages.length).toBeLessThanOrEqual(80);
  expect(seededFacetCount()).toBeLessThanOrEqual(nodes);
  expect(doc.pages.some(p => p.name === "P0")).toBe(false);
  ensurePageLoaded(page("P0"));
  expect(doc.byId["P0-0"].raw).toBe(page("P0").blocks[0].raw);
  expect(facetsOf(doc.byId["P0-0"].raw, "md").marker).toBe("TODO");
});

it("L10-01: 1,000 replacements release old seeds while a large loaded page stays all hits", () => {
  loadSingle(page("Fixed", 60));
  for (let i = 1; i <= 1000; i++) reloadPage(page("Fixed", 60, i));
  console.log("L10-01 reloads", seededFacetCount());
  expect(seededFacetCount()).toBe(60);
  loadSingle(page("Large", 6000));
  for (const b of page("Large", 6000).blocks) expect(facetsOf(b.raw, "md").marker).toBe("TODO");
});

it("L10-02: 1,000 64-KiB drafts have a byte bound and reset drops them eagerly", () => {
  for (let i = 0; i < 1000; i++) parseBlock(`${i} ${"a ".repeat(32768)}`, false);
  console.log("L10-02 drafts", parseCacheStats());
  expect(parseCacheStats().bytes).toBeLessThanOrEqual(8 * 1024 * 1024);
  resetStore();
  expect(parseCacheStats()).toEqual({ entries: 0, bytes: 0 });
});

it("L10-02: a large current AST survives repeated reads without strong admission", () => {
  const raws = Array.from({ length: 48 }, (_, i) => `${i} ${"large ".repeat(20000)}`);
  const current = raws.map(raw => parseBlock(raw, false));
  raws.forEach((raw, i) => expect(parseBlock(raw, false)).toBe(current[i]));
  expect(parseCacheStats()).toEqual({ entries: 0, bytes: 0 });
});

it("L10-01: shared raw survives until its last loaded owner edits or leaves", () => {
  const a = page("A", 1), b = page("B", 1);
  b.blocks[0].raw = a.blocks[0].raw;
  loadSingle(a); ensurePageLoaded(b);
  expect(seededFacetCount()).toBe(1);
  forgetPage("A");
  expect(seededFacetCount()).toBe(1);
  expect(facetsOf(b.blocks[0].raw, "md").marker).toBe("TODO");
  setDoc("byId", "B-0", "raw", "edited");
  expect(seededFacetCount()).toBe(0);
});

it("L10-04/L7-01: 4,000 32-KiB answers bound inactive bytes and reset releases answers and keys", async () => {
  vi.spyOn(backend(), "resolveBlocks").mockImplementation(async ids => ids.map(id => ({ page: "Refs", kind: "page", blocks: [{ id, raw: `${id} ${"r".repeat(32768)}`, collapsed: false, children: [] }] })));
  await Promise.all(Array.from({ length: 4000 }, (_, i) => resolveBlockBatched(`ref-${i}`)));
  console.log("L10-04 answers", resolutionCacheStats(), "keys", changeKeyCount());
  expect(resolutionCacheStats().bytes).toBeLessThanOrEqual(8 * 1024 * 1024);
  expect(resolutionCacheStats().entries).toBeLessThanOrEqual(512);
  expect(changeKeyCount()).toBeLessThanOrEqual(512);
  flushChangeKeys();
  expect(resolutionCacheStats()).toEqual({ entries: 0, bytes: 0 });
  await resolveBlockBatched("new");
  resetStore();
  expect(resolutionCacheStats()).toEqual({ entries: 0, bytes: 0 });
  expect(changeKeyCount()).toBe(0);
});

it("L10-04: live consumers keep answers, refresh by key and dispose their keys", async () => {
  let raw = "old";
  const read = vi.spyOn(backend(), "resolveBlocks").mockImplementation(async ids => ids.map(id => ({ page: "P", kind: "page", blocks: [{ id, raw, collapsed: false, children: [] }] })));
  let answer = "";
  const dispose = createRoot(dispose => {
    createComputed(() => { changed("block:live"); void resolveBlockBatched("live").then(g => { answer = g?.blocks[0].raw ?? ""; }); });
    return dispose;
  });
  await vi.waitFor(() => expect(answer).toBe("old"));
  const duplicate = resolveBlockBatched("live");
  expect(resolveBlockBatched("live")).toBe(duplicate);
  await duplicate;
  raw = "edited";
  stageChangeKeys({ rev: "1", inventoryChanged: false, full: false, pages: [], blocks: ["live"], linksTo: [], props: {} });
  flushChangeKeys(true);
  await vi.waitFor(() => expect(answer).toBe("edited"));
  expect(read).toHaveBeenCalledTimes(2);
  dispose();
  await Promise.resolve();
  expect(changeKeyCount()).toBe(0);
});

it("L10-04: live oversized answers remain shared and release on consumer disposal", async () => {
  vi.spyOn(backend(), "resolveBlocks").mockImplementation(async ids => ids.map(id => ({ page: "P", kind: "page", blocks: [{ id, raw: "r".repeat(300000), collapsed: false, children: [] }] })));
  const dispose = createRoot(dispose => { createComputed(() => { changed("block:large"); }); return dispose; });
  const answer = await resolveBlockBatched("large");
  expect(resolutionCacheStats().bytes).toBeGreaterThan(1024 * 1024);
  expect(await resolveBlockBatched("large")).toBe(answer);
  dispose();
  await Promise.resolve();
  expect(resolutionCacheStats()).toEqual({ entries: 0, bytes: 0 });
});

it("L10-04: queued and late answers cannot repopulate a reset binding", async () => {
  let finish!: (groups: null[]) => void;
  const read = vi.spyOn(backend(), "resolveBlocks").mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
  const old = resolveBlockBatched("old");
  await vi.waitFor(() => expect(read).toHaveBeenCalledWith(["old"]));
  const queued = resolveBlockBatched("queued");
  resetStore();
  await expect(old).resolves.toBeNull();
  await expect(queued).resolves.toBeNull();
  finish([null]);
  await Promise.resolve(); await Promise.resolve();
  expect(resolutionCacheStats()).toEqual({ entries: 0, bytes: 0 });
  expect(read).toHaveBeenCalledTimes(1);
});

it("L10-04: concurrent duplicate requests share one in-flight read; inactive misses have a cardinality bound", async () => {
  const read = vi.spyOn(backend(), "resolveBlocks").mockImplementation(async ids => ids.map(() => null));
  const a = resolveBlockBatched("a");
  expect(resolveBlockBatched("a")).toBe(a);
  const b = resolveBlockBatched("b");
  await Promise.all([a, b]);
  expect(read).toHaveBeenCalledExactlyOnceWith(["a", "b"]);
  await Promise.all(Array.from({ length: 1000 }, (_, i) => resolveBlockBatched(`miss-${i}`)));
  expect(resolutionCacheStats().entries).toBe(512);
});

it("L10-04: changing tracked dependencies releases the old key; shared consumers keep theirs", async () => {
  const [id, setId] = createSignal("a");
  const dispose = createRoot(dispose => { createComputed(() => { changed(`block:${id()}`); }); return dispose; });
  const other = createRoot(dispose => { createComputed(() => { changed("block:a"); }); return dispose; });
  setId("b");
  expect(changeKeyCount()).toBe(2);
  other();
  await Promise.resolve();
  expect(changeKeyCount()).toBe(1);
  dispose();
  await Promise.resolve();
  expect(changeKeyCount()).toBe(0);
  const old = changed("block:a");
  for (let i = 0; i < 4000; i++) changed(`block:${i}`);
  expect(changeKeyCount()).toBeLessThanOrEqual(512);
  expect(changed("block:a")).not.toBe(old);
});
