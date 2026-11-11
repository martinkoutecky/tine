// GH #891 review finding 1 (PC-1, PC-3, PC-4, PC-5): typing in one ordinary
// child of a commented block must cost the same whatever the number of its
// siblings. Comment membership, passage order and grouping follow structural and
// raw-text deltas (as `createQuotedSourceRanges` does), not a rescan of every
// child per keystroke. Instruments: foreign store reads (keystrokeProbe) and
// facet lookups from outside the facet module.
import "../keystrokeProbe.test.support";
import { afterEach, beforeAll, beforeEach, expect, it, vi } from "vitest";
import { setImmediate } from "node:timers/promises";
import { render } from "solid-js/web";
import { createComputed } from "solid-js";
import { SurfaceContext } from "./Block";
import { BlockList } from "./BlockList";
import { createMarginSurface } from "./MarginColumn";
import { MarginContext } from "./marginContext";
import { CommentResolutionScope } from "./CommentResolutionScope";
import { blockRegions, initParser } from "../render/parse";
import { loadSingle } from "../document/workingSet";
import { insertEmptyChildBlock, resetStore } from "../document";
import { setDoc } from "../document/model";
import { pageByName } from "../document/model";
import { persistCommentMarginHidden } from "../commentMarginPref";
import { bindTestHost } from "../document/host/wiring.test.support";
import { answerOpensFromDocument } from "../document/host/documentHost.test.support";
import { measure } from "../keystrokeProbe.test.support";
import { tapHighlight } from "../tests/commentEntry";
import type { BlockDto } from "../types";

const facetCalls = vi.hoisted(() => ({ count: 0 }));
const commentViews = vi.hoisted(() => ({ runs: 0 }));
vi.mock("../commentIndex", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../commentIndex")>();
  return { ...actual, createCommentChildren: (...args: Parameters<typeof actual.createCommentChildren>) => {
    const children = actual.createCommentChildren(...args);
    createComputed(() => { children(); commentViews.runs++; });
    return children;
  } };
});
vi.mock("solid-js/store", async (importOriginal) => {
  const actual = await importOriginal<typeof import("solid-js/store")>();
  const probe = await import("../keystrokeProbe.test.support");
  return {
    ...actual,
    createStore: ((init: object, opts?: object) => {
      const [state, set] = (actual.createStore as (a: object, b?: object) => [object, unknown])(init, opts);
      return [probe.instrumentDocStore(state, init), set];
    }) as typeof actual.createStore,
  };
});
vi.mock("../render/facets", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../render/facets")>();
  return { ...actual, facetsOf: (...args: Parameters<typeof actual.facetsOf>) => { facetCalls.count++; return actual.facetsOf(...args); } };
});

let dispose: (() => void) | undefined;
beforeAll(async () => { await initParser(); });
beforeEach(async () => { answerOpensFromDocument(await bindTestHost()); persistCommentMarginHidden(false); });
// Give the worker's pending task-update RPCs an event-loop turn between mounts.
afterEach(async () => { dispose?.(); dispose = undefined; resetStore(); document.body.replaceChildren(); vi.restoreAllMocks(); await setImmediate(); });

