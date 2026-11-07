import { afterEach, expect, it } from "vitest";
import { createComputed, createRoot } from "solid-js";
import { produce } from "solid-js/store";
import { onBlockRawChanged, pageByName, setDoc, someBlockInTrees, writeRaw, writeWholePages } from "./model";
import { ensurePageLoaded, loadFeed, resetStore, retireFeedPages } from "./workingSet";

afterEach(resetStore);

it.each([8, 32])("GH #886: raw writes dispatch only to registered block owners, K=%i", K => {
  setDoc("byId", Object.fromEntries(Array.from({ length: K }, (_, i) => [`raw${i}`, {
    id: `raw${i}`, raw: "body", page: "P", parent: null, children: [], collapsed: false,
  }])));
  let calls = 0;
  const stops = Array.from({ length: K }, (_, i) => onBlockRawChanged(() => { calls++; }, [`raw${i}`]));
  const overlapping = onBlockRawChanged(() => { calls++; }, ["raw0", "raw1", "raw0"]);
  try {
    setDoc("byId", "raw0", "raw", "edited");
    expect(calls).toBe(2);
    calls = 0;
    setDoc(produce(s => { writeRaw(s, "raw1", "producer"); writeRaw(s, "raw1", "final"); }));
    expect(calls).toBe(4); // Preserve notification order and repeated writes.
    calls = 0;
    stops[0](); overlapping();
    setDoc("byId", "raw0", "raw", "after disposal");
    expect(calls).toBe(0);
  } finally { stops.forEach(stop => stop()); overlapping(); }
});
it.each([8, 32])("GH #886: a page tree follows its own replacement and ignores disjoint admissions, K=%i", K => {
  for (let i = 0; i < K; i++) ensurePageLoaded({ name: `P${i}`, title: `P${i}`, kind: "page", pre_block: null,
    blocks: [{ id: `root${i}`, raw: "body", collapsed: false, children: [] }] });
  let visits = 0;
  const answers: boolean[] = [];
  const dispose = createRoot(d => {
    for (let i = 0; i < K; i++) createComputed(() => {
      answers[i] = someBlockInTrees(pageByName(`P${i}`)!.roots, n => { visits++; return n.raw === "comment"; });
    });
    return d;
  });
  try {
    visits = 0;
    ensurePageLoaded({ name: "unrelated", title: "unrelated", kind: "page", pre_block: null, blocks: [] });
    expect(visits).toBe(0);
    // Whole-page publication can preserve roots (Solid reconciliation or a
    // producer); the page revision must still refresh its untracked tree read.
    writeWholePages(["P0"], () => setDoc(produce(s => { s.byId.root0.raw = "comment"; })));
    expect(answers[0]).toBe(true);
    expect(answers.slice(1)).toEqual(Array(K - 1).fill(false));
    expect(visits).toBe(1);
  } finally { dispose(); }
});

it.each([8, 32])("GH #886: retiring a disjoint feed page does no work in the main K-block tree, K=%i", K => {
  loadFeed(Array.from({ length: 14 }, (_, i) => ({ name: `Day${i}`, title: `Day${i}`, kind: "journal", pre_block: null,
    blocks: Array.from({ length: K }, (_, j) => ({ id: `d${i}-${j}`, raw: "body", collapsed: false, children: [] })) })));
  let visits = 0;
  const dispose = createRoot(d => {
    createComputed(() => someBlockInTrees(pageByName("Day0")!.roots, () => { visits++; return false; }));
    return d;
  });
  try {
    visits = 0;
    expect(retireFeedPages(["Day13"])).toEqual(["Day13"]);
    expect(visits).toBe(0);
  } finally { dispose(); }
});
