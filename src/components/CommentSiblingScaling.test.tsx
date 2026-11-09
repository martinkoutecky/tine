// GH #891 review finding 1 (PC-1, PC-3, PC-4, PC-5): typing in one ordinary
// child of a commented block must cost the same whatever the number of its
// siblings. Comment membership, passage order and grouping follow structural and
// raw-text deltas (as `createQuotedSourceRanges` does), not a rescan of every
// child per keystroke. Instruments: foreign store reads (keystrokeProbe) and
// facet lookups from outside the facet module.
import "../keystrokeProbe.test.support";
import { afterEach, beforeAll, beforeEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { SurfaceContext } from "./Block";
import { BlockList } from "./BlockList";
import { createMarginSurface } from "./MarginColumn";
import { MarginContext } from "./marginContext";
import { CommentResolutionScope } from "./CommentResolutionScope";
import { blockRegions, initParser } from "../render/parse";
import { loadSingle } from "../document/workingSet";
import { resetStore } from "../document";
import { setDoc } from "../document/model";
import { pageByName } from "../document/model";
import { persistCommentMarginHidden } from "../commentMarginPref";
import { bindTestHost } from "../document/host/wiring.test.support";
import { answerOpensFromDocument } from "../document/host/documentHost.test.support";
import { measure } from "../keystrokeProbe.test.support";
import type { BlockDto } from "../types";

const facetCalls = vi.hoisted(() => ({ count: 0 }));
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
afterEach(() => { dispose?.(); dispose = undefined; resetStore(); document.body.replaceChildren(); vi.restoreAllMocks(); });

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
    return <SurfaceContext.Provider value="main"><CommentResolutionScope roots={pageByName("Scale")?.roots ?? []} format="md"><MarginContext.Provider value={margin.placement}>
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
  dispose = render(() => <SurfaceContext.Provider value="main"><CommentResolutionScope roots={pageByName("Folded")?.roots ?? []} format="md">
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