function b(id: string, raw: string, children: BlockDto[] = []): BlockDto {
  return { id, raw, children, collapsed: false, properties: blockRegions(raw, "md").properties.filter(p => p.primary).map(p => [p.key, p.value]) };
}
function mount(siblings: number, width: number, comments: number) {
  const kids = Array.from({ length: siblings }, (_, i) => b(`k${i}`, `Ordinary child ${i}`));
  for (let i = 0; i < comments; i++) kids.push(b(`c${i}`, `Thread ${i}\ntine.quote:: passage`));
  loadSingle({ name: "Scale", title: "Scale", kind: "page", pre_block: null, format: "md", blocks: [b("p", "A passage to discuss", kids)] });
  const pane = document.createElement("main"); pane.className = "main-content";
  Object.defineProperty(pane, "clientWidth", { configurable: true, get: () => width }); document.body.append(pane);
  dispose = render(() => {
    let outline: HTMLDivElement | undefined;
    const margin = createMarginSurface({ page: () => pageByName("Scale"), eligible: () => true, outline: () => outline });
    return <SurfaceContext.Provider value="main"><CommentResolutionScope roots={pageByName("Scale")?.roots ?? []}><MarginContext.Provider value={margin.placement}>
      <div class="page-section"><div class="page-blocks" ref={outline}><BlockList ids={pageByName("Scale")?.roots ?? []} /></div><margin.Column /></div>
    </MarginContext.Provider></CommentResolutionScope></SurfaceContext.Provider>;
  }, pane);
}
function edit(siblings: number, width: number, comments: number) {
  mount(siblings, width, comments);
  let seq = 0;
  const one = () => {
    facetCalls.count = 0;
    const m = measure("k1", () => setDoc("byId", "k1", "raw", `Ordinary child 1 ~${++seq}`));
    if (process.env.SCALE_SITES && siblings === 64) console.log(JSON.stringify([...m.bySite].filter(([, v]) => v.rawReads > 0).sort((a, z) => z[1].rawReads - a[1].rawReads).slice(0, 12).map(([k, v]) => [k, v.runs, v.rawReads, [...v.readers.keys()].slice(0, 2)])));
    return { foreignRaw: m.foreignRaw, facets: facetCalls.count, ms: m.ms };
  };
  one(); // warm caches
  const runs = [one(), one()];
  dispose?.(); dispose = undefined; resetStore(); document.body.replaceChildren();
  return { foreignRaw: Math.max(...runs.map(r => r.foreignRaw)), facets: Math.max(...runs.map(r => r.facets)), ms: Math.max(...runs.map(r => r.ms)) };
}

it.each([
  { tier: "medium", width: 800, comments: 1 },
  { tier: "wide", width: 1400, comments: 1 },
  { tier: "medium", width: 800, comments: 0 },
  { tier: "phone", width: 390, comments: 2 },
])("typing in an ordinary sibling costs the same at 4, 16 and 64 siblings ($tier, $comments comments)", ({ width, comments, tier }) => {
  const results = [4, 16, 64].map(n => ({ n, ...edit(n, width, comments) }));
  console.log(JSON.stringify({ measurement: "ordinary sibling keystroke", tier, comments, results }));
  for (const r of results) {
    expect(r.foreignRaw, `foreign raw reads at ${r.n} siblings`).toBe(results[0].foreignRaw);
    expect(r.facets, `facet lookups at ${r.n} siblings`).toBe(results[0].facets);
  }
  expect(results[2].facets).toBeLessThanOrEqual(2);
  // Milliseconds are logged, not asserted: jsdom wall time on a shared host is
  // noise. The typing timing budget (p95 under 16 ms with 64 siblings) is
  // asserted natively by scripts/e2e-mc010-tiers.mjs.
});

// Review round 2 (PC-1/3/4/5): the "N below" count of a folded block follows
// structure and comment membership, so typing in the folded block itself never
// rescans its hidden descendants. The hidden thread sits one level down (a
// direct child's thread is the block's own, not "below").
function mountFolded(children: number, hiddenThreads: number) {
  const kids = Array.from({ length: children }, (_, i) => b(`k${i}`, `Hidden child ${i}`,
    i < hiddenThreads ? [b(`t${i}`, `Hidden thread ${i}\ntine.quote:: child ${i}`)] : []));
  loadSingle({ name: "Folded", title: "Folded", kind: "page", pre_block: null, format: "md", blocks: [{ ...b("p", "A folded parent", kids), collapsed: true }] });
  const pane = document.createElement("main"); pane.className = "main-content";
  Object.defineProperty(pane, "clientWidth", { configurable: true, get: () => 800 }); document.body.append(pane);
  dispose = render(() => <SurfaceContext.Provider value="main"><CommentResolutionScope roots={pageByName("Folded")?.roots ?? []}>
    <BlockList ids={pageByName("Folded")?.roots ?? []} />
  </CommentResolutionScope></SurfaceContext.Provider>, pane);
  return pane;
}
function typeInFolded(children: number, hiddenThreads: number) {
  const pane = mountFolded(children, hiddenThreads);
  const chip = () => pane.querySelector(".comment-below-chip")?.textContent ?? null;
  expect(chip()).toBe(hiddenThreads ? `${hiddenThreads} below` : null);
  let seq = 0;
  const one = () => {
    facetCalls.count = 0;
    const m = measure("p", () => setDoc("byId", "p", "raw", `A folded parent ~${++seq}`));
    return { foreignRaw: m.foreignRaw, facets: facetCalls.count, ms: m.ms };
  };
  one(); // warm caches
  const runs = Array.from({ length: 5 }, one);
  expect(chip(), "the count survives typing").toBe(hiddenThreads ? `${hiddenThreads} below` : null);
  dispose?.(); dispose = undefined; resetStore(); document.body.replaceChildren();
  return { foreignRaw: Math.max(...runs.map(r => r.foreignRaw)), facets: Math.max(...runs.map(r => r.facets)), ms: runs.map(r => r.ms).sort((x, y) => x - y)[2] };
}
it.each([0, 1])("typing in a folded block costs the same at 4, 16 and 64 hidden children (%s hidden threads)", hiddenThreads => {
  const results = [4, 16, 64].map(n => ({ n, ...typeInFolded(n, hiddenThreads) }));
  console.log(JSON.stringify({ measurement: "folded parent keystroke", hiddenThreads, results }));
  for (const r of results) {
    expect(r.foreignRaw, `foreign raw reads at ${r.n} hidden children`).toBe(results[0].foreignRaw);
    expect(r.facets, `facet lookups at ${r.n} hidden children`).toBe(results[0].facets);
  }
  expect(results[2].foreignRaw, "no hidden descendant is read").toBe(0);
  // Timing is asserted natively (scripts/e2e-mc010-tiers.mjs, folded-parent
  // typing budget); jsdom milliseconds are only logged.
});

// Review round 3 (PC-1/PC-3/PC-4/PC-5/PC-9): opening one passage's threads
// mounts that passage's cards only, whatever the number of other passages on the
// block; the others mount when the reader moves to them. Each passage `pNq` has
// one thread, so N passages are N groups.
function openOnePassage(passages: number) {
  const words = Array.from({ length: passages }, (_, i) => `p${i}q`);
  loadSingle({ name: "Passages", title: "Passages", kind: "page", pre_block: null, format: "md",
    blocks: [b("p", words.join(" "), words.map((word, i) => b(`t${i}`, `Thread on ${word}\ntine.quote:: ${word}`)))] });
  const pane = document.createElement("main"); pane.className = "main-content";
  Object.defineProperty(pane, "clientWidth", { configurable: true, get: () => 800 }); document.body.append(pane);
  dispose = render(() => <SurfaceContext.Provider value="main"><CommentResolutionScope roots={pageByName("Passages")?.roots ?? []}>
    <BlockList ids={pageByName("Passages")?.roots ?? []} />
  </CommentResolutionScope></SurfaceContext.Provider>, pane);
  const popup = () => document.querySelector<HTMLElement>(".comment-thread-popup");
  const samples: number[] = [];
  let counts = { groups: 0, cards: 0 };
  for (let round = 0; round < 3; round++) {
    const mark = pane.querySelector<HTMLElement>('.comment-quote-anchor[data-comment-id="t1"]')!;
    const at = performance.now();
    tapHighlight(mark);
    samples.push(performance.now() - at);
    expect(popup()?.textContent).toContain("Thread on p1q");
    counts = { groups: popup()!.querySelectorAll(".comment-group").length, cards: popup()!.querySelectorAll("[data-block-id]").length };
    popup()!.querySelector<HTMLButtonElement>(".comment-popup-close")!.click();
    expect(popup()).toBeNull();
  }
  dispose?.(); dispose = undefined; resetStore(); document.body.replaceChildren();
  return { ...counts, ms: Math.min(...samples) };
}
it("opening one passage mounts that passage's cards only at 4, 16 and 64 passages", () => {
  const results = [4, 16, 64].map(n => ({ n, ...openOnePassage(n) }));
  console.log(JSON.stringify({ measurement: "open one passage", results }));
  for (const r of results) {
    expect(r.groups, `mounted groups at ${r.n} passages`).toBe(1);
    expect(r.cards, `mounted cards at ${r.n} passages`).toBe(1);
  }
  // Timing check: opening must not grow with the passages it does not show.
  // Generous on purpose (shared host, jsdom); it fails when every passage's
  // live tree is mounted (review probe: 53/207/401 ms at 4/16/64).
  expect(results[2].ms, "opening at 64 passages vs 4").toBeLessThan(results[0].ms * 3 + 25);
});

it.each([0, 1].flatMap(comments => [16, 64, 256].map(n => ({ comments, n }))))(
  "a structural edit reruns constant downstream comment work ($n groups, $comments comments)", ({ comments, n }) => {
    // Insert into a group directly: no extra ordinary child needs a setup render.
    const blocks = Array.from({ length: n }, (_, i) => b(`g${i}`, `Group ${i}`, [
      ...(i === 0 && comments ? [b("t", "Comment\ntine.quote:: Group")] : [])]));
    loadSingle({ name: "Structure", title: "Structure", kind: "page", pre_block: null, format: "md", blocks });
    const pane = document.createElement("main"); pane.className = "main-content";
    Object.defineProperty(pane, "clientWidth", { configurable: true, get: () => 1400 }); document.body.append(pane);
    dispose = render(() => {
      let outline: HTMLDivElement | undefined;
      const margin = createMarginSurface({ page: () => pageByName("Structure"), eligible: () => true, outline: () => outline });
      return <SurfaceContext.Provider value="main"><CommentResolutionScope roots={pageByName("Structure")?.roots ?? []}><MarginContext.Provider value={margin.placement}>
        <div class="page-section"><div class="page-blocks" ref={outline}><BlockList ids={pageByName("Structure")?.roots ?? []} /></div><margin.Column /></div>
      </MarginContext.Provider></CommentResolutionScope></SurfaceContext.Provider>;
    }, pane);
    let inserted: string | null = null;
    commentViews.runs = 0;
    const m = measure("g0", () => { inserted = insertEmptyChildBlock("g0", 0); });
    expect(inserted).not.toBeNull();
    expect(pane.querySelector(`[data-block-id="${inserted}"]`)).not.toBeNull();
    const sites = [...m.bySite].filter(([site]) => site.includes("src/commentIndex.ts:"));
    const indexRuns = sites.reduce((sum, [, stat]) => sum + stat.runs, 0);
    const memoEvaluations = Math.max(...sites.map(([, stat]) => stat.runs));
    console.log(JSON.stringify({ measurement: "structural edit", comments, n, indexRuns, memoEvaluations, downstreamRuns: commentViews.runs,
      totalRuns: m.runs, ms: m.ms, sites: sites.map(([site, stat]) => [site, stat.runs]) }));
    // A leaf owns no comment reader. The first child initializes its parent's
    // view; an existing parent reuses its view. No other leaf subscribes to
    // the rescan, so this budget is constant even at 256 groups.
    expect(commentViews.runs).toBe(comments ? 0 : 1);
    expect(memoEvaluations).toBe(1);
    expect(indexRuns - memoEvaluations).toBe(comments ? 1 : 4);
});
